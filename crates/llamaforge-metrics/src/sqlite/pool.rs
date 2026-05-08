//! Read pool configuration and creation.
//!
//! See design doc Section 3.5.

use deadpool::managed::Hook;
use deadpool_sqlite::{Config as PoolConfig, Pool, Runtime};
use deadpool_sync::SyncWrapper;
use rusqlite::Connection;

use crate::error::StoreError;
use crate::types::MetricsConfig;

/// Create the deadpool-sqlite read pool.
#[allow(dead_code)]
pub fn create_read_pool(config: &MetricsConfig) -> Result<Pool, StoreError> {
    let max_size = config.read_pool_size.clamp(1, 16);

    let pool_config = PoolConfig {
        path: config.db_path.clone(),
        pool: Some(deadpool::managed::PoolConfig::new(max_size)),
    };

    let pool = pool_config
        .builder(Runtime::Tokio1)
        .map_err(|e| StoreError::Internal { source: Box::new(e) })?
        .post_create(Hook::async_fn(|wrapper: &mut SyncWrapper<Connection>, _| {
            Box::pin(async move {
                wrapper
                    .interact(|conn| {
                        // Verify WAL mode was set by the writer connection.
                        let journal_mode: String =
                            conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
                        if journal_mode.to_lowercase() != "wal" {
                            return Err(rusqlite::Error::SqliteFailure(
                                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                                Some("Database is not in WAL mode".to_string()),
                            ));
                        }

                        // Set read-only connection limits
                        conn.execute_batch(
                            r#"
                        PRAGMA query_only = ON;
                        PRAGMA busy_timeout = 5000;
                        PRAGMA synchronous = NORMAL;
                        "#,
                        )?;
                        Ok(())
                    })
                    .await
                    .map_err(|_| deadpool::managed::HookError::Message("Interact error".into()))?
                    .map_err(|e: rusqlite::Error| {
                        deadpool::managed::HookError::Message(e.to_string().into())
                    })
            })
        }))
        .pre_recycle(Hook::async_fn(|wrapper: &mut SyncWrapper<Connection>, _| {
            Box::pin(async move {
                wrapper
                    .interact(|conn| {
                        // Rollback any dangling transaction
                        let _ = conn.execute("ROLLBACK", []);

                        // Re-verify WAL mode
                        let journal_mode: String =
                            conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
                        if journal_mode.to_lowercase() != "wal" {
                            return Err(rusqlite::Error::SqliteFailure(
                                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                                Some("Database is no longer in WAL mode".to_string()),
                            ));
                        }
                        Ok(())
                    })
                    .await
                    .map_err(|_| deadpool::managed::HookError::Message("Interact error".into()))?
                    .map_err(|e: rusqlite::Error| {
                        deadpool::managed::HookError::Message(e.to_string().into())
                    })
            })
        }))
        .build()
        .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

    Ok(pool)
}
