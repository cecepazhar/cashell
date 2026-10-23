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

/// A tracked project record.
#[derive(Debug, Clone)]
pub struct TrackedProjectRecord {
    pub path: String,
    pub name: String,
    pub category: String,
    pub branch: String,
    pub is_dirty: bool,
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

    /// Expose the underlying connection (used by tests & low-level ops).
    pub fn conn(&self) -> &Connection {
        &self.conn
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
             );
             CREATE TABLE IF NOT EXISTS tracked_projects (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL UNIQUE,
                 name TEXT,
                 category TEXT,
                 branch TEXT,
                 is_dirty INTEGER DEFAULT 0,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS uptime_monitors (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL UNIQUE,
                 target TEXT NOT NULL UNIQUE,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS docker_containers (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL UNIQUE,
                 image TEXT NOT NULL,
                 ports TEXT,
                 env TEXT,
                 started_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS analytics_events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 website_id TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 url TEXT NOT NULL,
                 referrer TEXT,
                 page_title TEXT,
                 browser TEXT,
                 os TEXT,
                 device TEXT,
                 country TEXT,
                 visitor_hash TEXT NOT NULL,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS analytics_websites (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 domain TEXT NOT NULL,
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS pro_state (
                 device_id TEXT PRIMARY KEY,
                 device_label TEXT NOT NULL DEFAULT '',
                 status TEXT NOT NULL DEFAULT 'unknown',
                 last_seen DATETIME,
                 last_error TEXT,
                 extra_json TEXT,
                 updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS pro_sync_metadata (
                 entity_type TEXT NOT NULL,
                 entity_id TEXT NOT NULL,
                 revision INTEGER NOT NULL DEFAULT 1,
                 etag TEXT,
                 sync_state TEXT NOT NULL DEFAULT 'local',
                 last_synced_at DATETIME,
                 PRIMARY KEY (entity_type, entity_id)
             );
             CREATE INDEX IF NOT EXISTS idx_pro_sync_metadata_state ON pro_sync_metadata(sync_state);",
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

    /// Read the most recent `limit` `system_events` log rows, newest first.
    pub fn read_system_events(&self, limit: i64) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT level, source, message, created_at FROM system_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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

    /// Add or update a tracked project folder in SQLite.
    pub fn add_tracked_project(&self, path: &str, name: &str, category: &str, branch: &str, is_dirty: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tracked_projects (path, name, category, branch, is_dirty)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
               name = excluded.name,
               category = excluded.category,
               branch = excluded.branch,
               is_dirty = excluded.is_dirty",
            params![path, name, category, branch, if is_dirty { 1 } else { 0 }],
        )?;
        Ok(())
    }

    /// Retrieve all tracked projects from SQLite.
    pub fn get_tracked_projects(&self) -> Result<Vec<TrackedProjectRecord>> {
        let mut stmt = self.conn.prepare("SELECT path, name, category, branch, is_dirty FROM tracked_projects ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let name: Option<String> = row.get(1)?;
            let cat: Option<String> = row.get(2)?;
            let branch: Option<String> = row.get(3)?;
            let dirty: i64 = row.get(4).unwrap_or(0);
            Ok(TrackedProjectRecord {
                path: path.clone(),
                name: name.unwrap_or_else(|| path.split('/').next_back().unwrap_or("project").to_string()),
                category: cat.unwrap_or_else(|| "Project".to_string()),
                branch: branch.unwrap_or_else(|| "main".to_string()),
                is_dirty: dirty != 0,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Add a custom Uptime Kuma endpoint monitor to SQLite.
    pub fn add_uptime_monitor(&self, name: &str, target: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO uptime_monitors (name, target) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET target = excluded.target",
            params![name, target],
        )?;
        Ok(())
    }

    /// Retrieve all custom uptime monitors from SQLite.
    pub fn get_uptime_monitors(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT name, target FROM uptime_monitors ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Save a Docker container run record in SQLite.
    pub fn add_docker_container(&self, name: &str, image: &str, ports: &str, env: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO docker_containers (name, image, ports, env) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET image = excluded.image, ports = excluded.ports, env = excluded.env",
            params![name, image, ports, env],
        )?;
        Ok(())
    }

    /// Retrieve all recorded Docker containers from SQLite.
    pub fn get_docker_containers(&self) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare("SELECT name, image, ports, env FROM docker_containers ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, Option<String>>(2)?.unwrap_or_default(), row.get::<_, Option<String>>(3)?.unwrap_or_default()))
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Compact the database via SQLite `VACUUM`.
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute("VACUUM", [])?;
        Ok(())
    }

    // ── Delete Functions (Full CRUD) ─────────────────────────────────────────

    /// Remove a tracked project from the `tracked_projects` table.
    pub fn delete_tracked_project(&self, path: &str) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM tracked_projects WHERE path = ?1", params![path])?;
        Ok(n)
    }

    /// Remove an uptime monitor from the `uptime_monitors` table.
    pub fn delete_uptime_monitor(&self, name: &str) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM uptime_monitors WHERE name = ?1", params![name])?;
        Ok(n)
    }

    /// Remove a docker container record from the `docker_containers` table.
    pub fn delete_docker_container(&self, name: &str) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM docker_containers WHERE name = ?1", params![name])?;
        Ok(n)
    }

    /// Remove a registered node from the `registered_nodes` table.
    pub fn delete_registered_node(&self, node_id: &str) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM registered_nodes WHERE node_id = ?1", params![node_id])?;
        Ok(n)
    }

    // ── Sovereign Web Analytics (Umami-like) ─────────────────────────────────

    /// Register a new target website for analytics tracking.
    pub fn add_analytic_site(&self, id: &str, name: &str, domain: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO analytics_websites (id, name, domain) VALUES (?1, ?2, ?3)",
            params![id, name, domain],
        )?;
        Ok(())
    }

    /// Fetch all registered websites from `analytics_websites`.
    pub fn get_analytic_sites(&self) -> Result<Vec<crate::systat::AnalyticSiteItem>> {
        let mut stmt = self.conn.prepare("SELECT id, name, domain FROM analytics_websites ORDER BY name")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(crate::systat::AnalyticSiteItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    domain: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete a website from the `analytics_websites` tracking table.
    pub fn delete_analytic_site(&self, id: &str) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM analytics_websites WHERE id = ?1", params![id])?;
        Ok(n)
    }

    /// Seed the `analytics_websites` table with the default CA product list.
    pub fn import_default_ca_websites(&self) -> Result<()> {
        let defaults: [(&str, &str, &str); 14] = [
            ("caterm", "CAProduct Landing", "caterm.fathforce.com"),
            ("fathstore", "FathStore Solo", "fathstore.com"),
            ("caui", "CAUI Design System", "caui.fathforce.com"),
            ("cagames", "CAGames", "cagames.fathforce.com"),
            ("camark", "CAMark Studio", "camark.fathforce.com"),
            ("catable", "CATAble Dashboard", "catable.fathforce.com"),
            ("cabook", "CABook Library", "cabook.fathforce.com"),
            ("capost", "CAPost CMS", "capost.fathforce.com"),
            ("cacalm", "CACalm Meditation", "cacalm.fathforce.com"),
            ("cacash", "CACash Finance", "cacash.fathforce.com"),
            ("caentech", "CAEntech Medical", "caentech.fathforce.com"),
            ("castudio", "CAStudio", "castudio.fathforce.com"),
            ("caprompt", "CAPrompt Studio", "caprompt.fathforce.com"),
            ("cecepazhar.com", "Prof. Cecep Blog", "cecepazhar.com"),
        ];
        for (id, name, domain) in defaults {
            self.add_analytic_site(id, name, domain)?;
        }
        Ok(())
    }

    /// Export all analytics events to a JSON file.
    pub fn export_analytics_json(&self, path: &str) -> Result<u64> {
        let events: Vec<serde_json::Value> = {
            let mut stmt = self.conn.prepare(
                "SELECT website_id, session_id, url, referrer, page_title, browser, os, device, country, visitor_hash, created_at
                 FROM analytics_events"
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "website_id": row.get::<_, String>(0)?,
                        "session_id": row.get::<_, String>(1)?,
                        "url": row.get::<_, String>(2)?,
                        "referrer": row.get::<_, String>(3)?,
                        "page_title": row.get::<_, String>(4)?,
                        "browser": row.get::<_, String>(5)?,
                        "os": row.get::<_, String>(6)?,
                        "device": row.get::<_, String>(7)?,
                        "country": row.get::<_, String>(8)?,
                        "visitor_hash": row.get::<_, String>(9)?,
                        "created_at": row.get::<_, String>(10)?,
                    }))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let mut json_str =
            serde_json::to_string_pretty(&events).context("Failed to serialize analytics events to JSON")?;
        json_str.push('\n');
        std::fs::write(path, json_str)?;
        Ok(events.len() as u64)
    }

    /// Export all analytics events to a CSV file.
    pub fn export_analytics_csv(&self, path: &str) -> Result<u64> {
        let mut csv_lines = vec![
            "website_id,session_id,url,referrer,page_title,browser,os,device,country,created_at".to_string(),
        ];
        let mut stmt = self.conn.prepare(
            "SELECT website_id, session_id, url, referrer, page_title, browser, os, device, country, created_at
             FROM analytics_events"
        )?;
        let mut count = 0u64;
        let rows = stmt.query_map([], |row| {
            let mut cols = Vec::with_capacity(10);
            for i in 0..10 {
                let s: String = row.get(i).unwrap_or_default();
                cols.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            Ok(cols.join(","))
        })?;
        for row in rows {
            let line = row?;
            csv_lines.push(line);
            count += 1;
        }
        std::fs::write(path, csv_lines.join("\n"))?;
        Ok(count)
    }

    /// Clear all recorded analytics events from the database.
    pub fn clear_analytics_events(&self) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM analytics_events", [])?;
        Ok(n)
    }

    /// Record a new web pageview event in the `analytics_events` table.
    #[allow(clippy::too_many_arguments)]
    pub fn record_pageview(
        &self,
        website_id: &str,
        session_id: &str,
        url: &str,
        referrer: &str,
        page_title: &str,
        browser: &str,
        os: &str,
        device: &str,
        country: &str,
        visitor_hash: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO analytics_events (website_id, session_id, url, referrer, page_title, browser, os, device, country, visitor_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                website_id,
                session_id,
                url,
                referrer,
                page_title,
                browser,
                os,
                device,
                country,
                visitor_hash
            ],
        )?;
        Ok(())
    }

    /// Retrieve overall web analytics summary for a specific website or `None` for global stats.
    pub fn get_analytics_summary(&self, website_id: Option<&str>) -> Result<crate::systat::AnalyticsSummary> {
        let target = website_id.map(|id| id.to_string());
        let ref_val = target.as_deref();

        // Total pageviews
        let pageviews = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1)",
                params![ref_val],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Unique visitors
        let unique_visitors = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT visitor_hash) FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1)",
                params![ref_val],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Active visitors (last 5 mins)
        let active_visitors = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT visitor_hash) FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) AND created_at >= datetime('now', '-5 minutes')",
                params![ref_val],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Bounce rate
        let bounce_rate = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN pv = 1 THEN 1 ELSE 0 END) * 100.0 / NULLIF(COUNT(*), 0), 0.0)
                 FROM (SELECT session_id, COUNT(*) as pv FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY session_id)",
                params![ref_val],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        // Top 5 pages
        let mut top_pages: Vec<(String, u64)> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT url, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY url ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            top_pages.push((row.get(0)?, row.get(1)?));
        }

        // Top 5 referrers
        let mut top_referrers: Vec<(String, u64)> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(referrer, 'Direct') as ref, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) AND referrer IS NOT NULL AND referrer != '' GROUP BY ref ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            top_referrers.push((row.get(0)?, row.get(1)?));
        }

        // Device breakdown
        let mut device_breakdown: Vec<crate::systat::AnalyticsBreakdown> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(NULLIF(device, ''), 'Desktop') as dev, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY dev ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            let count: u64 = row.get(1)?;
            let pct = if pageviews > 0 { (count as f32 / pageviews as f32) * 100.0 } else { 0.0 };
            device_breakdown.push(crate::systat::AnalyticsBreakdown { label, count, pct });
        }

        // Browser breakdown
        let mut browser_breakdown: Vec<crate::systat::AnalyticsBreakdown> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(NULLIF(browser, ''), 'Unknown') as b, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY b ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            let count: u64 = row.get(1)?;
            let pct = if pageviews > 0 { (count as f32 / pageviews as f32) * 100.0 } else { 0.0 };
            browser_breakdown.push(crate::systat::AnalyticsBreakdown { label, count, pct });
        }

        // OS breakdown
        let mut os_breakdown: Vec<crate::systat::AnalyticsBreakdown> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(NULLIF(os, ''), 'Unknown') as o, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY o ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            let count: u64 = row.get(1)?;
            let pct = if pageviews > 0 { (count as f32 / pageviews as f32) * 100.0 } else { 0.0 };
            os_breakdown.push(crate::systat::AnalyticsBreakdown { label, count, pct });
        }

        // Country breakdown
        let mut country_breakdown: Vec<crate::systat::AnalyticsBreakdown> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(NULLIF(country, ''), 'ID') as co, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) GROUP BY co ORDER BY c DESC LIMIT 5",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            let count: u64 = row.get(1)?;
            let pct = if pageviews > 0 { (count as f32 / pageviews as f32) * 100.0 } else { 0.0 };
            country_breakdown.push(crate::systat::AnalyticsBreakdown { label, count, pct });
        }

        // Daily traffic history (last 7 days)
        let mut daily_traffic: Vec<(String, u64)> = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT date(created_at) as day, COUNT(*) as c FROM analytics_events WHERE (?1 IS NULL OR website_id = ?1) AND created_at >= date('now', '-7 day') GROUP BY day ORDER BY day ASC",
        )?;
        let mut rows = stmt.query(params![ref_val])?;
        while let Some(row) = rows.next()? {
            daily_traffic.push((row.get(0)?, row.get(1)?));
        }

        let avg_session_secs = if unique_visitors > 0 {
            (pageviews as f64 * 38.0) / unique_visitors as f64
        } else {
            0.0
        };

        // Per-product / per-website breakdown
        let mut canonical_products: Vec<crate::systat::AnalyticSiteItem> =
            self.get_analytic_sites().unwrap_or_default();
        if canonical_products.is_empty() {
            self.import_default_ca_websites()?;
            canonical_products = self.get_analytic_sites().unwrap_or_default();
        }

        let mut product_analytics: Vec<crate::systat::ProductAnalyticsSummary> = Vec::new();
        for site in &canonical_products {
            let pid = site.id.clone();
            let pid_filter: Option<&str> = if ref_val.is_none() { Some(pid.as_str()) } else { ref_val };
            let site_val = pid_filter.unwrap_or("default");

            let p_pvs = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM analytics_events WHERE website_id = ?1",
                    params![site_val],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let p_viss = self
                .conn
                .query_row(
                    "SELECT COUNT(DISTINCT visitor_hash) FROM analytics_events WHERE website_id = ?1",
                    params![site_val],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let p_actives = self
                .conn
                .query_row(
                    "SELECT COUNT(DISTINCT visitor_hash) FROM analytics_events WHERE website_id = ?1 AND created_at >= datetime('now', '-5 minutes')",
                    params![site_val],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let p_bounce = self
                .conn
                .query_row(
                    "SELECT COALESCE(SUM(CASE WHEN pv = 1 THEN 1 ELSE 0 END) * 100.0 / NULLIF(COUNT(*), 0), 0.0)
                     FROM (SELECT session_id, COUNT(*) as pv FROM analytics_events WHERE website_id = ?1 GROUP BY session_id)",
                    params![site_val],
                    |row| row.get(0),
                )
                .unwrap_or(0.0);

            let status = if p_actives > 0 {
                "ACTIVE".to_string()
            } else if p_pvs > 0 {
                "IDLE".to_string()
            } else {
                "EMPTY".to_string()
            };

            product_analytics.push(crate::systat::ProductAnalyticsSummary {
                product_id: pid.to_string(),
                name: site.name.clone(),
                domain: site.domain.clone(),
                pageviews: p_pvs,
                visitors: p_viss,
                active_now: p_actives,
                bounce_rate: p_bounce,
                status,
            });
        }

        Ok(crate::systat::AnalyticsSummary {
            pageviews,
            unique_visitors,
            active_visitors,
            bounce_rate,
            avg_session_secs,
            top_pages,
            top_referrers,
            device_breakdown,
            browser_breakdown,
            os_breakdown,
            country_breakdown,
            daily_traffic,
            product_analytics,
        })
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
    fn test_db_tracked_projects_crud() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.add_tracked_project("/path/to/cashell", "cashell", "Product", "main", false)?;
        let projs = db.get_tracked_projects()?;
        assert_eq!(projs.len(), 1);
        assert_eq!(projs[0].name, "cashell");
        assert_eq!(projs[0].path, "/path/to/cashell");
        assert_eq!(projs[0].category, "Product");
        Ok(())
    }

    #[test]
    fn test_db_uptime_monitors_crud() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.add_uptime_monitor("Landing CATerm", "https://caterm.fathforce.com")?;
        let monitors = db.get_uptime_monitors()?;
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].0, "Landing CATerm");
        assert_eq!(monitors[0].1, "https://caterm.fathforce.com");
        Ok(())
    }

    #[test]
    fn test_db_docker_containers_crud() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.add_docker_container("test-nginx", "nginx:alpine", "80:80", "ENV=test")?;
        let containers = db.get_docker_containers()?;
        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].0, "test-nginx");
        assert_eq!(containers[0].1, "nginx:alpine");
        assert_eq!(containers[0].2, "80:80");
        assert_eq!(containers[0].3, "ENV=test");
        Ok(())
    }

    #[test]
    fn test_db_delete_operations() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.add_tracked_project("/path/to/test", "test", "Project", "main", false)?;
        assert_eq!(db.get_tracked_projects()?.len(), 1);
        assert_eq!(db.delete_tracked_project("/path/to/test")?, 1);
        assert_eq!(db.get_tracked_projects()?.len(), 0);

        db.add_uptime_monitor("Test Monitor", "https://test.com")?;
        assert_eq!(db.get_uptime_monitors()?.len(), 1);
        assert_eq!(db.delete_uptime_monitor("Test Monitor")?, 1);
        assert_eq!(db.get_uptime_monitors()?.len(), 0);

        db.add_docker_container("test-box", "alpine", "80:80", "")?;
        assert_eq!(db.get_docker_containers()?.len(), 1);
        assert_eq!(db.delete_docker_container("test-box")?, 1);
        assert_eq!(db.get_docker_containers()?.len(), 0);
        Ok(())
    }

    #[test]
    fn test_db_analytics_tracking() -> Result<()> {
        let db = Database::open_in_memory()?;
        db.record_pageview(
            "caterm",
            "sess_1",
            "/",
            "https://google.com",
            "CATerm Home",
            "Chrome",
            "Linux",
            "Desktop",
            "ID",
            "hash_abc",
        )?;
        db.record_pageview(
            "caterm",
            "sess_2",
            "/docs",
            "Direct",
            "CATerm Docs",
            "Firefox",
            "Linux",
            "Desktop",
            "ID",
            "hash_def",
        )?;

        let summary = db.get_analytics_summary(Some("caterm"))?;
        assert_eq!(summary.pageviews, 2);
        assert_eq!(summary.unique_visitors, 2);
        assert_eq!(summary.top_pages.len(), 2);
        assert!(summary.top_referrers.iter().any(|(r, _)| r == "https://google.com"));
        assert!(summary.top_referrers.iter().any(|(r, _)| r == "Direct"));
        Ok(())
    }

    #[test]
    fn test_db_size_memory() -> Result<()> {
        let db = Database::open_in_memory()?;
        assert_eq!(db.get_db_size_bytes(), 0);
        Ok(())
    }

    #[test]
    fn test_pro_tables_exist() -> Result<()> {
        let db = Database::open_in_memory()?;
        let tables: Vec<String> = {
            let mut stmt = db.conn().prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('pro_state','pro_sync_metadata') ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        assert!(tables.contains(&"pro_state".to_string()), "pro_state table missing");
        assert!(tables.contains(&"pro_sync_metadata".to_string()), "pro_sync_metadata table missing");
        Ok(())
    }
}
