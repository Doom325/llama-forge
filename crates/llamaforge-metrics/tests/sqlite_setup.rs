//! Tests for schema migrations and advisory locking.

use std::fs::File;
use std::os::fd::AsRawFd;

#[allow(deprecated)]
use nix::fcntl::{flock, FlockArg};
use tempfile::TempDir;

use llamaforge_metrics::{
    create_read_pool, get_migrations, open_and_migrate, MetricsConfig, StoreError,
};

#[tokio::test]
async fn test_schema_migration_idempotent() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // 1. Run migrations once
    let mut conn = open_and_migrate(&db_path).expect("first migration failed");

    // Verify WAL mode
    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    // Verify tables exist
    let tables = [
        "audit_events",
        "request_metrics",
        "managed_processes",
        "config_snapshots",
        "config_overlays",
    ];
    for table in tables {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "table {} missing", table);
    }

    // 2. Run migrations again (idempotent)
    get_migrations().to_latest(&mut conn).expect("second migration failed");
}

#[tokio::test]
async fn test_advisory_lock() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // 1. Acquire lock manually to simulate another instance
    let mut lock_path = db_path.to_path_buf().into_os_string();
    lock_path.push(".lock");
    let lock_path = std::path::PathBuf::from(lock_path);
    let lock_file = File::create(&lock_path).unwrap();
    #[allow(deprecated)]
    flock(lock_file.as_raw_fd(), FlockArg::LockExclusiveNonblock).unwrap();

    // 2. Try to open DB
    let result = open_and_migrate(&db_path);
    assert!(matches!(result, Err(StoreError::Unavailable { message: _ })));

    // 3. Drop lock
    #[allow(deprecated)]
    flock(lock_file.as_raw_fd(), FlockArg::UnlockNonblock).unwrap();
    drop(lock_file);

    // 4. Try again -> success
    let _conn = open_and_migrate(&db_path).expect("failed after lock drop");
}

#[tokio::test]
async fn test_read_pool_query_only() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // Must migrate first to set WAL mode
    let _conn = open_and_migrate(&db_path).unwrap();

    let config =
        MetricsConfig { db_path: db_path.clone(), read_pool_size: 2, ..Default::default() };

    let pool = create_read_pool(&config).expect("failed to create pool");
    let conn = pool.get().await.unwrap();

    conn.interact(|c| {
        // query_only should prevent writes
        let result = c.execute(
            "INSERT INTO managed_processes (pid, server_id, started_at, cmdline) VALUES (1, 's1', 'now', 'cmd')",
            [],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("readonly"), "unexpected error: {}", msg);
    })
    .await
    .unwrap();
}
