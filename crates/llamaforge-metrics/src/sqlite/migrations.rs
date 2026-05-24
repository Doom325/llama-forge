//! Database schema migrations using rusqlite_migration.
//!
//! See design doc Section 2.6.

use rusqlite_migration::{Migrations, M};

/// Return the ordered list of database migrations.
pub fn get_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            r#"
        -- Event log for telemetry and audit records
        CREATE TABLE IF NOT EXISTS audit_events (
            event_id TEXT PRIMARY KEY,
            event_type TEXT NOT NULL,
            event_kind TEXT NOT NULL,
            server_id TEXT,
            request_id TEXT,
            created_at TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at);
        CREATE INDEX IF NOT EXISTS idx_audit_events_server_id ON audit_events(server_id);
        CREATE INDEX IF NOT EXISTS idx_audit_events_request_id ON audit_events(request_id);

        -- Performance and billing metrics per request
        CREATE TABLE IF NOT EXISTS request_metrics (
            request_id TEXT PRIMARY KEY,
            model_name TEXT NOT NULL,
            server_id TEXT,
            slot_id INTEGER,
            task_type TEXT NOT NULL,
            requestor TEXT NOT NULL,
            effective_priority INTEGER NOT NULL,
            
            received_at TEXT NOT NULL,
            dispatched_at TEXT,
            first_token_at TEXT,
            completed_at TEXT,
            
            queue_wait_ms REAL NOT NULL,
            ttft_ms REAL,
            generation_duration_ms REAL,
            total_latency_ms REAL NOT NULL,
            
            prompt_tokens INTEGER NOT NULL,
            completion_tokens INTEGER NOT NULL,
            
            prompt_tps REAL,
            generation_tps REAL,
            
            status TEXT NOT NULL,
            finish_reason TEXT,
            was_evicted INTEGER NOT NULL,
            eviction_count INTEGER NOT NULL,
            error_message TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_request_metrics_received_at ON request_metrics(received_at);
        CREATE INDEX IF NOT EXISTS idx_request_metrics_server_id ON request_metrics(server_id);
        CREATE INDEX IF NOT EXISTS idx_request_metrics_requestor ON request_metrics(requestor);

        -- Currently managed processes
        CREATE TABLE IF NOT EXISTS managed_processes (
            pid INTEGER PRIMARY KEY,
            server_id TEXT NOT NULL,
            started_at TEXT NOT NULL,
            cmdline TEXT NOT NULL
        );

        -- Historical snapshots of configuration
        CREATE TABLE IF NOT EXISTS config_snapshots (
            id TEXT PRIMARY KEY,
            trigger TEXT NOT NULL,
            created_at TEXT NOT NULL,
            config_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_config_snapshots_created_at ON config_snapshots(created_at);

        -- Targeted overrides for server configuration (see design doc §2.8)
        CREATE TABLE IF NOT EXISTS config_overlays (
            server_id TEXT NOT NULL,
            overlay_key TEXT NOT NULL,
            value TEXT NOT NULL,
            applied_at TEXT NOT NULL,
            PRIMARY KEY (server_id, overlay_key)
        );
        "#,
        ),
        M::up(
            r#"
        ALTER TABLE managed_processes ADD COLUMN port INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE managed_processes ADD COLUMN model_alias TEXT NOT NULL DEFAULT '';
        "#,
        ),
        M::up(
            r#"
        ALTER TABLE request_metrics ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE audit_events ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE config_snapshots ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1;
        "#,
        ),
    ])
}
