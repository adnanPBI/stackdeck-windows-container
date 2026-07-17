//! SQLite state management for PyStack Runner.
//!
/// Replaces the StateDB class from `core.py` — persists service state
/// in a SQLite database with the same schema.
use pystack_types::ServiceStatus;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// State database handle.
pub struct StateDb {
    conn: Connection,
    project: String,
}

impl StateDb {
    /// Open (or create) the state database at the given directory.
    /// The database file will be `dir/state.db`.
    pub fn open(project: &str, dir: &Path) -> Result<Self, StateError> {
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join("state.db");
        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=10000;")?;
        let mut db = Self {
            conn,
            project: project.to_string(),
        };
        db.init()?;
        Ok(db)
    }

    /// Initialize the schema (CREATE TABLE IF NOT EXISTS).
    fn init(&mut self) -> Result<(), StateError> {
        if self.table_exists("services")? && self.uses_service_only_primary_key()? {
            self.conn.execute_batch(
                "ALTER TABLE services RENAME TO services_old;
                 CREATE TABLE services (
                    project TEXT NOT NULL,
                    service TEXT NOT NULL,
                    pid INTEGER,
                    status TEXT NOT NULL,
                    command_hash TEXT,
                    cwd TEXT,
                    started_at TEXT,
                    updated_at TEXT,
                    restart_count INTEGER DEFAULT 0,
                    last_exit_code INTEGER,
                    last_error TEXT,
                    PRIMARY KEY(project, service)
                 );
                 INSERT OR REPLACE INTO services(project, service, pid, status, command_hash, cwd, started_at, updated_at, restart_count, last_exit_code, last_error)
                 SELECT project, service, pid, status, command_hash, cwd, started_at, updated_at, restart_count, last_exit_code, last_error FROM services_old;
                 DROP TABLE services_old;",
            )?;
            return Ok(());
        }
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS services (
                project TEXT NOT NULL,
                service TEXT NOT NULL,
                pid INTEGER,
                status TEXT NOT NULL,
                command_hash TEXT,
                cwd TEXT,
                started_at TEXT,
                updated_at TEXT,
                restart_count INTEGER DEFAULT 0,
                last_exit_code INTEGER,
                last_error TEXT,
                PRIMARY KEY(project, service)
            )",
            [],
        )?;
        Ok(())
    }

    fn table_exists(&self, table: &str) -> Result<bool, StateError> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![table],
            |row| row.get(0),
        )?;
        Ok(exists > 0)
    }

    fn uses_service_only_primary_key(&self) -> Result<bool, StateError> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(services)")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?;
        let mut project_pk = 0;
        let mut service_pk = 0;
        for row in rows {
            let (name, pk) = row?;
            if name == "project" {
                project_pk = pk;
            } else if name == "service" {
                service_pk = pk;
            }
        }
        Ok(project_pk == 0 && service_pk > 0)
    }

    /// Get the current timestamp as ISO 8601.
    fn now_iso() -> String {
        let now = std::time::SystemTime::now();
        let datetime: chrono::DateTime<chrono::Utc> = now.into();
        datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// Insert or update a service record.
    pub fn upsert(&self, service: &str, values: &ServiceUpdate) -> Result<(), StateError> {
        // Merge with existing row
        let existing = self.get(service).unwrap_or(None);
        let project = values
            .project
            .clone()
            .unwrap_or_else(|| self.project.clone());
        let pid = values.pid.or_else(|| existing.as_ref().and_then(|r| r.pid));
        let status = values
            .status
            .clone()
            .or_else(|| existing.as_ref().map(|r| r.status.clone()))
            .unwrap_or_else(|| "unknown".to_string());
        let command_hash = values
            .command_hash
            .clone()
            .or_else(|| existing.as_ref().and_then(|r| r.command_hash.clone()));
        let cwd = values
            .cwd
            .clone()
            .or_else(|| existing.as_ref().and_then(|r| r.cwd.clone()));
        let started_at = values
            .started_at
            .clone()
            .or_else(|| existing.as_ref().and_then(|r| r.started_at.clone()));
        let restart_count = values
            .restart_count
            .or_else(|| existing.as_ref().map(|r| r.restart_count))
            .unwrap_or(0);
        let last_exit_code = values
            .last_exit_code
            .or_else(|| existing.as_ref().and_then(|r| r.last_exit_code));
        let last_error = values
            .last_error
            .clone()
            .or_else(|| {
                if status == "running" || status == "healthy" || status == "starting" {
                    None
                } else {
                    existing.as_ref().and_then(|r| r.last_error.clone())
                }
            })
            .unwrap_or_default();

        self.conn.execute(
            "INSERT INTO services(project, service, pid, status, command_hash, cwd, started_at, updated_at, restart_count, last_exit_code, last_error)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(project, service) DO UPDATE SET
                project=excluded.project,
                pid=excluded.pid,
                status=excluded.status,
                command_hash=excluded.command_hash,
                cwd=excluded.cwd,
                started_at=excluded.started_at,
                updated_at=excluded.updated_at,
                restart_count=excluded.restart_count,
                last_exit_code=excluded.last_exit_code,
                last_error=excluded.last_error",
            params![
                project,
                service,
                pid,
                status,
                command_hash,
                cwd,
                started_at,
                Self::now_iso(),
                restart_count,
                last_exit_code,
                last_error,
            ],
        )?;
        Ok(())
    }

    /// Get a single service record.
    pub fn get(&self, service: &str) -> Result<Option<ServiceStatus>, StateError> {
        let mut stmt = self.conn.prepare(
            "SELECT project, service, pid, status, command_hash, cwd, started_at, updated_at, restart_count, last_exit_code, last_error FROM services WHERE project=?1 AND service=?2",
        )?;
        let mut rows = stmt.query(params![&self.project, service])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ServiceStatus {
                project: row.get(0)?,
                service: row.get(1)?,
                pid: row.get(2)?,
                status: row.get(3)?,
                command_hash: row.get(4)?,
                cwd: row.get(5)?,
                started_at: row.get(6)?,
                updated_at: row.get(7)?,
                restart_count: row.get(8)?,
                last_exit_code: row.get(9)?,
                last_error: row.get(10)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get all service records.
    pub fn all(&self) -> Result<Vec<ServiceStatus>, StateError> {
        let mut stmt = self.conn.prepare(
            "SELECT project, service, pid, status, command_hash, cwd, started_at, updated_at, restart_count, last_exit_code, last_error FROM services WHERE project=?1 ORDER BY service",
        )?;
        let rows = stmt.query_map(params![&self.project], |row| {
            Ok(ServiceStatus {
                project: row.get(0)?,
                service: row.get(1)?,
                pid: row.get(2)?,
                status: row.get(3)?,
                command_hash: row.get(4)?,
                cwd: row.get(5)?,
                started_at: row.get(6)?,
                updated_at: row.get(7)?,
                restart_count: row.get(8)?,
                last_exit_code: row.get(9)?,
                last_error: row.get(10)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Clear the PID for a service (mark as stopped).
    pub fn clear_pid(
        &self,
        service: &str,
        status: &str,
        exit_code: Option<i32>,
        error: &str,
    ) -> Result<(), StateError> {
        // Directly set pid=NULL and update status
        self.conn.execute(
            "UPDATE services SET pid=NULL, status=?1, updated_at=?2, last_exit_code=?3, last_error=?4 WHERE project=?5 AND service=?6",
            params![status, Self::now_iso(), exit_code, error, &self.project, service],
        )?;
        Ok(())
    }
}

/// Partial update for a service record.
#[derive(Debug, Default)]
pub struct ServiceUpdate {
    pub project: Option<String>,
    pub pid: Option<u32>,
    pub status: Option<String>,
    pub command_hash: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub restart_count: Option<u32>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pystack_state_test_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_open_and_init() {
        let dir = temp_dir("init");
        let db = StateDb::open("test-project", &dir);
        assert!(db.is_ok());
        let db = db.unwrap();
        // Verify table exists by querying
        let all = db.all().unwrap();
        assert!(all.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_upsert_and_get() {
        let dir = temp_dir("upsert_get");
        let db = StateDb::open("test-project", &dir).unwrap();

        db.upsert(
            "web",
            &ServiceUpdate {
                pid: Some(1234),
                status: Some("running".to_string()),
                cwd: Some("/app".to_string()),
                started_at: Some("2024-01-01T00:00:00Z".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let row = db.get("web").unwrap().unwrap();
        assert_eq!(row.service, "web");
        assert_eq!(row.pid, Some(1234));
        assert_eq!(row.status, "running");
        assert_eq!(row.cwd, Some("/app".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_upsert_updates_existing() {
        let dir = temp_dir("upsert_update");
        let db = StateDb::open("test-project", &dir).unwrap();

        db.upsert(
            "web",
            &ServiceUpdate {
                pid: Some(100),
                status: Some("starting".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        db.upsert(
            "web",
            &ServiceUpdate {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let row = db.get("web").unwrap().unwrap();
        assert_eq!(row.pid, Some(100)); // preserved from first upsert
        assert_eq!(row.status, "running"); // updated

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_clear_pid() {
        let dir = temp_dir("clear_pid");
        let db = StateDb::open("test-project", &dir).unwrap();

        db.upsert(
            "web",
            &ServiceUpdate {
                pid: Some(1234),
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        db.clear_pid("web", "stopped", Some(0), "").unwrap();

        let row = db.get("web").unwrap().unwrap();
        assert_eq!(row.pid, None);
        assert_eq!(row.status, "stopped");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_all_services() {
        let dir = temp_dir("all_services");
        let db = StateDb::open("test-project", &dir).unwrap();

        db.upsert(
            "web",
            &ServiceUpdate {
                status: Some("running".into()),
                ..Default::default()
            },
        )
        .unwrap();
        db.upsert(
            "db",
            &ServiceUpdate {
                status: Some("running".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let all = db.all().unwrap();
        assert_eq!(all.len(), 2);
        // Should be sorted by service name
        assert_eq!(all[0].service, "db");
        assert_eq!(all[1].service, "web");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_get_nonexistent() {
        let dir = temp_dir("get_nonexist");
        let db = StateDb::open("test-project", &dir).unwrap();
        assert!(db.get("nonexistent").unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_same_service_name_is_isolated_by_project() {
        let dir = temp_dir("project_isolation");
        let db_a = StateDb::open("project-a", &dir).unwrap();
        let db_b = StateDb::open("project-b", &dir).unwrap();

        db_a.upsert(
            "web",
            &ServiceUpdate {
                pid: Some(111),
                status: Some("running".into()),
                ..Default::default()
            },
        )
        .unwrap();
        db_b.upsert(
            "web",
            &ServiceUpdate {
                pid: Some(222),
                status: Some("running".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(db_a.get("web").unwrap().unwrap().pid, Some(111));
        assert_eq!(db_b.get("web").unwrap().unwrap().pid, Some(222));
        assert_eq!(db_a.all().unwrap().len(), 1);
        assert_eq!(db_b.all().unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }
}
