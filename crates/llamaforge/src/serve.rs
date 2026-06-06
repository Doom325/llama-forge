//! Top-level `serve` entry point: builds the orchestrator, starts the
//! HTTP and gRPC transports, and waits for a shutdown signal.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use llamaforge_api::{setup_router, start_servers_with_setup, SetupMount, SetupState};
use llamaforge_core::config::{
    AppConfig, ConfigHandle, ConfigPaths, ConfigReader, ConfigWriter, TomlFileConfigSource,
};
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

/// First-run / setup mode (08_SETUP_WIZARD.md Section 13): boot a full transport
/// with an empty config and the `/api/setup/*` router mounted + mode guard, then
/// wait for the wizard to complete (`validate-launch` succeeds). On completion,
/// load the now-written config and transition to running mode in-process; if the
/// reload fails, fall back to setup mode and let the user retry.
pub async fn run_first_run(paths: ConfigPaths) -> anyhow::Result<()> {
    crate::tracing_init::init(AppConfig::default().global.log_format);
    tracing::info!("lifecycle.first_run.detected");

    let shutdown = CancellationToken::new();

    // Boot with an empty config backed by the (not-yet-existing) file source so
    // a later `reload()` reads the config the wizard writes.
    let file_source = Arc::new(TomlFileConfigSource::new(paths.clone()));
    let mut initial = AppConfig::default();
    // First-run has no config file to set the HTTP port, so it binds the default
    // (8000). `LLAMAFORGE_HTTP_PORT` overrides it — lets tests/automation bind an
    // ephemeral port instead of colliding on the fixed default (issue #281). The
    // transport binds this once at startup; the later config reload doesn't rebind.
    if let Some(port) =
        std::env::var("LLAMAFORGE_HTTP_PORT").ok().and_then(|p| p.parse::<u16>().ok())
    {
        initial.global.http_port = port;
    }
    let config = Arc::new(ConfigHandle::with_initial_config(initial, file_source));
    let (deps, bg) = build::build(config.clone(), shutdown.clone()).await?;

    let server_manager = bg.server_manager.clone();
    server_manager
        .clone()
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("server-manager start (setup): {}", e))?;
    // Reloads spawn the configured fleet via the diff forwarder.
    let diff_task = spawn_diff_forwarder(config.clone(), server_manager.clone());

    let http_port = config.snapshot().global.http_port;
    let setup_url = format!("http://127.0.0.1:{http_port}/ui/setup");
    let completion = Arc::new(tokio::sync::Notify::new());
    let setup_active = Arc::new(AtomicBool::new(true));
    let setup_state = SetupState::new().with_completion(completion.clone());
    let mount = SetupMount {
        setup_router: setup_router(setup_state),
        setup_active: setup_active.clone(),
        setup_url: setup_url.clone(),
    };

    let handles = start_servers_with_setup(deps, Some(mount))
        .await
        .map_err(|e| anyhow::anyhow!("transport start (setup): {}", e))?;
    tracing::info!(setup_url = %setup_url, "lifecycle.first_run.ready");

    // Drive the setup→running transition until it completes or is cancelled.
    let cancelled =
        run_setup_transition(&config, &setup_active, &completion, wait_for_signal()).await;

    // In running mode, keep serving (honoring SIGHUP reloads) until shutdown.
    if !cancelled {
        wait_for_shutdown_or_reload(&config).await;
    } else {
        tracing::info!("lifecycle.first_run.cancelled");
    }

    shutdown.cancel();
    handles.shutdown();
    let _ = handles.wait().await;
    diff_task.abort();
    bg.server_manager.shutdown_all().await;
    let _ = bg.scheduler.await;
    tracing::info!("lifecycle.first_run.completed");
    Ok(())
}

/// The setup→running transition loop, factored out of [`run_first_run`] so it
/// can be tested without a transport or ports (08_SETUP_WIZARD.md §13.2/13.5):
/// on each `completion` notification, hot-reload `config`; on success flip
/// `setup_active` to `false` and return `false` (running); on a failed reload
/// stay in setup and await the next completion (retry). Returns `true` if
/// `cancel` fires before the transition completes.
async fn run_setup_transition(
    config: &ConfigHandle,
    setup_active: &AtomicBool,
    completion: &tokio::sync::Notify,
    cancel: impl std::future::Future<Output = ()>,
) -> bool {
    tokio::pin!(cancel);
    loop {
        tokio::select! {
            _ = completion.notified() => match config.reload().await {
                Ok(diff) => {
                    setup_active.store(false, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(
                        changes = diff.changes.len(),
                        "lifecycle.first_run.transition.running"
                    );
                    return false;
                }
                Err(e) => tracing::warn!(error = %e, "lifecycle.first_run.reload_failed"),
            },
            _ = &mut cancel => return true,
        }
    }
}

/// Wait for Ctrl-C or SIGTERM (setup mode has no SIGHUP reload).
async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Round-trip a default `AppConfig` through the TOML source so a later
    /// `reload()` of `path` succeeds.
    fn write_valid_config(dir: &std::path::Path, path: &std::path::Path) {
        let paths =
            ConfigPaths::new(path.to_path_buf(), dir.join("no-rules.toml"), dir.to_path_buf());
        let source = Arc::new(TomlFileConfigSource::new(paths));
        let handle = ConfigHandle::with_initial_config(AppConfig::default(), source);
        std::fs::write(path, handle.export_toml().unwrap()).unwrap();
    }

    fn setup_config(dir: &std::path::Path, cfg: &std::path::Path) -> Arc<ConfigHandle> {
        let paths =
            ConfigPaths::new(cfg.to_path_buf(), dir.join("no-rules.toml"), dir.to_path_buf());
        Arc::new(ConfigHandle::with_initial_config(
            AppConfig::default(),
            Arc::new(TomlFileConfigSource::new(paths)),
        ))
    }

    // Headline first-run transition (VC-2.7 / tasks 7.10, 7.17): a failed reload
    // reverts (stays in setup), and a later valid config flips to running — the
    // retry path the `Notify` permit makes race-safe.
    #[tokio::test]
    async fn transition_reverts_on_failed_reload_then_flips_on_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("llamaforge.toml"); // absent initially
        let config = setup_config(dir.path(), &cfg);
        let setup_active = Arc::new(AtomicBool::new(true));
        let completion = Arc::new(tokio::sync::Notify::new());
        let cancel = CancellationToken::new();

        let task = tokio::spawn({
            let config = config.clone();
            let setup_active = setup_active.clone();
            let completion = completion.clone();
            let cancel = cancel.clone();
            async move {
                run_setup_transition(&config, &setup_active, &completion, cancel.cancelled_owned())
                    .await
            }
        });

        // No config file yet → reload fails → stays in setup (revert path).
        completion.notify_one();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(setup_active.load(Ordering::Relaxed), "stays in setup after a failed reload");
        assert!(!task.is_finished(), "loop keeps waiting to retry");

        // Valid config written → next completion → reload succeeds → running.
        write_valid_config(dir.path(), &cfg);
        completion.notify_one();
        let cancelled = task.await.unwrap();
        assert!(!cancelled);
        assert!(!setup_active.load(Ordering::Relaxed), "flips to running after a valid reload");
    }

    #[tokio::test]
    async fn transition_returns_cancelled_on_signal() {
        let dir = tempfile::tempdir().unwrap();
        let config = setup_config(dir.path(), &dir.path().join("llamaforge.toml"));
        let setup_active = Arc::new(AtomicBool::new(true));
        let completion = Arc::new(tokio::sync::Notify::new());
        let cancel = CancellationToken::new();
        cancel.cancel();

        let cancelled =
            run_setup_transition(&config, &setup_active, &completion, cancel.cancelled_owned())
                .await;
        assert!(cancelled);
        assert!(setup_active.load(Ordering::Relaxed), "stays in setup when cancelled");
    }
}
