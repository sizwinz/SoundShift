use rusqlite::{Connection, Result};
use std::path::Path;

pub fn init_db(app_dir: &Path) -> Result<Connection> {
    std::fs::create_dir_all(app_dir).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;

    let db_path = app_dir.join("soundshift.db");
    let conn = Connection::open(db_path)?;

    // WAL mode for concurrent, low-latency client operations
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    create_tables(&conn)?;

    Ok(conn)
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
            total_tracks INTEGER NOT NULL,
            matched_tracks INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS transfer_snapshots (
            snapshot_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            mutation_payload TEXT NOT NULL,
            is_rolled_back INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY(job_id) REFERENCES transfer_jobs(job_id)
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
        );"
    )?;

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
        assert!(tables.contains(&"transfer_snapshots".to_string()));
        assert!(tables.contains(&"cached_playlists".to_string()));
        assert!(tables.contains(&"cached_tracks".to_string()));
    }
}
