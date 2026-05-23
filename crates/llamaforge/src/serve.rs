//! Top-level `serve` entry point: builds the orchestrator, starts the
//! HTTP and gRPC transports, and waits for a shutdown signal.

use std::sync::Arc;

use llamaforge_core::config::{ConfigHandle, ConfigReader, ConfigWriter};
use llamaforge_server::ServerManager;
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

    // Forward config-change events to the server-manager so a
    // `ReloadConfig` RPC or a SIGHUP-triggered reload actually
    // propagates into the running fleet (hot-reloadable fields apply
    // immediately; restart-required fields are rejected at the RPC
    // boundary before they reach this path).
    let diff_task = spawn_diff_forwarder(config.clone(), server_manager.clone());

    let handles = llamaforge_api::transport::start_servers(deps)
        .await
        .map_err(|e| anyhow::anyhow!("transport start_servers: {}", e))?;

    tracing::info!("lifecycle.startup.ready");

    wait_for_shutdown_or_reload(&config).await;
    tracing::info!("lifecycle.shutdown.started");

    // Signal every subsystem.
    shutdown_token.cancel();
    handles.shutdown();
    diff_task.abort();

    // Wait for the transports to finish their graceful drain.
    if let Err(e) = handles.wait().await {
        tracing::error!(error = %e, "lifecycle.shutdown.wait_error");
    }

    bg.server_manager.shutdown_all().await;

    // Stop the scheduler.
    let _ = bg.scheduler.await;

    tracing::info!("lifecycle.shutdown.completed");
    Ok(())
}

/// Subscribe to the `ConfigHandle`'s diff broadcast and forward every
/// non-empty `ConfigDiff` to `ServerManager::apply_config_diff`. Before
/// this wire-up, `ReloadConfig` swapped the `ArcSwap` but the running
/// fleet never picked up the change — alias / lora-adapter edits
/// silently no-op'd.
fn spawn_diff_forwarder(
    config: Arc<ConfigHandle>,
    server_manager: Arc<ServerManager>,
) -> tokio::task::JoinHandle<()> {
    let mut rx = config.subscribe();
    tokio::spawn(async move {
        while let Ok(diff) = rx.recv().await {
            if diff.changes.is_empty() {
                continue;
            }
            let changes = diff.changes.len();
            tracing::info!(changes, "lifecycle.config.reload.applying");
            server_manager.apply_config_diff(&diff).await;
            tracing::info!(changes, "lifecycle.config.reload.applied");
        }
    })
}

async fn wait_for_shutdown_or_reload(config: &Arc<ConfigHandle>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "lifecycle.sigterm_handler_install_failed");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        let mut hup = match signal(SignalKind::hangup()) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "lifecycle.sighup_handler_install_failed");
                None
            }
        };

        loop {
            let hup_fut = async {
                match hup.as_mut() {
                    Some(s) => s.recv().await,
                    None => std::future::pending::<Option<()>>().await,
                }
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return,
                _ = term.recv() => return,
                _ = hup_fut => {
                    tracing::info!("lifecycle.config.reload.signaled");
                    match config.reload().await {
                        Ok(diff) => tracing::info!(
                            changes = diff.changes.len(),
                            "lifecycle.config.reload.completed"
                        ),
                        Err(llamaforge_core::config::ConfigReloadError::RestartRequired { fields }) => {
                            tracing::warn!(
                                ?fields,
                                "lifecycle.config.reload.restart_required"
                            );
                        }
                        Err(e) => tracing::error!(
                            error = %e,
                            "lifecycle.config.reload.failed"
                        ),
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
