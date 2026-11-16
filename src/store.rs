//! The profile's database (`data.sqlite` in the profile directory): state the program writes and
//! reads back, as opposed to configuration a person edits. Today: each workspace's tabs, so a
//! workspace reopens as it was; later bookmarks and history.
//!
//! One connection per process, used from the GTK thread only.

use rusqlite::{params, Connection};
use std::path::Path;

/// Bump when the tables change and add a migration to `migrate`.
const SCHEMA_VERSION: i64 = 1;

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
        self.conn.pragma_update(None, "user_version", SCHEMA_VERSION)
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

    /// Forget everything stored for a workspace.
    pub fn delete_workspace(&self, workspace: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM tabs WHERE workspace = ?1", params![workspace])?;
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
