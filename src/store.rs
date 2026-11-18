//! The profile's database (`data.sqlite` in the profile directory): state the program writes and
//! reads back, as opposed to configuration a person edits. Today: each workspace's tabs, so a
//! workspace reopens as it was; later bookmarks and history.
//!
//! One connection per process, used from the GTK thread only.

use rusqlite::{params, Connection};
use std::path::Path;

/// Bump when the tables change and add a migration to `migrate`.
const SCHEMA_VERSION: i64 = 12;

/// A tab as remembered between runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRecord {
    /// Stable for the life of the tab; names its screenshot file.
    pub uid: u64,
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub selected: bool,
    /// How the tab has been used (`sweep::Usage::to_text`).
    pub usage: String,
    /// The tab's group (0: none).
    pub group: u32,
    /// Its back and forward list (`navhistory::encode`), empty for none.
    pub history: String,
}

/// A tab group as remembered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRecord {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub collapsed: bool,
}

/// A bookmark: one per address in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

/// A finished (or stopped) download, as remembered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRecord {
    pub id: i64,
    pub workspace: Option<String>,
    pub name: String,
    pub path: String,
    pub url: String,
    /// Unix time, seconds.
    pub at: i64,
    /// `done`, `cancelled` or `failed`.
    pub state: String,
    pub bytes: i64,
}

/// A page visit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub id: i64,
    /// The workspace's id; `None` for an ephemeral window.
    pub workspace: Option<String>,
    pub url: String,
    pub title: String,
    /// Unix time, seconds.
    pub at: i64,
}

/// A tab that was closed, kept so it can be reopened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTab {
    pub id: i64,
    pub workspace: Option<String>,
    pub url: String,
    pub title: String,
    pub at: i64,
    /// Why it closed: `user`, `window` (an ephemeral window closed with it open), later `sweep:<rule>`.
    pub reason: String,
    /// Tabs closed together with an ephemeral window share a group.
    pub group: Option<i64>,
}

/// Which workspaces' history to look at.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    All,
    Workspace(&'a str),
    /// Windows with no workspace.
    Ephemeral,
}

impl Scope<'_> {
    /// The SQL condition and its parameter (the placeholder is `?N`).
    fn clause(&self, n: usize) -> (String, Option<String>) {
        match self {
            Scope::All => ("1".into(), None),
            Scope::Workspace(w) => (format!("workspace = ?{n}"), Some((*w).to_owned())),
            Scope::Ephemeral => ("workspace IS NULL".into(), None),
        }
    }
}

/// A web app window's remembered size (when not maximized) and whether it was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        let conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::init(conn).map_err(|e| format!("{}: {e}", path.display()))
    }

    #[cfg(test)]
    fn open_memory() -> Store {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> rusqlite::Result<Store> {
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tabs (
                    workspace TEXT NOT NULL,
                    position  INTEGER NOT NULL,
                    uid       INTEGER NOT NULL,
                    url       TEXT NOT NULL,
                    title     TEXT NOT NULL DEFAULT '',
                    pinned    INTEGER NOT NULL DEFAULT 0,
                    selected  INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (workspace, position)
                );",
            )?;
        }
        if version < 2 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS bookmarks (
                    workspace TEXT NOT NULL,
                    position  INTEGER NOT NULL,
                    url       TEXT NOT NULL,
                    title     TEXT NOT NULL DEFAULT '',
                    PRIMARY KEY (workspace, position)
                );",
            )?;
        }
        if version < 3 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS visits (
                    id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    workspace TEXT,
                    url       TEXT NOT NULL,
                    title     TEXT NOT NULL DEFAULT '',
                    at        INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS visits_at ON visits (at);
                CREATE INDEX IF NOT EXISTS visits_workspace ON visits (workspace, at);
                CREATE TABLE IF NOT EXISTS closed_tabs (
                    id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    workspace TEXT,
                    url       TEXT NOT NULL,
                    title     TEXT NOT NULL DEFAULT '',
                    at        INTEGER NOT NULL,
                    reason    TEXT NOT NULL,
                    grp       INTEGER
                );
                CREATE INDEX IF NOT EXISTS closed_at ON closed_tabs (at);",
            )?;
        }
        if version < 4 {
            self.conn.execute_batch("ALTER TABLE tabs ADD COLUMN usage TEXT NOT NULL DEFAULT '';")?;
        }
        if version < 5 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS permission_decisions (
                    origin TEXT NOT NULL,
                    kind   TEXT NOT NULL,
                    choice TEXT NOT NULL,
                    at     INTEGER NOT NULL,
                    PRIMARY KEY (origin, kind)
                );",
            )?;
        }
        if version < 6 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS downloads (
                    id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    workspace TEXT,
                    name      TEXT NOT NULL,
                    path      TEXT NOT NULL,
                    url       TEXT NOT NULL DEFAULT '',
                    at        INTEGER NOT NULL,
                    state     TEXT NOT NULL,
                    bytes     INTEGER NOT NULL DEFAULT 0
                );",
            )?;
        }
        if version < 7 {
            self.conn.execute_batch("CREATE TABLE IF NOT EXISTS site_zoom (host TEXT PRIMARY KEY, factor REAL NOT NULL);")?;
        }
        if version < 8 {
            self.conn.execute_batch(
                "ALTER TABLE tabs ADD COLUMN grp INTEGER NOT NULL DEFAULT 0;
                 CREATE TABLE IF NOT EXISTS tab_groups (
                    workspace TEXT NOT NULL,
                    id        INTEGER NOT NULL,
                    name      TEXT NOT NULL,
                    color     TEXT NOT NULL,
                    collapsed INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (workspace, id)
                );",
            )?;
        }
        if version < 9 {
            self.conn.execute_batch("CREATE TABLE IF NOT EXISTS favicons (origin TEXT PRIMARY KEY, png BLOB NOT NULL, at INTEGER NOT NULL);")?;
        }
        if version < 10 {
            self.conn.execute_batch("CREATE TABLE IF NOT EXISTS app_windows (app TEXT PRIMARY KEY, width INTEGER NOT NULL, height INTEGER NOT NULL, maximized INTEGER NOT NULL DEFAULT 0);")?;
        }
        if version < 11 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS workspace_use (workspace TEXT PRIMARY KEY, at INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )?;
        }
        if version < 12 {
            self.conn.execute_batch("ALTER TABLE tabs ADD COLUMN nav_history TEXT NOT NULL DEFAULT '';")?;
        }
        self.conn.pragma_update(None, "user_version", SCHEMA_VERSION)
    }

    // ---- workspace use and small preferences --------------------------------------------------------

    /// Note that a workspace was used (a window of it opened or was focused), at `at` seconds.
    pub fn touch_workspace(&self, workspace: &str, at: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO workspace_use (workspace, at) VALUES (?1, ?2) ON CONFLICT(workspace) DO UPDATE SET at = ?2", params![workspace, at])?;
        Ok(())
    }

    /// When each workspace was last used.
    pub fn workspace_last_used(&self) -> std::collections::HashMap<String, i64> {
        let Ok(mut stmt) = self.conn.prepare("SELECT workspace, at FROM workspace_use") else { return Default::default() };
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)));
        rows.map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// A remembered setting of the profile (for example the workspace picker's order).
    pub fn pref(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM prefs WHERE key = ?1", params![key], |r| r.get(0)).ok()
    }

    pub fn set_pref(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO prefs (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", params![key, value])?;
        Ok(())
    }

    // ---- web app windows ----------------------------------------------------------------------------

    /// The size and state the app's window had when it was last used.
    pub fn app_window(&self, app: &str) -> Option<WindowState> {
        self.conn
            .query_row("SELECT width, height, maximized FROM app_windows WHERE app = ?1", params![app], |r| Ok(WindowState { width: r.get(0)?, height: r.get(1)?, maximized: r.get::<_, i64>(2)? != 0 }))
            .ok()
    }

    pub fn save_app_window(&self, app: &str, state: WindowState) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO app_windows (app, width, height, maximized) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(app) DO UPDATE SET width = ?2, height = ?3, maximized = ?4",
            params![app, state.width, state.height, state.maximized as i64],
        )?;
        Ok(())
    }

    // ---- permissions -------------------------------------------------------------------------------

    /// What was answered before for this site and kind (`allow` or `deny`), if anything.
    pub fn remembered_permission(&self, origin: &str, kind: &str) -> Option<String> {
        self.conn.query_row("SELECT choice FROM permission_decisions WHERE origin = ?1 AND kind = ?2", params![origin, kind], |r| r.get(0)).ok()
    }

    pub fn remember_permission(&self, origin: &str, kind: &str, choice: &str, at: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO permission_decisions (origin, kind, choice, at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(origin, kind) DO UPDATE SET choice = ?3, at = ?4",
            params![origin, kind, choice, at],
        )?;
        Ok(())
    }

    /// Every remembered answer: (origin, kind, choice, time), by site.
    pub fn remembered_permissions(&self) -> Vec<(String, String, String, i64)> {
        let Ok(mut stmt) = self.conn.prepare("SELECT origin, kind, choice, at FROM permission_decisions ORDER BY origin, kind") else { return Vec::new() };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// Forget a site's answers, or all of them with `None`. Returns how many were forgotten.
    pub fn forget_permissions(&self, origin: Option<&str>) -> rusqlite::Result<usize> {
        match origin {
            Some(o) => self.conn.execute("DELETE FROM permission_decisions WHERE origin = ?1", params![o]),
            None => self.conn.execute("DELETE FROM permission_decisions", []),
        }
    }

    /// A workspace's bookmarks, in the order they were added.
    pub fn bookmarks(&self, workspace: &str) -> Vec<Bookmark> {
        let Ok(mut stmt) = self.conn.prepare("SELECT url, title FROM bookmarks WHERE workspace = ?1 ORDER BY position") else {
            return Vec::new();
        };
        match stmt.query_map(params![workspace], |r| Ok(Bookmark { url: r.get(0)?, title: r.get(1)? })) {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn has_bookmark(&self, workspace: &str, url: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM bookmarks WHERE workspace = ?1 AND url = ?2", params![workspace, url], |_| Ok(()))
            .is_ok()
    }

    /// Add a bookmark at the end (a no-op if the address is already bookmarked).
    pub fn add_bookmark(&self, workspace: &str, url: &str, title: &str) -> rusqlite::Result<()> {
        if self.has_bookmark(workspace, url) {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO bookmarks (workspace, position, url, title) VALUES (?1, (SELECT COALESCE(MAX(position) + 1, 0) FROM bookmarks WHERE workspace = ?1), ?2, ?3)",
            params![workspace, url, title],
        )?;
        Ok(())
    }

    pub fn remove_bookmark(&self, workspace: &str, url: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM bookmarks WHERE workspace = ?1 AND url = ?2", params![workspace, url])?;
        Ok(())
    }

    /// Replace a workspace's remembered tabs.
    pub fn save_tabs(&self, workspace: &str, tabs: &[TabRecord]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM tabs WHERE workspace = ?1", params![workspace])?;
        {
            let mut insert = tx.prepare("INSERT INTO tabs (workspace, position, uid, url, title, pinned, selected, usage, grp, nav_history) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")?;
            for (i, t) in tabs.iter().enumerate() {
                insert.execute(params![workspace, i as i64, t.uid as i64, t.url, t.title, t.pinned, t.selected, t.usage, t.group as i64, t.history])?;
            }
        }
        tx.commit()
    }

    /// A workspace's remembered tabs, in order.
    pub fn load_tabs(&self, workspace: &str) -> Vec<TabRecord> {
        let Ok(mut stmt) = self.conn.prepare("SELECT uid, url, title, pinned, selected, usage, grp, nav_history FROM tabs WHERE workspace = ?1 ORDER BY position") else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![workspace], |r| {
            Ok(TabRecord { uid: r.get::<_, i64>(0)? as u64, url: r.get(1)?, title: r.get(2)?, pinned: r.get(3)?, selected: r.get(4)?, usage: r.get(5)?, group: r.get::<_, i64>(6)? as u32, history: r.get(7)? })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    // ---- tab groups --------------------------------------------------------------------------------

    pub fn save_groups(&self, workspace: &str, groups: &[GroupRecord]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM tab_groups WHERE workspace = ?1", params![workspace])?;
        for g in groups {
            tx.execute("INSERT INTO tab_groups (workspace, id, name, color, collapsed) VALUES (?1, ?2, ?3, ?4, ?5)", params![workspace, g.id as i64, g.name, g.color, g.collapsed])?;
        }
        tx.commit()
    }

    pub fn load_groups(&self, workspace: &str) -> Vec<GroupRecord> {
        let Ok(mut stmt) = self.conn.prepare("SELECT id, name, color, collapsed FROM tab_groups WHERE workspace = ?1 ORDER BY id") else { return Vec::new() };
        stmt.query_map(params![workspace], |r| Ok(GroupRecord { id: r.get::<_, i64>(0)? as u32, name: r.get(1)?, color: r.get(2)?, collapsed: r.get(3)? }))
            .map(|r| r.flatten().collect())
            .unwrap_or_default()
    }

    // ---- favicons ----------------------------------------------------------------------------------

    /// A site's icon (PNG), by origin (`https://example.com`), as last seen.
    pub fn favicon(&self, origin: &str) -> Option<Vec<u8>> {
        self.conn.query_row("SELECT png FROM favicons WHERE origin = ?1", params![origin], |r| r.get(0)).ok()
    }

    /// Remember a site's icon. Keeps the newest 2000 sites.
    pub fn set_favicon(&self, origin: &str, png: &[u8], at: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO favicons (origin, png, at) VALUES (?1, ?2, ?3) ON CONFLICT(origin) DO UPDATE SET png = ?2, at = ?3", params![origin, png, at])?;
        self.conn.execute("DELETE FROM favicons WHERE origin NOT IN (SELECT origin FROM favicons ORDER BY at DESC LIMIT 2000)", [])?;
        Ok(())
    }

    /// Forget one site's icon (the self-test starts from nothing).
    pub fn forget_favicon(&self, origin: &str) {
        let _ = self.conn.execute("DELETE FROM favicons WHERE origin = ?1", params![origin]);
    }

    // ---- zoom --------------------------------------------------------------------------------------

    /// The zoom factor remembered for a site (host), if it is not 100 %.
    pub fn site_zoom(&self, host: &str) -> Option<f64> {
        self.conn.query_row("SELECT factor FROM site_zoom WHERE host = ?1", params![host], |r| r.get(0)).ok()
    }

    /// Remember a site's zoom factor (100 % forgets it).
    pub fn set_site_zoom(&self, host: &str, factor: f64) -> rusqlite::Result<()> {
        if (factor - 1.0).abs() < 0.001 {
            self.conn.execute("DELETE FROM site_zoom WHERE host = ?1", params![host])?;
        } else {
            self.conn.execute("INSERT INTO site_zoom (host, factor) VALUES (?1, ?2) ON CONFLICT(host) DO UPDATE SET factor = ?2", params![host, factor])?;
        }
        Ok(())
    }

    // ---- downloads ---------------------------------------------------------------------------------

    pub fn record_download(&self, workspace: Option<&str>, name: &str, path: &str, url: &str, at: i64, state: &str, bytes: i64) -> rusqlite::Result<i64> {
        self.conn.execute("INSERT INTO downloads (workspace, name, path, url, at, state, bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![workspace, name, path, url, at, state, bytes])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The remembered downloads, newest first.
    pub fn downloads(&self, limit: usize) -> Vec<DownloadRecord> {
        let Ok(mut stmt) = self.conn.prepare(&format!("SELECT id, workspace, name, path, url, at, state, bytes FROM downloads ORDER BY at DESC, id DESC LIMIT {limit}")) else { return Vec::new() };
        stmt.query_map([], |r| Ok(DownloadRecord { id: r.get(0)?, workspace: r.get(1)?, name: r.get(2)?, path: r.get(3)?, url: r.get(4)?, at: r.get(5)?, state: r.get(6)?, bytes: r.get(7)? }))
            .map(|r| r.flatten().collect())
            .unwrap_or_default()
    }

    /// Forget one remembered download (the file stays), or all of them with `None`.
    pub fn remove_downloads(&self, id: Option<i64>) -> rusqlite::Result<usize> {
        match id {
            Some(id) => self.conn.execute("DELETE FROM downloads WHERE id = ?1", params![id]),
            None => self.conn.execute("DELETE FROM downloads", []),
        }
    }

    // ---- history -----------------------------------------------------------------------------

    pub fn record_visit(&self, workspace: Option<&str>, url: &str, title: &str, at: i64) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO visits (workspace, url, title, at) VALUES (?1, ?2, ?3, ?4)", params![workspace, url, title, at])?;
        Ok(())
    }

    /// Visits, newest first, whose address or title contains `query` (case-insensitive).
    pub fn search_visits(&self, query: &str, scope: Scope, limit: usize) -> Vec<Visit> {
        let (clause, param) = scope.clause(2);
        let sql = format!("SELECT id, workspace, url, title, at FROM visits WHERE (url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\') AND {clause} ORDER BY at DESC, id DESC LIMIT {limit}");
        let like = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let Ok(mut stmt) = self.conn.prepare(&sql) else { return Vec::new() };
        let map = |r: &rusqlite::Row| Ok(Visit { id: r.get(0)?, workspace: r.get(1)?, url: r.get(2)?, title: r.get(3)?, at: r.get(4)? });
        let rows = match &param {
            Some(p) => stmt.query_map(params![like, p], map),
            None => stmt.query_map(params![like], map),
        };
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// Delete visits (and closed tabs) older than `before` (unix seconds), or all of them with `None`.
    pub fn clear_history(&self, before: Option<i64>) -> rusqlite::Result<usize> {
        let cutoff = before.unwrap_or(i64::MAX);
        let n = self.conn.execute("DELETE FROM visits WHERE at < ?1", params![cutoff])?;
        self.conn.execute("DELETE FROM closed_tabs WHERE at < ?1", params![cutoff])?;
        Ok(n)
    }

    pub fn record_closed(&self, workspace: Option<&str>, url: &str, title: &str, at: i64, reason: &str, group: Option<i64>) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO closed_tabs (workspace, url, title, at, reason, grp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![workspace, url, title, at, reason, group])?;
        Ok(())
    }

    /// Closed tabs, newest first.
    pub fn closed_tabs(&self, scope: Scope, limit: usize) -> Vec<ClosedTab> {
        let (clause, param) = scope.clause(1);
        let sql = format!("SELECT id, workspace, url, title, at, reason, grp FROM closed_tabs WHERE {clause} ORDER BY at DESC, id DESC LIMIT {limit}");
        let Ok(mut stmt) = self.conn.prepare(&sql) else { return Vec::new() };
        let map = |r: &rusqlite::Row| Ok(ClosedTab { id: r.get(0)?, workspace: r.get(1)?, url: r.get(2)?, title: r.get(3)?, at: r.get(4)?, reason: r.get(5)?, group: r.get(6)? });
        let rows = match &param {
            Some(p) => stmt.query_map(params![p], map),
            None => stmt.query_map([], map),
        };
        rows.map(|r| r.flatten().collect()).unwrap_or_default()
    }

    /// Remove a closed tab from the list (it was reopened). Returns it if it was there.
    pub fn take_closed(&self, id: i64) -> Option<ClosedTab> {
        let tab = self.closed_tabs(Scope::All, 10_000).into_iter().find(|t| t.id == id)?;
        self.conn.execute("DELETE FROM closed_tabs WHERE id = ?1", params![id]).ok()?;
        Some(tab)
    }

    /// Remove and return the tabs of the most recently closed ephemeral window, in the order they were
    /// open. Tabs closed on their own (no group) count as a window of one.
    pub fn take_last_closed_window(&self) -> Vec<ClosedTab> {
        let mut recent = self.closed_tabs(Scope::Ephemeral, 200);
        let Some(first) = recent.first().cloned() else { return Vec::new() };
        let tabs: Vec<ClosedTab> = match first.group {
            Some(g) => recent.drain(..).filter(|t| t.group == Some(g)).collect(),
            None => vec![first],
        };
        for t in &tabs {
            let _ = self.conn.execute("DELETE FROM closed_tabs WHERE id = ?1", params![t.id]);
        }
        let mut tabs = tabs;
        tabs.sort_by_key(|t| t.id);
        tabs
    }

    /// Forget everything stored for a workspace.
    pub fn delete_workspace(&self, workspace: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM tabs WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM tab_groups WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM bookmarks WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM visits WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM closed_tabs WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM app_windows WHERE app = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM workspace_use WHERE workspace = ?1", params![workspace])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(uid: u64, url: &str, selected: bool) -> TabRecord {
        TabRecord { uid, url: url.into(), title: format!("t{uid}"), pinned: uid == 1, selected, usage: format!("{uid}:1"), group: 0, history: String::new() }
    }

    #[test]
    fn a_tabs_history_is_saved_and_loaded_with_it() {
        let s = Store::open_memory();
        let mut with = rec(1, "https://e.example/3", true);
        with.history = crate::navhistory::encode(&[crate::navhistory::Entry::new("https://e.example/2", "two")], &crate::navhistory::Entry::new("https://e.example/3", "three"), &[]);
        assert!(!with.history.is_empty());
        s.save_tabs("w", &[with.clone(), rec(2, "https://e.example/9", false)]).unwrap();
        let loaded = s.load_tabs("w");
        assert_eq!(loaded[0].history, with.history);
        assert_eq!(loaded[1].history, "");
    }

    #[test]
    fn workspace_use_and_prefs_are_remembered() {
        let s = Store::open_memory();
        assert!(s.workspace_last_used().is_empty());
        s.touch_workspace("mail", 100).unwrap();
        s.touch_workspace("docs", 200).unwrap();
        s.touch_workspace("mail", 300).unwrap();
        let used = s.workspace_last_used();
        assert_eq!((used.get("mail"), used.get("docs")), (Some(&300), Some(&200)));
        s.delete_workspace("docs").unwrap();
        assert!(!s.workspace_last_used().contains_key("docs"), "removing a workspace forgets its use");
        assert_eq!(s.pref("workspace_order"), None);
        s.set_pref("workspace_order", "alphabetical").unwrap();
        s.set_pref("workspace_order", "last_used").unwrap();
        assert_eq!(s.pref("workspace_order").as_deref(), Some("last_used"));
    }

    #[test]
    fn tab_groups_round_trip_with_their_tabs() {
        let s = Store::open_memory();
        let mut a = rec(1, "https://a", true);
        a.group = 2;
        s.save_tabs("w", &[a, rec(2, "https://b", false)]).unwrap();
        s.save_groups("w", &[GroupRecord { id: 2, name: "News".into(), color: "#3584e4".into(), collapsed: true }]).unwrap();
        assert_eq!(s.load_tabs("w").iter().map(|t| t.group).collect::<Vec<_>>(), [2, 0]);
        let g = s.load_groups("w");
        assert_eq!((g.len(), g[0].name.as_str(), g[0].collapsed), (1, "News", true));
        assert!(s.load_groups("other").is_empty());
        s.delete_workspace("w").unwrap();
        assert!(s.load_groups("w").is_empty());
    }

    #[test]
    fn favicons_are_remembered_by_origin_and_capped() {
        let s = Store::open_memory();
        assert_eq!(s.favicon("https://a.example"), None);
        s.set_favicon("https://a.example", b"one", 1).unwrap();
        s.set_favicon("https://a.example", b"two", 2).unwrap();
        assert_eq!(s.favicon("https://a.example").as_deref(), Some(&b"two"[..]), "the newest wins");
        assert_eq!(s.favicon("https://b.example"), None);
        for i in 0..2100 {
            s.set_favicon(&format!("https://site{i}.example"), b"x", 10 + i).unwrap();
        }
        assert_eq!(s.favicon("https://a.example"), None, "the oldest are dropped past 2000");
        assert!(s.favicon("https://site2099.example").is_some());
    }

    #[test]
    fn site_zoom_is_remembered_and_forgotten_at_100_percent() {
        let s = Store::open_memory();
        assert_eq!(s.site_zoom("a.example"), None);
        s.set_site_zoom("a.example", 1.5).unwrap();
        s.set_site_zoom("a.example", 1.25).unwrap();
        assert_eq!(s.site_zoom("a.example"), Some(1.25));
        assert_eq!(s.site_zoom("b.example"), None);
        s.set_site_zoom("a.example", 1.0).unwrap();
        assert_eq!(s.site_zoom("a.example"), None);
    }

    #[test]
    fn downloads_are_remembered_newest_first_and_can_be_removed() {
        let s = Store::open_memory();
        let a = s.record_download(Some("w"), "a.txt", "/d/a.txt", "https://x/a", 10, "done", 5).unwrap();
        let b = s.record_download(None, "b.zip", "/d/b.zip", "https://x/b", 20, "cancelled", 0).unwrap();
        let all = s.downloads(10);
        assert_eq!(all.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["b.zip", "a.txt"]);
        assert_eq!((all[1].workspace.as_deref(), all[1].bytes, all[0].state.as_str()), (Some("w"), 5, "cancelled"));
        assert_eq!(s.downloads(1).len(), 1);
        s.remove_downloads(Some(a)).unwrap();
        assert_eq!(s.downloads(10).iter().map(|d| d.id).collect::<Vec<_>>(), [b]);
        s.remove_downloads(None).unwrap();
        assert!(s.downloads(10).is_empty());
    }

    #[test]
    fn tabs_round_trip_in_order_per_workspace() {
        let s = Store::open_memory();
        assert!(s.load_tabs("mail").is_empty());
        let a = vec![rec(1, "https://a", false), rec(2, "https://b", true), rec(3, "https://c", false)];
        s.save_tabs("mail", &a).unwrap();
        s.save_tabs("docs", &[rec(9, "https://d", true)]).unwrap();
        assert_eq!(s.load_tabs("mail"), a);
        assert_eq!(s.load_tabs("docs").len(), 1);
        // Saving replaces, it does not append, and an empty list clears.
        s.save_tabs("mail", &[rec(4, "https://e", true)]).unwrap();
        assert_eq!(s.load_tabs("mail"), vec![rec(4, "https://e", true)]);
        s.save_tabs("mail", &[]).unwrap();
        assert!(s.load_tabs("mail").is_empty());
        assert_eq!(s.load_tabs("docs").len(), 1);
    }

    #[test]
    fn bookmarks_are_per_workspace_ordered_and_unique() {
        let s = Store::open_memory();
        s.add_bookmark("a", "https://one", "One").unwrap();
        s.add_bookmark("a", "https://two", "Two").unwrap();
        s.add_bookmark("a", "https://one", "One again").unwrap(); // already there
        s.add_bookmark("b", "https://one", "Other workspace").unwrap();
        let a = s.bookmarks("a");
        assert_eq!(a.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
        assert!(s.has_bookmark("a", "https://two") && !s.has_bookmark("b", "https://two"));
        s.remove_bookmark("a", "https://one").unwrap();
        s.add_bookmark("a", "https://three", "Three").unwrap();
        assert_eq!(s.bookmarks("a").iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["Two", "Three"]);
        assert_eq!(s.bookmarks("b").len(), 1);
        s.delete_workspace("a").unwrap();
        assert!(s.bookmarks("a").is_empty() && s.bookmarks("b").len() == 1);
    }

    #[test]
    fn visits_are_searchable_by_workspace_and_text() {
        let s = Store::open_memory();
        s.record_visit(Some("mail"), "https://mail.example.com/inbox", "Inbox", 100).unwrap();
        s.record_visit(Some("docs"), "https://docs.example.com/a_b", "Spec 100%", 200).unwrap();
        s.record_visit(None, "https://news.net/", "News", 300).unwrap();
        let all = s.search_visits("", Scope::All, 10);
        assert_eq!(all.iter().map(|v| v.at).collect::<Vec<_>>(), [300, 200, 100], "newest first");
        assert_eq!(s.search_visits("EXAMPLE", Scope::All, 10).len(), 2, "case-insensitive, address");
        assert_eq!(s.search_visits("inbox", Scope::All, 10).len(), 1, "title");
        assert_eq!(s.search_visits("", Scope::Workspace("docs"), 10).len(), 1);
        assert_eq!(s.search_visits("", Scope::Ephemeral, 10)[0].url, "https://news.net/");
        // Wildcard characters in the query are literal.
        assert_eq!(s.search_visits("100%", Scope::All, 10).len(), 1);
        assert_eq!(s.search_visits("%", Scope::All, 10).len(), 1);
        assert_eq!(s.search_visits("a_b", Scope::All, 10).len(), 1);
        assert_eq!(s.search_visits("a\\b", Scope::All, 10).len(), 0);
        assert_eq!(s.search_visits("", Scope::All, 2).len(), 2, "limit");
    }

    #[test]
    fn clearing_and_retention() {
        let s = Store::open_memory();
        for (i, at) in [10, 20, 30].iter().enumerate() {
            s.record_visit(Some("w"), &format!("https://x/{i}"), "", *at).unwrap();
            s.record_closed(Some("w"), &format!("https://c/{i}"), "", *at, "user", None).unwrap();
        }
        assert_eq!(s.clear_history(Some(25)).unwrap(), 2);
        assert_eq!(s.search_visits("", Scope::All, 10).len(), 1);
        assert_eq!(s.closed_tabs(Scope::All, 10).len(), 1, "closed tabs follow the same retention");
        s.clear_history(None).unwrap();
        assert!(s.search_visits("", Scope::All, 10).is_empty() && s.closed_tabs(Scope::All, 10).is_empty());
    }

    #[test]
    fn closed_tabs_can_be_reopened_once_and_ephemeral_windows_come_back_whole() {
        let s = Store::open_memory();
        s.record_closed(Some("w"), "https://one", "One", 10, "user", None).unwrap();
        s.record_closed(Some("w"), "https://two", "Two", 20, "sweep:old", None).unwrap();
        s.record_closed(Some("other"), "https://three", "Three", 30, "user", None).unwrap();
        let mine = s.closed_tabs(Scope::Workspace("w"), 10);
        assert_eq!(mine.iter().map(|t| t.reason.as_str()).collect::<Vec<_>>(), ["sweep:old", "user"], "the reason is kept");
        let taken = s.take_closed(mine[0].id).unwrap();
        assert_eq!(taken.url, "https://two");
        assert!(s.take_closed(mine[0].id).is_none(), "a closed tab is reopened once");
        assert_eq!(s.closed_tabs(Scope::Workspace("w"), 10).len(), 1);
        // An ephemeral window's tabs share a group and return together, in order.
        s.record_closed(None, "https://e1", "E1", 40, "window", Some(7)).unwrap();
        s.record_closed(None, "https://e2", "E2", 40, "window", Some(7)).unwrap();
        s.record_closed(None, "https://old", "Old", 5, "window", Some(3)).unwrap();
        let w = s.take_last_closed_window();
        assert_eq!(w.iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), ["https://e1", "https://e2"]);
        assert_eq!(s.take_last_closed_window().len(), 1, "then the older window");
        assert!(s.take_last_closed_window().is_empty());
    }

    #[test]
    fn permission_answers_are_remembered_per_site_and_kind() {
        let s = Store::open_memory();
        assert_eq!(s.remembered_permission("https://a.example", "camera"), None);
        s.remember_permission("https://a.example", "camera", "allow", 10).unwrap();
        s.remember_permission("https://a.example", "location", "deny", 11).unwrap();
        s.remember_permission("https://b.example", "camera", "deny", 12).unwrap();
        assert_eq!(s.remembered_permission("https://a.example", "camera").as_deref(), Some("allow"));
        assert_eq!(s.remembered_permission("https://b.example", "camera").as_deref(), Some("deny"));
        // A new answer replaces the old one.
        s.remember_permission("https://a.example", "camera", "deny", 20).unwrap();
        assert_eq!(s.remembered_permission("https://a.example", "camera").as_deref(), Some("deny"));
        assert_eq!(s.remembered_permissions().len(), 3);
        assert_eq!(s.forget_permissions(Some("https://a.example")).unwrap(), 2);
        assert_eq!(s.remembered_permissions().len(), 1);
        assert_eq!(s.forget_permissions(None).unwrap(), 1);
        assert!(s.remembered_permissions().is_empty());
    }

    #[test]
    fn an_apps_window_state_is_remembered_per_app() {
        let s = Store::open_memory();
        assert_eq!(s.app_window("mail"), None);
        s.save_app_window("mail", WindowState { width: 900, height: 700, maximized: false }).unwrap();
        s.save_app_window("chat", WindowState { width: 400, height: 800, maximized: true }).unwrap();
        s.save_app_window("mail", WindowState { width: 1000, height: 650, maximized: true }).unwrap();
        assert_eq!(s.app_window("mail"), Some(WindowState { width: 1000, height: 650, maximized: true }));
        assert_eq!(s.app_window("chat").map(|w| w.width), Some(400));
        s.delete_workspace("mail").unwrap();
        assert_eq!(s.app_window("mail"), None);
        assert!(s.app_window("chat").is_some());
    }

    #[test]
    fn delete_workspace_removes_only_its_tabs() {
        let s = Store::open_memory();
        s.save_tabs("a", &[rec(1, "https://a", true)]).unwrap();
        s.save_tabs("b", &[rec(2, "https://b", true)]).unwrap();
        s.delete_workspace("a").unwrap();
        assert!(s.load_tabs("a").is_empty());
        assert_eq!(s.load_tabs("b").len(), 1);
    }

    #[test]
    fn a_file_database_persists_and_migrates_once() {
        let dir = std::env::temp_dir().join(format!("browsah-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.save_tabs("w", &[rec(1, "https://a", true)]).unwrap();
        }
        let s = Store::open(&path).unwrap(); // opens again, migration does not wipe
        assert_eq!(s.load_tabs("w").len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
