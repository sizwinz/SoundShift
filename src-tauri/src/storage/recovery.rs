//! Filesystem recovery must never turn an unreadable database into a new install.
use super::schema::{create_tables, run_migrations, CURRENT_SCHEMA_VERSION};
use crate::engine::snapshot::{set_job_state, BackupRecord};
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const RECOVERY_MARKER: &str = "recovery-required.json";
const RESTORE_NOTICE: &str = "Database restored from an older backup. Later jobs may be missing. Review provider state before any further transfer or rollback.";

#[derive(Serialize, Deserialize)]
struct BackupManifest {
    version: u32,
    backup_id: String,
    created_at: i64,
    checksum: String,
}

#[derive(Serialize, Deserialize)]
struct RecoveryMarker {
    version: u32,
    quarantine_id: String,
    reason: String,
}

fn open_existing(path: &Path, readonly: bool) -> Result<Connection, String> {
    let flags = if readonly {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| format!("Cannot open existing database: {e}"))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Only SQLite's explicit corruption outcomes justify quarantine. Busy, locked,
/// permission, and I/O failures must leave the existing files alone.
fn integrity_ok(conn: &Connection) -> Result<bool, String> {
    match conn.query_row::<String, _, _>("PRAGMA integrity_check", [], |row| row.get(0)) {
        Ok(result) => Ok(result == "ok"),
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(format!(
            "Database integrity check could not run; files preserved: {error}"
        )),
    }
}

fn validate_schema_version(conn: &Connection) -> Result<(), String> {
    let has_metadata: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_metadata')",
        [], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if has_metadata {
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM schema_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if version
            .as_deref()
            .is_some_and(|value| value != CURRENT_SCHEMA_VERSION.to_string())
        {
            return Err(
                "Unsupported database schema version; database preserved without migration".into(),
            );
        }
    }
    Ok(())
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    sync_directory(path.parent().ok_or("Missing parent directory")?)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn checksum(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn valid_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() <= 100
        && id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
}

fn read_marker(app_dir: &Path) -> Result<RecoveryMarker, String> {
    let marker: RecoveryMarker = serde_json::from_slice(
        &fs::read(app_dir.join(RECOVERY_MARKER)).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "Recovery marker is unreadable. Preserve files and review recovery manually.")?;
    if marker.version != 1 || !valid_id(&marker.quarantine_id, "quarantine_") {
        return Err("Invalid recovery marker; manual recovery required".into());
    }
    Ok(marker)
}

pub fn init_db(app_dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
    let db_path = app_dir.join("soundshift.db");
    let marker_path = app_dir.join(RECOVERY_MARKER);
    let mut recovered = marker_path.exists();
    if recovered {
        let marker = read_marker(app_dir)?;
        if !app_dir
            .join("quarantine")
            .join(marker.quarantine_id)
            .join("complete")
            .is_file()
        {
            return Err("Quarantine was interrupted. Original database/WAL files are preserved across their recorded locations; manual recovery required.".into());
        }
        if !db_path.exists() {
            restore_latest_backup(app_dir)?;
        }
    }
    if db_path.exists() {
        let conn = open_existing(&db_path, false)?;
        let healthy = integrity_ok(&conn)?;
        drop(conn);
        if !healthy {
            if recovered {
                return Err(
                    "Restored database failed integrity checks; recovery marker retained".into(),
                );
            }
            quarantine_corrupted_database(
                app_dir,
                "SQLite reported database corruption during startup",
            )?;
            recovered = true;
            restore_latest_backup(app_dir)?;
        }
    } else if app_dir.join("soundshift.db-wal").exists()
        || app_dir.join("soundshift.db-shm").exists()
        || app_dir.join("quarantine").exists()
        || app_dir.join("backups").exists()
    {
        return Err("Database is missing but recovery files exist. No empty database was created; restore a verified backup or recover the preserved files.".into());
    }

    let conn = if db_path.exists() {
        open_existing(&db_path, false)?
    } else {
        Connection::open(&db_path).map_err(|e| e.to_string())?
    };
    if !integrity_ok(&conn)? {
        return Err("Database failed integrity verification; files preserved".into());
    }
    validate_schema_version(&conn)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    create_tables(&conn).map_err(|e| e.to_string())?;
    run_migrations(&conn).map_err(|e| e.to_string())?;
    import_legacy_backup_manifests(&conn, app_dir)?;
    if recovered {
        mark_restored_database(&conn)?;
    }
    crate::engine::snapshot::reconcile_incomplete_jobs(&conn)?;
    create_database_backup_if_needed(&conn, app_dir)?;
    if recovered {
        fs::remove_file(marker_path)
            .map_err(|e| format!("Recovery succeeded but marker cleanup failed: {e}"))?;
    }
    Ok(conn)
}

fn mark_restored_database(conn: &Connection) -> Result<(), String> {
    let ids = {
        let mut stmt = conn
            .prepare("SELECT job_id FROM transfer_jobs")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    for id in ids {
        set_job_state(conn, &id, "recovery_required", Some(RESTORE_NOTICE))?;
    }
    conn.execute(
        "DELETE FROM transfer_manifest_state WHERE key = 'database_restore_reviewed'",
        [],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO transfer_manifest_state (key, value, updated_at) VALUES ('database_restored', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![RESTORE_NOTICE, chrono::Utc::now().timestamp()],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Older backups cannot prove the absence of later confirmed remote mutations.
pub fn ensure_mutations_allowed(conn: &Connection) -> Result<(), String> {
    match recovery_notice(conn)? {
        Some(notice) => Err(notice),
        None => Ok(()),
    }
}

pub fn recovery_notice(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM transfer_manifest_state WHERE key = 'database_restored'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// This permits new user-directed work only. Restored jobs remain recovery_required.
pub fn acknowledge_recovery(conn: &Connection, reviewed: bool) -> Result<(), String> {
    if !reviewed {
        return Err("Confirm that the provider playlists were manually reviewed first".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO transfer_manifest_state (key, value, updated_at)
        SELECT 'database_restore_reviewed', value, ?1 FROM transfer_manifest_state WHERE key = 'database_restored'
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        [chrono::Utc::now().timestamp()]).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM transfer_manifest_state WHERE key = 'database_restored'",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

fn import_legacy_backup_manifests(conn: &Connection, app_dir: &Path) -> Result<(), String> {
    let mut stmt = conn.prepare("SELECT backup_id, created_at, checksum FROM transfer_backups WHERE status = 'verified'").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BackupManifest {
                version: 1,
                backup_id: row.get(0)?,
                created_at: row.get(1)?,
                checksum: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let manifest = row.map_err(|e| e.to_string())?;
        if !valid_id(&manifest.backup_id, "backup_") {
            continue;
        }
        let path = app_dir
            .join("backups")
            .join(format!("{}.db", manifest.backup_id));
        if path.with_extension("json").exists() {
            continue;
        }
        // Expected hashes come from the healthy live database, never the candidate.
        if matches!(verify_backup_integrity(&path, &manifest.checksum), Ok(true)) {
            write_new_synced(
                &path.with_extension("json"),
                &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
            )?;
        }
    }
    Ok(())
}

pub fn create_database_backup(conn: &Connection, app_dir: &Path) -> Result<BackupRecord, String> {
    let backups_dir = app_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;
    sync_directory(app_dir)?;
    let now = chrono::Utc::now().timestamp();
    let backup_id = format!("backup_{}_{}", now, rand::random::<u64>());
    let staging = backups_dir.join(format!("{backup_id}.partial"));
    let path = backups_dir.join(format!("{backup_id}.db"));
    conn.execute("VACUUM INTO ?1", [staging.to_string_lossy().as_ref()])
        .map_err(|e| e.to_string())?;
    OpenOptions::new()
        .write(true)
        .open(&staging)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    let hash = checksum(&staging)?;
    if !verify_backup_integrity(&staging, &hash)? {
        return Err("Backup failed integrity verification; it was not published".into());
    }
    fs::rename(&staging, &path).map_err(|e| e.to_string())?;
    let manifest = BackupManifest {
        version: 1,
        backup_id: backup_id.clone(),
        created_at: now,
        checksum: hash.clone(),
    };
    // A sidecar is deliberately independent of the database it protects.
    write_new_synced(
        &path.with_extension("json"),
        &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
    )?;
    conn.execute(
        "INSERT INTO transfer_backups (backup_id, created_at, database_path, wal_path, checksum, status)
         VALUES (?1, ?2, ?3, NULL, ?4, 'verified')",
        rusqlite::params![backup_id, now, path.to_string_lossy(), hash],
    ).map_err(|e| e.to_string())?;
    prune_old_backups(conn, &backups_dir)?;
    Ok(BackupRecord {
        backup_id,
        created_at: now,
        database_path: path.to_string_lossy().into_owned(),
        wal_path: None,
        checksum: hash,
        status: "verified".into(),
    })
}

pub fn create_database_backup_if_needed(
    conn: &Connection,
    app_dir: &Path,
) -> Result<Option<BackupRecord>, String> {
    let day_ago = chrono::Utc::now().timestamp() - 86400;
    let candidates = verified_candidates(app_dir)?;
    for (path, manifest) in candidates {
        if manifest.created_at > day_ago && verify_backup_integrity(&path, &manifest.checksum)? {
            return Ok(None);
        }
    }
    create_database_backup(conn, app_dir).map(Some)
}

pub fn verify_backup_integrity(path: &Path, expected_checksum: &str) -> Result<bool, String> {
    if !path.is_file()
        || fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Ok(false);
    }
    if !checksum(path)?.eq_ignore_ascii_case(expected_checksum) {
        return Ok(false);
    }
    let conn = open_existing(path, true)?;
    if !integrity_ok(&conn)? {
        return Ok(false);
    }
    validate_schema_version(&conn)?;
    // A structurally valid unrelated SQLite file is not an application backup.
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('transfer_jobs', 'transfer_operations', 'transfer_snapshots', 'schema_metadata')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut fk = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?;
    let has_fk_error = fk.exists([]).map_err(|e| e.to_string())?;
    Ok(tables == 4 && !has_fk_error)
}

fn verified_candidates(app_dir: &Path) -> Result<Vec<(PathBuf, BackupManifest)>, String> {
    let dir = app_dir.join("backups");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let manifest: BackupManifest = match fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(manifest) => manifest,
            None => continue,
        };
        if manifest.version != 1
            || !valid_id(&manifest.backup_id, "backup_")
            || path.file_stem().and_then(|s| s.to_str()) != Some(&manifest.backup_id)
            || manifest.checksum.len() != 64
            || !manifest.checksum.bytes().all(|ch| ch.is_ascii_hexdigit())
        {
            continue;
        }
        candidates.push((path.with_extension("db"), manifest));
    }
    candidates.sort_by(|a, b| {
        b.1.created_at
            .cmp(&a.1.created_at)
            .then_with(|| b.1.backup_id.cmp(&a.1.backup_id))
    });
    Ok(candidates)
}

pub fn quarantine_corrupted_database(app_dir: &Path, reason: &str) -> Result<String, String> {
    if !app_dir.join("soundshift.db").is_file() {
        return Err("No database file exists to quarantine; files preserved".into());
    }
    let id = format!(
        "quarantine_{}_{}",
        chrono::Utc::now().timestamp(),
        rand::random::<u64>()
    );
    let dir = app_dir.join("quarantine").join(&id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    sync_directory(&app_dir.join("quarantine"))?;
    sync_directory(app_dir)?;
    let marker = RecoveryMarker {
        version: 1,
        quarantine_id: id,
        reason: crate::diagnostics::sanitize_for_log(reason),
    };
    let json = serde_json::to_vec(&marker).map_err(|e| e.to_string())?;
    write_new_synced(&app_dir.join(RECOVERY_MARKER), &json)?;
    write_new_synced(&dir.join("diagnostics.json"), &json)?;
    for name in ["soundshift.db", "soundshift.db-wal", "soundshift.db-shm"] {
        let source = app_dir.join(name);
        if source.is_dir() {
            return Err(format!(
                "Quarantine found a directory at {name}; manual recovery required"
            ));
        }
        match fs::rename(&source, dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Quarantine could not preserve {name}: {error}. Recovery remains blocked."
                ))
            }
        }
    }
    sync_directory(app_dir)?;
    sync_directory(&dir)?;
    write_new_synced(
        &dir.join("complete"),
        b"All database files moved successfully",
    )?;
    Ok(dir.join("soundshift.db").to_string_lossy().into_owned())
}

pub fn restore_latest_backup(app_dir: &Path) -> Result<String, String> {
    let target = app_dir.join("soundshift.db");
    if target.exists()
        || app_dir.join("soundshift.db-wal").exists()
        || app_dir.join("soundshift.db-shm").exists()
    {
        return Err("Restore refused to replace an existing database or WAL files; quarantine must complete first".into());
    }
    for (path, manifest) in verified_candidates(app_dir)? {
        // Bad/missing candidates are preserved and skipped; no checksum is inferred
        // from the candidate itself during recovery.
        if !matches!(verify_backup_integrity(&path, &manifest.checksum), Ok(true)) {
            continue;
        }
        let staging = app_dir.join(format!("restore_{}.partial", rand::random::<u64>()));
        let mut input = File::open(&path).map_err(|e| e.to_string())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        if !verify_backup_integrity(&staging, &manifest.checksum)? {
            return Err(
                "Staged restore failed checksum/integrity verification; database was not replaced"
                    .into(),
            );
        }
        // Persist uncertainty inside the staged database before publication,
        // including callers that explicitly restore outside init_db.
        let staged = open_existing(&staging, false)?;
        staged
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|e| e.to_string())?;
        staged
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|e| e.to_string())?;
        mark_restored_database(&staged)?;
        drop(staged);
        OpenOptions::new()
            .write(true)
            .open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        // hard_link provides no-replace publication on supported local filesystems.
        // Unlike rename on Unix, it cannot overwrite a target created meanwhile.
        fs::hard_link(&staging, &target).map_err(|e| {
            format!("Could not publish restored database without replacing existing files: {e}")
        })?;
        sync_directory(app_dir)?;
        fs::remove_file(&staging).map_err(|e| e.to_string())?;
        return Ok(path.to_string_lossy().into_owned());
    }
    Err("No checksum-verified compatible backup is available. Recovery files were preserved; no empty database was created.".into())
}

fn prune_old_backups(conn: &Connection, backups_dir: &Path) -> Result<(), String> {
    let app_dir = backups_dir.parent().ok_or("Invalid backup directory")?;
    let cutoff = chrono::Utc::now().timestamp() - 7 * 86400;
    let mut verified = Vec::new();
    for (path, manifest) in verified_candidates(app_dir)? {
        if matches!(verify_backup_integrity(&path, &manifest.checksum), Ok(true)) {
            verified.push((path, manifest));
        }
    }
    for (path, manifest) in verified.into_iter().skip(3) {
        if manifest.created_at >= cutoff {
            continue;
        }
        // Paths are derived from validated IDs in this directory, never from SQL.
        fs::remove_file(&path).map_err(|e| e.to_string())?;
        fs::remove_file(path.with_extension("json")).map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM transfer_backups WHERE backup_id = ?1",
            [manifest.backup_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
