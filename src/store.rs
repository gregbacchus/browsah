//! The profile's database (`data.sqlite` in the profile directory): state the program writes and
//! reads back, as opposed to configuration a person edits. Today: each workspace's tabs, so a
//! workspace reopens as it was; later bookmarks and history.
//!
//! One connection per process, used from the GTK thread only.

use rusqlite::{params, Connection};
use std::path::Path;

/// Bump when the tables change and add a migration to `migrate`.
const SCHEMA_VERSION: i64 = 3;

/// A tab as remembered between runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRecord {
    /// Stable for the life of the tab; names its screenshot file.
    pub uid: u64,
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub selected: bool,
}

/// A bookmark: one per address in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
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
        self.conn.pragma_update(None, "user_version", SCHEMA_VERSION)
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
            let mut insert = tx.prepare("INSERT INTO tabs (workspace, position, uid, url, title, pinned, selected) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
            for (i, t) in tabs.iter().enumerate() {
                insert.execute(params![workspace, i as i64, t.uid as i64, t.url, t.title, t.pinned, t.selected])?;
            }
        }
        tx.commit()
    }

    /// A workspace's remembered tabs, in order.
    pub fn load_tabs(&self, workspace: &str) -> Vec<TabRecord> {
        let Ok(mut stmt) = self.conn.prepare("SELECT uid, url, title, pinned, selected FROM tabs WHERE workspace = ?1 ORDER BY position") else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![workspace], |r| {
            Ok(TabRecord { uid: r.get::<_, i64>(0)? as u64, url: r.get(1)?, title: r.get(2)?, pinned: r.get(3)?, selected: r.get(4)? })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
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
        self.conn.execute("DELETE FROM bookmarks WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM visits WHERE workspace = ?1", params![workspace])?;
        self.conn.execute("DELETE FROM closed_tabs WHERE workspace = ?1", params![workspace])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(uid: u64, url: &str, selected: bool) -> TabRecord {
        TabRecord { uid, url: url.into(), title: format!("t{uid}"), pinned: uid == 1, selected }
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
