//! SQLite backend infrastructure

pub(crate) mod batch_collector;
pub(crate) mod commands;
pub(crate) mod in_memory;
pub(crate) mod liveness;
pub(crate) mod migrations;
pub(crate) mod pipeline;
pub(crate) mod pool;
pub(crate) mod prune;
pub(crate) mod query_builder;
pub(crate) mod rows;
pub(crate) mod store;
pub(crate) mod writer;

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

#[allow(deprecated)]
use nix::fcntl::{flock, FlockArg};
use rusqlite::Connection;

use crate::error::StoreError;

/// Open a SQLite connection, run WAL pragmas, run migrations, and acquire an advisory lock.
pub fn open_and_migrate(db_path: &Path) -> Result<(Connection, File), StoreError> {
    // 1. Acquire advisory lock on {db_path}.lock
    let mut lock_path = db_path.to_path_buf().into_os_string();
    lock_path.push(".lock");
    let lock_path = std::path::PathBuf::from(lock_path);
    let lock_file =
        File::create(&lock_path).map_err(|e| StoreError::Internal { source: Box::new(e) })?;

    // Attempt exclusive, non-blocking lock.
    #[allow(deprecated)]
    flock(lock_file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|_| {
        StoreError::Unavailable { message: "Database is locked by another instance".into() }
    })?;

    // 2. Open connection
    let mut conn =
        Connection::open(db_path).map_err(|e| StoreError::Internal { source: Box::new(e) })?;

    // 3. Set PRAGMAs for writer connection
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        PRAGMA foreign_keys = ON;
        "#,
    )
    .map_err(|e| StoreError::Internal { source: Box::new(e) })?;
    // 4. Run schema migrations
    let migrations = migrations::get_migrations();
    migrations.to_latest(&mut conn).map_err(|e| StoreError::Internal { source: Box::new(e) })?;

    Ok((conn, lock_file))
}
