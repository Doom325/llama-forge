//! Specific test cases outlined in design doc Section 8.4.

use std::time::Duration;

use llamaforge_metrics::{create_read_pool, MetricsConfig};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pre_recycle_rollback() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // Initialize DB with WAL mode
    let (_conn, _lock_file) = llamaforge_metrics::open_and_migrate(&db_path).unwrap();

    let config = MetricsConfig {
        db_path,
        read_pool_size: 1, // pool size 1 ensures we get the same connection back
        ..Default::default()
    };

    let pool = create_read_pool(&config).unwrap();

    // 1. Get connection and start a transaction
    let conn1 = pool.get().await.unwrap();
    conn1
        .interact(|c| {
            c.execute("BEGIN TRANSACTION", []).map_err(|e| std::io::Error::other(e.to_string()))
        })
        .await
        .unwrap()
        .unwrap();

    // Verify it is in a transaction (autocommit is false)
    let is_autocommit = conn1.interact(|c| c.is_autocommit()).await.unwrap();
    assert!(!is_autocommit);

    // Drop conn1 to return it to the pool. The pre_recycle hook should run ROLLBACK.
    drop(conn1);

    // 2. Get connection again (it's the same one since pool size is 1)
    let conn2 = pool.get().await.unwrap();

    // Verify it is NOT in a transaction anymore (autocommit is true)
    let is_autocommit2 = conn2.interact(|c| c.is_autocommit()).await.unwrap();
    assert!(is_autocommit2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_read_pool_statement_timeout() {
    // Tests that `tokio::time::timeout` correctly drops the `interact` future if it takes too long.
    // We simulate this by having the interact closure sleep longer than the timeout.

    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");
    let (_conn, _lock_file) = llamaforge_metrics::open_and_migrate(&db_path).unwrap();

    let config = MetricsConfig { db_path, ..Default::default() };

    let pool = create_read_pool(&config).unwrap();
    let conn = pool.get().await.unwrap();

    let timeout_duration = Duration::from_millis(50);

    let future = conn.interact(|_c| {
        std::thread::sleep(Duration::from_millis(200));
        Ok::<(), ()>(())
    });

    let res = tokio::time::timeout(timeout_duration, future).await;
    assert!(res.is_err(), "Expected timeout error!");
}
