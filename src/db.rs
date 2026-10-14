//! SQLite database persistence module for cashell.
//!
//! Stores AI chat history, command history, audit reports, and system events
//! at `~/.config/cashell/cashell.db`.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// Primary SQLite persistence manager for cashell.
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// Return the default SQLite database path (`~/.config/cashell/cashell.db`).
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("cashell")
            .join("cashell.db")
    }

    /// Open or create the database at the default path.
    pub fn open() -> Result<Self> {
        Self::open_path(&Self::default_path())
    }

    /// Open or create the database at a specific path.
    pub fn open_path(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed creating DB directory {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed opening SQLite database at {}", path.display()))?;

        let db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.init()?;
        Ok(db)
    }

    /// In-memory database for unit tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn,
            path: PathBuf::from(":memory:"),
        };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS ai_chat (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT,
                 role TEXT,
                 content TEXT,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS command_history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 command TEXT,
                 args TEXT,
                 exit_code INTEGER,
                 duration_ms INTEGER,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS audit_reports (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 repo_path TEXT,
                 critical_count INTEGER,
                 high_count INTEGER,
                 report_path TEXT,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS system_events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 level TEXT,
                 source TEXT,
                 message TEXT,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );",
        )?;
        Ok(())
    }

    /// Save a chat turn to the `ai_chat` table.
    pub fn save_chat(&self, role: &str, content: &str) -> Result<()> {
        self.save_chat_session("default", role, content)
    }

    /// Save a chat turn with an explicit session identifier.
    pub fn save_chat_session(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_chat (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, content],
        )?;
        Ok(())
    }

    /// Load the most recent `limit` messages in chronological order.
    pub fn load_recent_chat(&self, limit: usize) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM ai_chat ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut msgs = Vec::new();
        for r in rows {
            msgs.push(r?);
        }
        msgs.reverse();
        Ok(msgs)
    }

    /// Append a log entry to `system_events`.
    pub fn log_event(&self, level: &str, source: &str, msg: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO system_events (level, source, message) VALUES (?1, ?2, ?3)",
            params![level, source, msg],
        )?;
        Ok(())
    }

    /// Delete records older than `days` days across all historical tables.
    pub fn prune_older_than(&self, days: u32) -> Result<usize> {
        let modifier = format!("-{} days", days);
        let mut deleted = 0;
        deleted += self.conn.execute(
            "DELETE FROM ai_chat WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        deleted += self.conn.execute(
            "DELETE FROM command_history WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        deleted += self.conn.execute(
            "DELETE FROM audit_reports WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        deleted += self.conn.execute(
            "DELETE FROM system_events WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        Ok(deleted)
    }

    /// Calculate total database size in bytes, including WAL and SHM journal files.
    pub fn get_db_size_bytes(&self) -> u64 {
        if self.path == Path::new(":memory:") {
            return 0;
        }
        let mut total = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        if let Ok(m) = std::fs::metadata(wal) {
            total += m.len();
        }
        let shm = PathBuf::from(format!("{}-shm", self.path.display()));
        if let Ok(m) = std::fs::metadata(shm) {
            total += m.len();
        }
        total
    }

    /// Truncate all records from the `ai_chat` table.
    pub fn clear_chat(&self) -> Result<()> {
        self.conn.execute("DELETE FROM ai_chat", [])?;
        Ok(())
    }

    /// Truncate all records across all database tables.
    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM ai_chat;
             DELETE FROM command_history;
             DELETE FROM audit_reports;
             DELETE FROM system_events;",
        )?;
        Ok(())
    }

    /// Compact the database via SQLite `VACUUM`.
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute("VACUUM", [])?;
        Ok(())
    }
}

// ── Top-level convenience wrappers ──────────────────────────────────────────

/// Return the default DB path.
pub fn default_path() -> PathBuf {
    Database::default_path()
}

/// Open the default database.
pub fn open() -> Result<Database> {
    Database::open()
}

/// Save a chat turn to the default database.
pub fn save_chat(role: &str, content: &str) -> Result<()> {
    Database::open()?.save_chat(role, content)
}

/// Load recent chat messages in chronological order from default database.
pub fn load_recent_chat(limit: usize) -> Result<Vec<(String, String)>> {
    Database::open()?.load_recent_chat(limit)
}

/// Log a system event to default database.
pub fn log_event(level: &str, source: &str, msg: &str) -> Result<()> {
    Database::open()?.log_event(level, source, msg)
}

/// Prune entries older than `days` days from default database.
pub fn prune_older_than(days: u32) -> Result<usize> {
    Database::open()?.prune_older_than(days)
}

/// Calculate current database disk usage in bytes.
pub fn get_db_size_bytes() -> u64 {
    let path = default_path();
    let mut total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    if let Ok(m) = std::fs::metadata(wal) {
        total += m.len();
    }
    let shm = PathBuf::from(format!("{}-shm", path.display()));
    if let Ok(m) = std::fs::metadata(shm) {
        total += m.len();
    }
    total
}

/// Clear chat messages from default database.
pub fn clear_chat() -> Result<()> {
    Database::open()?.clear_chat()
}

/// Clear all tables in default database.
pub fn clear_all() -> Result<()> {
    Database::open()?.clear_all()
}

/// Run SQLite vacuum on default database.
pub fn vacuum() -> Result<()> {
    Database::open()?.vacuum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_init_and_chat_roundtrip() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.save_chat("User", "Hello Hana")?;
        db.save_chat("AI", "Greetings Prof. Cecep")?;

        let recent = db.load_recent_chat(10)?;
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0], ("User".to_string(), "Hello Hana".to_string()));
        assert_eq!(recent[1], ("AI".to_string(), "Greetings Prof. Cecep".to_string()));
        Ok(())
    }

    #[test]
    fn test_db_chat_limit_order() -> Result<()> {
        let db = Database::open_in_memory()?;
        for i in 1..=5 {
            db.save_chat("User", &format!("Msg {i}"))?;
        }

        let recent = db.load_recent_chat(3)?;
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].1, "Msg 3");
        assert_eq!(recent[1].1, "Msg 4");
        assert_eq!(recent[2].1, "Msg 5");
        Ok(())
    }

    #[test]
    fn test_db_log_event_and_prune() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.log_event("INFO", "cockpit", "System booted")?;
        db.log_event("WARN", "tailscale", "Interface latency")?;

        // Pruning older than 1 day should not touch brand-new entries
        let pruned = db.prune_older_than(1)?;
        assert_eq!(pruned, 0);
        Ok(())
    }

    #[test]
    fn test_db_clear_and_vacuum() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.save_chat("User", "Secret prompt")?;
        db.log_event("INFO", "test", "event")?;

        db.clear_chat()?;
        let chat_after = db.load_recent_chat(10)?;
        assert!(chat_after.is_empty());

        db.clear_all()?;
        db.vacuum()?;
        Ok(())
    }

    #[test]
    fn test_db_size_memory() -> Result<()> {
        let db = Database::open_in_memory()?;
        assert_eq!(db.get_db_size_bytes(), 0);
        Ok(())
    }
}
