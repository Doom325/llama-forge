use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use llamaforge_api::{wizard_router, SetupState};
use tokio_util::sync::CancellationToken;

pub const LLAMAFORGE_EXAMPLE: &str = include_str!("../../../examples/llamaforge.toml");
pub const PRIORITY_RULES_EXAMPLE: &str = include_str!("../../../examples/priority_rules.toml");

pub fn run(out_path: &Path, rules_path: Option<&Path>, state_dir: &Path, force: bool) -> ! {
    if let Err(e) = fs::create_dir_all(state_dir) {
        eprintln!("error: failed to create state dir {}: {}", state_dir.display(), e);
        process::exit(78);
    }
    if let Ok(metadata) = fs::metadata(state_dir) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o750);
        if let Err(e) = fs::set_permissions(state_dir, perms) {
            eprintln!(
                "warning: could not set state dir to 0750: {}. You may want to chmod manually.",
                e
            );
        }
    }

    if out_path.exists() && !force {
        let mtime = fs::metadata(out_path).and_then(|m| m.modified()).ok();
        let date_str = match mtime {
            Some(t) => {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            }
            None => "unknown".to_string(),
        };
        eprintln!(
            "error: existing config at {} last modified {}. Use --force to overwrite.",
            out_path.display(),
            date_str
        );
        process::exit(78);
    }

    if let Some(parent) = out_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("error: failed to create parent dir {}: {}", parent.display(), e);
            process::exit(78);
        }
    }

    if let Err(e) = fs::write(out_path, LLAMAFORGE_EXAMPLE) {
        eprintln!("error: failed to write config to {}: {}", out_path.display(), e);
        process::exit(78);
    }

    if let Some(r_path) = rules_path {
        if r_path.exists() && !force {
            eprintln!("error: existing rules at {}. Use --force to overwrite.", r_path.display());
            process::exit(78);
        }
        if let Some(parent) = r_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("error: failed to create parent dir {}: {}", parent.display(), e);
                process::exit(78);
            }
        }
        if let Err(e) = fs::write(r_path, PRIORITY_RULES_EXAMPLE) {
            eprintln!("error: failed to write rules to {}: {}", r_path.display(), e);
            process::exit(78);
        }
    }

    println!(
        "Wrote default config to {}. Edit `binary_path` (path to llama-server), `model_path`, `port`, `ctx_size` for your llama-server instances, then run `llamaforge`.",
        out_path.display()
    );
    process::exit(0);
}

/// Reminder cadence for a long-idle wizard (Section 12.5).
const REMINDER_INTERVAL: Duration = Duration::from_secs(300);

/// Run the browser-based setup wizard (08_SETUP_WIZARD.md Section 12): a minimal
/// HTTP server on an ephemeral loopback port serving the embedded UI + the
/// `/api/setup/*` API — no gRPC, no inference, no config handle. Exits 0 when
/// setup completes or the idle timeout fires, 130 on Ctrl-C. Never returns.
pub fn run_interactive(idle_timeout: Duration) -> ! {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: failed to start async runtime: {e}");
            process::exit(70);
        }
    };
    let code = runtime.block_on(wizard_main(idle_timeout));
    process::exit(code);
}

async fn wizard_main(idle_timeout: Duration) -> i32 {
    init_wizard_logging();

    let last_activity = Arc::new(AtomicU64::new(now_secs()));
    let completion = CancellationToken::new();

    let state = SetupState::new().with_completion(completion.clone());
    let router = wizard_router(state)
        .layer(axum::middleware::from_fn_with_state(last_activity.clone(), track_activity));

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("error: could not bind the wizard server: {e}");
            return 70;
        }
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("error: could not resolve the wizard address: {e}");
            return 70;
        }
    };
    let url = format!("http://{addr}/ui/setup");

    // One startup banner, always emitted regardless of RUST_LOG.
    println!("\nLlamaForge setup wizard is running at:\n    {url}\n");
    println!("Open it in your browser to begin. Press Ctrl-C to cancel. (RUST_LOG=debug for verbose logs)\n");
    open_browser(&url);

    let shutdown = CancellationToken::new();
    spawn_idle_monitor(last_activity, idle_timeout, url, shutdown.clone());

    let graceful = shutdown.clone();
    let completion_for_serve = completion.clone();
    let server =
        axum::serve(listener, router.into_make_service()).with_graceful_shutdown(async move {
            // Shut down on idle/expiry OR a completed setup (validate-launch ok).
            tokio::select! {
                _ = graceful.cancelled() => {}
                _ = completion_for_serve.cancelled() => {}
            }
        });

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                eprintln!("error: wizard server failed: {e}");
                return 70;
            }
        }
        _ = tokio::signal::ctrl_c() => return 130,
    }

    if completion.is_cancelled() {
        println!("Setup complete. Run `llamaforge serve` to start the orchestrator.");
    }
    0
}

/// Reset the idle timer on every `/api/setup/*` request.
async fn track_activity(
    State(last_activity): State<Arc<AtomicU64>>,
    req: Request,
    next: Next,
) -> Response {
    if req.uri().path().starts_with("/api/setup/") {
        last_activity.store(now_secs(), Ordering::Relaxed);
    }
    next.run(req).await
}

/// Exit when the wizard has been idle past `idle_timeout`; remind periodically.
fn spawn_idle_monitor(
    last_activity: Arc<AtomicU64>,
    idle_timeout: Duration,
    url: String,
    shutdown: CancellationToken,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut since_reminder = Duration::ZERO;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tick.tick() => {}
            }
            let idle = Duration::from_secs(
                now_secs().saturating_sub(last_activity.load(Ordering::Relaxed)),
            );
            if idle >= idle_timeout {
                println!(
                    "Setup wizard idle for {}; exiting.",
                    humantime::format_duration(idle_timeout)
                );
                shutdown.cancel();
                break;
            }
            since_reminder += Duration::from_secs(60);
            if since_reminder >= REMINDER_INTERVAL {
                since_reminder = Duration::ZERO;
                println!("Setup wizard still running at {url}");
            }
        }
    });
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Best-effort browser launch (`open` on macOS, `xdg-open` elsewhere).
fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Wizard logging posture (Section 12.3): default `warn` to stderr.
fn init_wizard_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ =
        tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_examples_match_repo() {
        let repo_config = fs::read_to_string("../../examples/llamaforge.toml").unwrap();
        let repo_rules = fs::read_to_string("../../examples/priority_rules.toml").unwrap();

        assert_eq!(repo_config, LLAMAFORGE_EXAMPLE);
        assert_eq!(repo_rules, PRIORITY_RULES_EXAMPLE);
    }
}
