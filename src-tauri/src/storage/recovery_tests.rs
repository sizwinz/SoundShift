use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("soundshift_recovery_{}", rand::random::<u64>()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn db(&self) -> PathBuf {
        self.0.join("soundshift.db")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn insert_job(conn: &Connection, id: &str) {
    conn.execute("INSERT INTO transfer_jobs (job_id, source_service, target_service, source_playlist_name,
        target_playlist_id, target_playlist_name, total_tracks, matched_tracks, job_state, created_at, updated_at)
        VALUES (?1, 'spotify', 'ytmusic', 'Source', 'target', 'Destination', 1, 1, 'completed_verified', 1, 1)", [id]).unwrap();
}

fn new_database(dir: &TestDirectory) -> Connection {
    let conn = Connection::open(dir.db()).unwrap();
    create_tables(&conn).unwrap();
    run_migrations(&conn).unwrap();
    conn
}

#[test]
fn corrupt_database_without_backup_stays_blocked_on_repeated_startup() {
    let dir = TestDirectory::new();
    let original = b"not a SQLite database";
    fs::write(dir.db(), original).unwrap();
    assert!(init_db(dir.path())
        .unwrap_err()
        .contains("No checksum-verified"));
    assert!(!dir.db().exists());
    let marker = read_marker(dir.path()).unwrap();
    let quarantine = dir.path().join("quarantine").join(marker.quarantine_id);
    assert_eq!(
        fs::read(quarantine.join("soundshift.db")).unwrap(),
        original
    );
    assert!(quarantine.join("diagnostics.json").is_file());
    assert!(init_db(dir.path()).is_err());
    assert!(
        !dir.db().exists(),
        "repeated startup must not create a fresh database"
    );
}

#[test]
fn missing_database_with_old_recovery_files_is_not_a_new_install() {
    let dir = TestDirectory::new();
    fs::create_dir(dir.path().join("quarantine")).unwrap();
    assert!(init_db(dir.path())
        .unwrap_err()
        .contains("No empty database"));
    assert!(!dir.db().exists());
}

#[test]
fn quarantine_preserves_database_wal_shm_and_reason() {
    let dir = TestDirectory::new();
    for (name, bytes) in [
        ("soundshift.db", "db"),
        ("soundshift.db-wal", "wal"),
        ("soundshift.db-shm", "shm"),
    ] {
        fs::write(dir.path().join(name), bytes).unwrap();
    }
    let path = quarantine_corrupted_database(dir.path(), "test diagnosis").unwrap();
    let quarantine = Path::new(&path).parent().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "db");
    assert_eq!(
        fs::read_to_string(quarantine.join("soundshift.db-wal")).unwrap(),
        "wal"
    );
    assert_eq!(
        fs::read_to_string(quarantine.join("soundshift.db-shm")).unwrap(),
        "shm"
    );
    assert!(fs::read_to_string(quarantine.join("diagnostics.json"))
        .unwrap()
        .contains("test diagnosis"));
}

#[test]
fn quarantine_diagnostic_files_do_not_persist_secret_values() {
    let dir = TestDirectory::new();
    fs::write(dir.db(), "corrupt").unwrap();
    let path = quarantine_corrupted_database(
        dir.path(),
        "Cookie: sp_dc=diagnostic_secret\nAuthorization: Bearer secret_bearer",
    )
    .unwrap();
    for path in [
        dir.path().join(RECOVERY_MARKER),
        Path::new(&path).parent().unwrap().join("diagnostics.json"),
    ] {
        let json = fs::read_to_string(path).unwrap();
        assert!(!json.contains("diagnostic_secret"));
        assert!(!json.contains("secret_bearer"));
        assert!(json.contains("REDACTED"));
    }
}

#[test]
fn interrupted_quarantine_cannot_trigger_restore_or_empty_database() {
    let dir = TestDirectory::new();
    fs::write(dir.db(), "corrupt").unwrap();
    // A filesystem failure partway through moving the related files.
    fs::create_dir(dir.path().join("soundshift.db-wal")).unwrap();
    assert!(quarantine_corrupted_database(dir.path(), "injected failure").is_err());
    assert!(init_db(dir.path())
        .unwrap_err()
        .contains("Quarantine was interrupted"));
    assert!(!dir.db().exists());
    let marker = read_marker(dir.path()).unwrap();
    assert!(dir
        .path()
        .join("quarantine")
        .join(marker.quarantine_id)
        .join("soundshift.db")
        .exists());
}

#[test]
fn locked_database_is_not_quarantined() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
    assert!(init_db(dir.path()).is_err());
    assert!(!dir.path().join(RECOVERY_MARKER).exists());
    assert!(!dir.path().join("quarantine").exists());
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(integrity_ok(&conn).unwrap());
}

#[test]
fn backup_contains_committed_wal_rows_and_independent_checksum() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    insert_job(&conn, "committed_in_wal");
    assert!(dir.path().join("soundshift.db-wal").exists());
    let record = create_database_backup(&conn, dir.path()).unwrap();
    let path = Path::new(&record.database_path);
    assert!(path.with_extension("json").exists());
    assert!(verify_backup_integrity(path, &record.checksum).unwrap());
    let copy = open_existing(path, true).unwrap();
    let id: String = copy
        .query_row("SELECT job_id FROM transfer_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(id, "committed_in_wal");
}

#[test]
fn backup_creation_rejects_unsupported_schema_before_publication() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    conn.execute(
        "UPDATE schema_metadata SET value = '999' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    assert!(create_database_backup(&conn, dir.path()).is_err());
    assert!(verified_candidates(dir.path()).unwrap().is_empty());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_backups", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn altered_but_structurally_valid_backup_is_not_restored() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    insert_job(&conn, "original");
    let record = create_database_backup(&conn, dir.path()).unwrap();
    drop(conn);
    let backup = Connection::open(&record.database_path).unwrap();
    backup
        .execute(
            "UPDATE transfer_jobs SET source_playlist_name = 'altered'",
            [],
        )
        .unwrap();
    assert!(integrity_ok(&backup).unwrap());
    drop(backup);
    fs::remove_file(dir.db()).unwrap();
    assert!(restore_latest_backup(dir.path()).is_err());
    assert!(!dir.db().exists());
}

#[test]
fn backup_without_checksum_manifest_is_preserved_but_not_trusted() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    let record = create_database_backup(&conn, dir.path()).unwrap();
    drop(conn);
    fs::remove_file(Path::new(&record.database_path).with_extension("json")).unwrap();
    fs::remove_file(dir.db()).unwrap();
    assert!(restore_latest_backup(dir.path()).is_err());
    assert!(Path::new(&record.database_path).exists());
    assert!(!dir.db().exists());
}

#[test]
fn restore_refuses_to_overwrite_existing_database() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    create_database_backup(&conn, dir.path()).unwrap();
    drop(conn);
    let before = checksum(&dir.db()).unwrap();
    assert!(restore_latest_backup(dir.path())
        .unwrap_err()
        .contains("refused"));
    assert_eq!(checksum(&dir.db()).unwrap(), before);
}

#[test]
fn startup_restores_verified_backup_but_requires_review_even_for_completed_jobs() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    insert_job(&conn, "completed_before_backup");
    create_database_backup(&conn, dir.path()).unwrap();
    drop(conn);
    fs::write(dir.db(), "corrupt").unwrap();
    let recovered = init_db(dir.path()).unwrap();
    assert!(ensure_mutations_allowed(&recovered).is_err());
    assert!(acknowledge_recovery(&recovered, false).is_err());
    assert!(ensure_mutations_allowed(&recovered).is_err());
    let state: String = recovered
        .query_row("SELECT job_state FROM transfer_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(state, "recovery_required");
    assert!(!dir.path().join(RECOVERY_MARKER).exists());
    drop(recovered);
    let reopened = init_db(dir.path()).unwrap();
    assert!(
        ensure_mutations_allowed(&reopened).is_err(),
        "review gate survives restart"
    );
    acknowledge_recovery(&reopened, true).unwrap();
    assert!(ensure_mutations_allowed(&reopened).is_ok());
    let state: String = reopened
        .query_row("SELECT job_state FROM transfer_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        state, "recovery_required",
        "acknowledging review must not reconcile old jobs"
    );
}

#[test]
fn older_verified_backup_is_used_when_newest_was_altered() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    insert_job(&conn, "older_job");
    let old = create_database_backup(&conn, dir.path()).unwrap();
    insert_job(&conn, "newer_job");
    let new = create_database_backup(&conn, dir.path()).unwrap();
    for (record, created_at) in [(&old, 1), (&new, 2)] {
        let path = Path::new(&record.database_path).with_extension("json");
        let mut manifest: BackupManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.created_at = created_at;
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    let tampered = Connection::open(&new.database_path).unwrap();
    tampered
        .execute(
            "UPDATE transfer_jobs SET source_playlist_name = 'changed'",
            [],
        )
        .unwrap();
    drop(tampered);
    drop(conn);
    fs::remove_file(dir.db()).unwrap();
    assert_eq!(
        restore_latest_backup(dir.path()).unwrap(),
        old.database_path
    );
    let recovered = open_existing(&dir.db(), false).unwrap();
    let count: i64 = recovered
        .query_row("SELECT COUNT(*) FROM transfer_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(ensure_mutations_allowed(&recovered).is_err());
}

#[test]
fn healthy_database_can_upgrade_old_backups_using_recorded_expected_checksums() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    let backup = create_database_backup(&conn, dir.path()).unwrap();
    let manifest = Path::new(&backup.database_path).with_extension("json");
    fs::remove_file(&manifest).unwrap();
    drop(conn);
    let _reopened = init_db(dir.path()).unwrap();
    assert!(manifest.is_file());
}

#[test]
fn unsupported_live_schema_is_preserved_without_quarantine_or_migration() {
    let dir = TestDirectory::new();
    let conn = new_database(&dir);
    conn.execute("UPDATE schema_metadata SET value = 'future'", [])
        .unwrap();
    drop(conn);
    let before = checksum(&dir.db()).unwrap();
    assert!(init_db(dir.path()).unwrap_err().contains("Unsupported"));
    assert_eq!(checksum(&dir.db()).unwrap(), before);
    assert!(!dir.path().join("quarantine").exists());
}
