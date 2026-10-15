//! SQLite database persistence module for cashell.
//!
//! Stores AI chat history, command history, audit reports, and system events
//! at `~/.config/cashell/cashell.db`.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// A heartbeat record from a remote node agent.
#[derive(Debug, Clone)]
pub struct NodeHeartbeat {
    pub node_id: String,
    pub cpu_percent: f32,
    pub ram_used_mb: i64,
    pub ram_total_mb: i64,
    pub disk_used_gb: i64,
    pub disk_total_gb: i64,
    pub services_json: String,
    pub uptime_secs: i64,
    pub version: String,
    pub received_at: String,
    pub display_name: Option<String>,
    pub ip: Option<String>,
    pub signature: Option<String>,
}

/// A registered node in the fleet.
#[derive(Debug, Clone)]
pub struct RegisteredNode {
    pub node_id: String,
    pub display_name: Option<String>,
    pub ip: Option<String>,
    pub pubkey: Option<String>,
    pub last_seen: Option<String>,
    pub registered_at: String,
}

/// A virtual pet (Tamagotchi) entity.
#[derive(Debug, Clone)]
pub struct Pet {
    pub id: i64,
    pub species: String,
    pub name: String,
    pub stage: String,
    pub xp: i64,
    pub born_at: String,
    pub last_active_at: String,
}

/// A cron job record.
#[derive(Debug, Clone)]
pub struct CronJob {
    pub id: i64,
    pub schedule: String,
    pub command: String,
    pub description: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub last_exit_code: Option<i64>,
    pub created_at: String,
}

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
             );
             CREATE TABLE IF NOT EXISTS node_heartbeats (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 node_id TEXT NOT NULL,
                 cpu_percent REAL,
                 ram_used_mb INTEGER,
                 ram_total_mb INTEGER,
                 disk_used_gb INTEGER,
                 disk_total_gb INTEGER,
                 services_json TEXT,
                 uptime_secs INTEGER,
                 version TEXT,
                 received_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE INDEX IF NOT EXISTS idx_heartbeats_node ON node_heartbeats(node_id, received_at);
             CREATE TABLE IF NOT EXISTS registered_nodes (
                 node_id TEXT PRIMARY KEY,
                 display_name TEXT,
                 ip TEXT,
                 pubkey TEXT,
                 last_seen DATETIME,
                 registered_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS pets (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 species TEXT NOT NULL,
                 name TEXT NOT NULL,
                 stage TEXT DEFAULT 'egg',
                 xp INTEGER DEFAULT 0,
                 born_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                 last_active_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS cron_jobs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 schedule TEXT NOT NULL,
                 command TEXT NOT NULL,
                 description TEXT DEFAULT '',
                 enabled INTEGER DEFAULT 1,
                 last_run DATETIME,
                 last_exit_code INTEGER,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS pet_action_logs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 pet_id INTEGER,
                 pet_name TEXT,
                 action_type TEXT,
                 detail TEXT,
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
        deleted += self.conn.execute(
            "DELETE FROM node_heartbeats WHERE received_at < datetime('now', ?1)",
            params![modifier],
        )?;
        deleted += self.conn.execute(
            "DELETE FROM cron_jobs WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        deleted += self.conn.execute(
            "DELETE FROM pet_action_logs WHERE created_at < datetime('now', ?1)",
            params![modifier],
        )?;
        Ok(deleted)
    }

    // ── Node Heartbeat Storage ──────────────────────────────────────────────

    /// Store a heartbeat from a remote node.
    pub fn store_heartbeat(&self, hb: &NodeHeartbeat) -> Result<()> {
        self.conn.execute(
            "INSERT INTO node_heartbeats (node_id, cpu_percent, ram_used_mb, ram_total_mb, disk_used_gb, disk_total_gb, services_json, uptime_secs, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                hb.node_id,
                hb.cpu_percent,
                hb.ram_used_mb,
                hb.ram_total_mb,
                hb.disk_used_gb,
                hb.disk_total_gb,
                hb.services_json,
                hb.uptime_secs,
                hb.version,
            ],
        )?;
        // Upsert registered_nodes
        self.conn.execute(
            "INSERT INTO registered_nodes (node_id, display_name, ip, last_seen)
             VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
             ON CONFLICT(node_id) DO UPDATE SET
               display_name = COALESCE(excluded.display_name, display_name),
               ip = COALESCE(excluded.ip, ip),
               last_seen = CURRENT_TIMESTAMP",
            params![hb.node_id, hb.display_name, hb.ip],
        )?;
        Ok(())
    }

    /// Register a node with its public key.
    pub fn register_node(&self, node_id: &str, display_name: &str, ip: &str, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO registered_nodes (node_id, display_name, ip, pubkey, last_seen)
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
             ON CONFLICT(node_id) DO UPDATE SET
               display_name = excluded.display_name,
               ip = excluded.ip,
               pubkey = excluded.pubkey,
               last_seen = CURRENT_TIMESTAMP",
            params![node_id, display_name, ip, pubkey],
        )?;
        Ok(())
    }

    /// Get all registered nodes.
    pub fn get_all_nodes(&self) -> Result<Vec<RegisteredNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, display_name, ip, pubkey, last_seen, registered_at FROM registered_nodes ORDER BY node_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RegisteredNode {
                node_id: row.get(0)?,
                display_name: row.get(1)?,
                ip: row.get(2)?,
                pubkey: row.get(3)?,
                last_seen: row.get(4)?,
                registered_at: row.get(5)?,
            })
        })?;
        let mut nodes = Vec::new();
        for r in rows {
            nodes.push(r?);
        }
        Ok(nodes)
    }

    /// Get latest heartbeat for a specific node.
    pub fn get_latest_heartbeat(&self, node_id: &str) -> Result<Option<NodeHeartbeat>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, cpu_percent, ram_used_mb, ram_total_mb, disk_used_gb, disk_total_gb, services_json, uptime_secs, version, received_at
             FROM node_heartbeats WHERE node_id = ?1 ORDER BY received_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![node_id], |row| {
            Ok(NodeHeartbeat {
                node_id: row.get(0)?,
                cpu_percent: row.get(1)?,
                ram_used_mb: row.get(2)?,
                ram_total_mb: row.get(3)?,
                disk_used_gb: row.get(4)?,
                disk_total_gb: row.get(5)?,
                services_json: row.get(6)?,
                uptime_secs: row.get(7)?,
                version: row.get(8)?,
                received_at: row.get(9)?,
                display_name: None,
                ip: None,
                signature: None,
            })
        })?;
        match rows.next() {
            Some(Ok(hb)) => Ok(Some(hb)),
            _ => Ok(None),
        }
    }

    /// Get latest heartbeats for all nodes (one per node).
    pub fn get_all_latest_heartbeats(&self) -> Result<Vec<NodeHeartbeat>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.node_id, h.cpu_percent, h.ram_used_mb, h.ram_total_mb, h.disk_used_gb, h.disk_total_gb, h.services_json, h.uptime_secs, h.version, h.received_at, n.display_name, n.ip
             FROM node_heartbeats h
             INNER JOIN (SELECT node_id, MAX(received_at) as max_ts FROM node_heartbeats GROUP BY node_id) latest
               ON h.node_id = latest.node_id AND h.received_at = latest.max_ts
             LEFT JOIN registered_nodes n ON h.node_id = n.node_id
             ORDER BY h.node_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NodeHeartbeat {
                node_id: row.get(0)?,
                cpu_percent: row.get(1)?,
                ram_used_mb: row.get(2)?,
                ram_total_mb: row.get(3)?,
                disk_used_gb: row.get(4)?,
                disk_total_gb: row.get(5)?,
                services_json: row.get(6)?,
                uptime_secs: row.get(7)?,
                version: row.get(8)?,
                received_at: row.get(9)?,
                display_name: row.get(10)?,
                ip: row.get(11)?,
                signature: None,
            })
        })?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    // ── Pet (Tamagotchi) Operations ──────────────────────────────────────────

    /// Get current active pet, or None if none created.
    pub fn get_active_pet(&self) -> Result<Option<Pet>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, species, name, stage, xp, born_at, last_active_at FROM pets ORDER BY id DESC LIMIT 1"
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(Pet {
                id: row.get(0)?,
                species: row.get(1)?,
                name: row.get(2)?,
                stage: row.get(3)?,
                xp: row.get(4)?,
                born_at: row.get(5)?,
                last_active_at: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(Ok(p)) => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    /// Create a new pet.
    pub fn create_pet(&self, species: &str, name: &str) -> Result<Pet> {
        self.conn.execute(
            "INSERT INTO pets (species, name, stage, xp) VALUES (?1, ?2, 'egg', 0)",
            params![species, name],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(Pet {
            id,
            species: species.to_string(),
            name: name.to_string(),
            stage: "egg".to_string(),
            xp: 0,
            born_at: chrono::Local::now().to_rfc3339(),
            last_active_at: chrono::Local::now().to_rfc3339(),
        })
    }

    /// Add XP to active pet and advance stage if threshold reached.
    pub fn add_pet_xp(&self, id: i64, xp_to_add: i64) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT species, stage, xp FROM pets WHERE id = ?1"
        )?;
        let pet_opt = stmt.query_row(params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        }).ok();

        if let Some((_species, current_stage, current_xp)) = pet_opt {
            let new_xp = current_xp + xp_to_add;
            // Stage progression: egg(0), hatch(60), juvenile(180), adult(480), mating(960), egg_again(1200)
            let new_stage = if new_xp >= 1200 {
                "egg".to_string() // resets to egg (laid an egg!)
            } else if new_xp >= 960 {
                "mating".to_string()
            } else if new_xp >= 480 {
                "adult".to_string()
            } else if new_xp >= 180 {
                "juvenile".to_string()
            } else if new_xp >= 60 {
                "hatch".to_string()
            } else {
                current_stage
            };

            let final_xp = if new_xp >= 1200 { 0 } else { new_xp };

            self.conn.execute(
                "UPDATE pets SET xp = ?1, stage = ?2, last_active_at = CURRENT_TIMESTAMP WHERE id = ?3",
                params![final_xp, new_stage, id],
            )?;
        }
        Ok(())
    }

    // ── Cron Jobs Operations ─────────────────────────────────────────────────

    /// Get all saved cron jobs.
    pub fn get_cron_jobs(&self) -> Result<Vec<CronJob>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, schedule, command, description, enabled, last_run, last_exit_code, created_at FROM cron_jobs ORDER BY id ASC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CronJob {
                id: row.get(0)?,
                schedule: row.get(1)?,
                command: row.get(2)?,
                description: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                last_run: row.get(5)?,
                last_exit_code: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Add a cron job.
    pub fn add_cron_job(&self, schedule: &str, command: &str, description: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO cron_jobs (schedule, command, description, enabled) VALUES (?1, ?2, ?3, 1)",
            params![schedule, command, description],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a cron job by id.
    pub fn delete_cron_job(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Update cron job last run status.
    pub fn update_cron_run(&self, id: i64, exit_code: i32) -> Result<()> {
        self.conn.execute(
            "UPDATE cron_jobs SET last_run = CURRENT_TIMESTAMP, last_exit_code = ?1 WHERE id = ?2",
            params![exit_code, id],
        )?;
        Ok(())
    }

    /// Log a pet interaction event.
    pub fn log_pet_action(&self, pet_id: i64, pet_name: &str, action_type: &str, detail: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pet_action_logs (pet_id, pet_name, action_type, detail) VALUES (?1, ?2, ?3, ?4)",
            params![pet_id, pet_name, action_type, detail],
        )?;
        Ok(())
    }

    /// Get recent pet action logs.
    pub fn get_pet_history(&self, limit: usize) -> Result<Vec<crate::pet::PetActionLog>> {
        let mut stmt = self.conn.prepare(
            "SELECT created_at, action_type, detail FROM pet_action_logs ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::pet::PetActionLog {
                timestamp: row.get(0)?,
                action_type: row.get(1)?,
                detail: row.get(2)?,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
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
             DELETE FROM system_events;
             DELETE FROM node_heartbeats;
             DELETE FROM cron_jobs;",
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

/// Open default database.
pub fn get_db() -> Result<Database> {
    Database::open()
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
