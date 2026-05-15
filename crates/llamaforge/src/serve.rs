//! Top-level `serve` entry point: builds the orchestrator, starts the
//! HTTP and gRPC transports, and waits for a shutdown signal.

use std::sync::Arc;

use llamaforge_config::{ConfigHandle, ConfigReader};
use tokio_util::sync::CancellationToken;

use crate::build;

pub async fn run(config: Arc<ConfigHandle>) -> anyhow::Result<()> {
    let snapshot = config.snapshot();
    crate::tracing_init::init(snapshot.global.log_format);

    tracing::info!(
        bind_address = %snapshot.global.bind_address,
        http_port = snapshot.global.http_port,
        grpc_port = snapshot.global.grpc_port,
        "lifecycle.startup.begin"
    );

    let shutdown_token = CancellationToken::new();
    let (deps, bg) = build::build(config.clone(), shutdown_token.clone()).await?;

    // Start the server-manager's lifecycle tasks (spawns child llama-server
    // processes per the config). Errors are propagated; the binary refuses
    // to start the transports if the fleet failed to come up.
    let server_manager = bg.server_manager.clone();
    server_manager
        .clone()
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("server-manager start: {}", e))?;

    let handles = llamaforge_transport::start_servers(deps)
        .await
        .map_err(|e| anyhow::anyhow!("transport start_servers: {}", e))?;

    tracing::info!("lifecycle.startup.ready");

    wait_for_shutdown().await;
    tracing::info!("lifecycle.shutdown.started");

    // Signal every subsystem.
    shutdown_token.cancel();
    handles.shutdown();

    // Wait for the transports to finish their graceful drain.
    if let Err(e) = handles.wait().await {
        tracing::error!(error = %e, "lifecycle.shutdown.wait_error");
    }

    // Stop the scheduler.
    let _ = bg.scheduler.await;

    tracing::info!("lifecycle.shutdown.completed");
    Ok(())
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to install SIGTERM handler; falling back to Ctrl+C only");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
