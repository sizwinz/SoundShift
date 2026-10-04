use rusqlite::{Connection, OptionalExtension, Result};
use std::collections::HashSet;
use std::path::Path;

pub const CURRENT_SCHEMA_VERSION: i64 = 1;

fn ensure_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table_name))?;
    let columns: HashSet<String> = stmt
        .query_map([], |row| row.get(1))?
        .filter_map(Result::ok)
        .collect();

    if !columns.contains(column_name) {
        conn.execute(
            &format!(
                "ALTER TABLE {} ADD COLUMN {} {}",
                table_name, column_name, definition
            ),
            [],
        )?;
    }

    Ok(())
}

fn ensure_indexes(conn: &Connection) -> Result<()> {
    ensure_column(
        conn,
        "transfer_jobs",
        "created_at",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "updated_at",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "target_playlist_name",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "job_state",
        "TEXT NOT NULL DEFAULT 'planned'",
    )?;
    ensure_column(conn, "transfer_jobs", "manifest_hash", "TEXT")?;
    ensure_column(conn, "transfer_jobs", "source_fingerprint", "TEXT")?;
    ensure_column(conn, "transfer_jobs", "target_fingerprint", "TEXT")?;
    ensure_column(
        conn,
        "transfer_jobs",
        "recovery_required",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(conn, "transfer_jobs", "last_error", "TEXT")?;
    ensure_column(
        conn,
        "transfer_snapshots",
        "payload_hash",
        "TEXT NOT NULL DEFAULT ''",
    )?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_transfer_jobs_state ON transfer_jobs(job_state, recovery_required);
        CREATE INDEX IF NOT EXISTS idx_transfer_operations_job ON transfer_operations(job_id, status, operation_index);
        CREATE INDEX IF NOT EXISTS idx_transfer_events_job_seq ON transfer_events(job_id, event_seq, created_at);
        CREATE INDEX IF NOT EXISTS idx_transfer_snapshots_job ON transfer_snapshots(job_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_transfer_quarantine_job ON transfer_quarantine(job_id, created_at);",
    )?;

    Ok(())
}

pub fn init_db(app_dir: &Path) -> Result<Connection> {
    std::fs::create_dir_all(app_dir).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let db_path = app_dir.join("soundshift.db");
    let conn = Connection::open(db_path)?;

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    create_tables(&conn)?;
    run_migrations(&conn)?;

    Ok(conn)
}

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        [],
    )?;

    let persisted: Option<i64> = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM schema_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    let version = persisted.unwrap_or(CURRENT_SCHEMA_VERSION);
    if version != CURRENT_SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidQuery);
    }

    ensure_column(
        conn,
        "transfer_jobs",
        "created_at",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "updated_at",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "target_playlist_name",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "transfer_jobs",
        "job_state",
        "TEXT NOT NULL DEFAULT 'planned'",
    )?;
    ensure_column(conn, "transfer_jobs", "manifest_hash", "TEXT")?;
    ensure_column(conn, "transfer_jobs", "source_fingerprint", "TEXT")?;
    ensure_column(conn, "transfer_jobs", "target_fingerprint", "TEXT")?;
    ensure_column(
        conn,
        "transfer_jobs",
        "recovery_required",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(conn, "transfer_jobs", "last_error", "TEXT")?;

    ensure_column(
        conn,
        "transfer_snapshots",
        "payload_hash",
        "TEXT NOT NULL DEFAULT ''",
    )?;

    ensure_indexes(conn)?;

    conn.execute(
        "INSERT INTO schema_metadata (key, value, updated_at) VALUES ('schema_version', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![
            CURRENT_SCHEMA_VERSION.to_string(),
            chrono::Utc::now().timestamp()
        ],
    )?;

    Ok(())
}

pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS service_sessions (
            service_id TEXT PRIMARY KEY,
            auth_data TEXT NOT NULL,
            expires_at INTEGER,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS track_match_cache (
            source_service TEXT NOT NULL,
            source_track_id TEXT NOT NULL,
            target_service TEXT NOT NULL,
            target_track_id TEXT NOT NULL,
            match_method TEXT NOT NULL,
            confidence REAL NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (source_service, source_track_id, target_service)
        );

        CREATE TABLE IF NOT EXISTS transfer_jobs (
            job_id TEXT PRIMARY KEY,
            source_service TEXT NOT NULL,
            target_service TEXT NOT NULL,
            source_playlist_name TEXT NOT NULL,
            target_playlist_id TEXT NOT NULL,
            target_playlist_name TEXT NOT NULL,
            total_tracks INTEGER NOT NULL,
            matched_tracks INTEGER NOT NULL,
            job_state TEXT NOT NULL DEFAULT 'planned',
            manifest_hash TEXT,
            source_fingerprint TEXT,
            target_fingerprint TEXT,
            recovery_required INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_error TEXT
        );

        CREATE TABLE IF NOT EXISTS transfer_operations (
            operation_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            operation_type TEXT NOT NULL,
            operation_index INTEGER NOT NULL,
            payload_hash TEXT NOT NULL,
            certainty TEXT NOT NULL DEFAULT 'ambiguous',
            status TEXT NOT NULL DEFAULT 'pending',
            attempt_count INTEGER NOT NULL DEFAULT 0,
            provider_response TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(job_id) REFERENCES transfer_jobs(job_id)
        );

        CREATE TABLE IF NOT EXISTS transfer_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            event_payload TEXT NOT NULL,
            event_seq INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(job_id) REFERENCES transfer_jobs(job_id)
        );

        CREATE TABLE IF NOT EXISTS transfer_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            mutation_payload TEXT NOT NULL,
            payload_hash TEXT NOT NULL,
            is_rolled_back INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(job_id) REFERENCES transfer_jobs(job_id)
        );

        CREATE TABLE IF NOT EXISTS transfer_backups (
            backup_id TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL,
            database_path TEXT NOT NULL,
            wal_path TEXT,
            checksum TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'ready'
        );

        CREATE TABLE IF NOT EXISTS transfer_quarantine (
            quarantine_id TEXT PRIMARY KEY,
            job_id TEXT,
            reason TEXT NOT NULL,
            backup_id TEXT,
            diagnostics TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(backup_id) REFERENCES transfer_backups(backup_id)
        );

        CREATE TABLE IF NOT EXISTS cached_playlists (
            id TEXT PRIMARY KEY,
            service TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            track_count INTEGER NOT NULL,
            is_public INTEGER NOT NULL,
            cover_url TEXT,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cached_tracks (
            playlist_id TEXT NOT NULL,
            track_id TEXT NOT NULL,
            position INTEGER NOT NULL,
            track_data TEXT NOT NULL,
            PRIMARY KEY(playlist_id, track_id, position)
        );

        CREATE TABLE IF NOT EXISTS transfer_manifest_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    )?;

    ensure_indexes(conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_tables_in_memory() {
        let conn = Connection::open_in_memory().expect("failed to open memory db");
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        create_tables(&conn).expect("failed to create tables");
        run_migrations(&conn).expect("failed to initialize schema metadata");

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .expect("prepare failed");
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("query failed")
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"service_sessions".to_string()));
        assert!(tables.contains(&"track_match_cache".to_string()));
        assert!(tables.contains(&"transfer_jobs".to_string()));
        assert!(tables.contains(&"transfer_operations".to_string()));
        assert!(tables.contains(&"transfer_events".to_string()));
        assert!(tables.contains(&"transfer_snapshots".to_string()));
        assert!(tables.contains(&"transfer_backups".to_string()));
        assert!(tables.contains(&"transfer_quarantine".to_string()));
        assert!(tables.contains(&"cached_playlists".to_string()));
        assert!(tables.contains(&"cached_tracks".to_string()));

        let version: i64 = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM schema_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version missing");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_legacy_transfer_jobs_with_missing_target_playlist_name() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        create_tables(&conn).expect("create supporting tables");
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_transfer_jobs_state;
             PRAGMA foreign_keys = OFF;
             DROP TABLE transfer_jobs;
             CREATE TABLE transfer_jobs (
                job_id TEXT PRIMARY KEY,
                source_service TEXT NOT NULL,
                target_service TEXT NOT NULL,
                source_playlist_name TEXT NOT NULL,
                target_playlist_id TEXT NOT NULL,
                total_tracks INTEGER NOT NULL,
                matched_tracks INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            PRAGMA foreign_keys = ON;",
        )
        .expect("create legacy transfer_jobs table");

        run_migrations(&conn).expect("migrate legacy transfer_jobs table");

        let column: String = conn
            .query_row(
                "SELECT name FROM pragma_table_info('transfer_jobs')
                 WHERE name = 'target_playlist_name'",
                [],
                |row| row.get(0),
            )
            .expect("target_playlist_name should be added");
        assert_eq!(column, "target_playlist_name");

        let updated_at_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('transfer_jobs')
                 WHERE name = 'updated_at'",
                [],
                |row| row.get(0),
            )
            .expect("updated_at should be queryable");
        assert_eq!(updated_at_exists, 1);
    }
}
