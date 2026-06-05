mod init;
mod validate;

use clap::Parser;
use llamaforge_core::config::{ConfigHandle, ConfigLoadError, ConfigPaths, TomlFileConfigSource};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to llamaforge.toml (overrides env var and default search)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to priority_rules.toml (optional overlay)
    #[arg(long)]
    priority_rules: Option<PathBuf>,

    /// Writable state directory (overrides env var and default)
    #[arg(long)]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Initialize default configuration files and state directory
    Init {
        /// Path to write llamaforge.toml
        #[arg(long)]
        out: Option<PathBuf>,

        /// Path to write priority_rules.toml
        #[arg(long)]
        rules: Option<PathBuf>,

        /// Writable state directory
        #[arg(long)]
        state_dir: Option<PathBuf>,

        /// Overwrite existing files
        #[arg(long)]
        force: bool,

        /// Launch the browser-based setup wizard instead of writing a template
        /// config (08_SETUP_WIZARD.md Section 12).
        #[arg(long)]
        interactive: bool,

        /// Idle timeout for the interactive wizard before it exits (e.g. 30m).
        #[arg(long, default_value = "30m")]
        timeout: humantime::Duration,
    },
    /// Validate configuration without starting the server
    Validate {
        /// Path to llamaforge.toml
        #[arg(long)]
        config: Option<PathBuf>,

        /// Path to priority_rules.toml
        #[arg(long)]
        rules: Option<PathBuf>,
    },
    /// Run the orchestrator. Equivalent to invoking the binary with
    /// no subcommand; exists so `llamaforge serve --config <path>`
    /// matches the README and operator muscle-memory. Per-subcommand
    /// flags take precedence over the top-level forms.
    Serve {
        /// Path to llamaforge.toml
        #[arg(long)]
        config: Option<PathBuf>,

        /// Path to priority_rules.toml
        #[arg(long)]
        rules: Option<PathBuf>,

        /// Writable state directory
        #[arg(long)]
        state_dir: Option<PathBuf>,
    },
}

fn xdg_config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        if let Some(home) = env::var_os("HOME") {
            PathBuf::from(home).join(".config")
        } else {
            PathBuf::from("/etc") // fallback if HOME is missing
        }
    })
}

fn find_config() -> PathBuf {
    if let Ok(p) = env::var("LLAMAFORGE_CONFIG") {
        return PathBuf::from(p);
    }
    let config_home = xdg_config_home();
    let xdg_path = config_home.join("llamaforge").join("llamaforge.toml");
    if xdg_path.exists() {
        return xdg_path;
    }
    let etc_path = PathBuf::from("/etc/llamaforge/llamaforge.toml");
    if etc_path.exists() {
        return etc_path;
    }
    xdg_path
}

fn find_rules() -> PathBuf {
    if let Ok(p) = env::var("LLAMAFORGE_RULES") {
        return PathBuf::from(p);
    }
    let config_home = xdg_config_home();
    let xdg_path = config_home.join("llamaforge").join("priority_rules.toml");
    if xdg_path.exists() {
        return xdg_path;
    }
    let etc_path = PathBuf::from("/etc/llamaforge/priority_rules.toml");
    if etc_path.exists() {
        return etc_path;
    }
    xdg_path
}

/// Platform-aware default state directory.
///
/// Precedence: explicit `$LLAMAFORGE_STATE_DIR` → platform-native
/// per-user state directory → the system-wide `/var/run/llamaforge/`
/// (only meaningful when running as root on Linux, where systemd /
/// runit can pre-create it).
///
/// * macOS: `~/Library/Application Support/llamaforge`
/// * Linux / *nix non-root: `$XDG_STATE_HOME/llamaforge`, or
///   `~/.local/state/llamaforge` if `$XDG_STATE_HOME` is unset.
/// * Otherwise: `/var/run/llamaforge/` (legacy default; preserved for
///   any container / sysd unit that already targets this path).
fn find_state_dir() -> PathBuf {
    if let Ok(p) = env::var("LLAMAFORGE_STATE_DIR") {
        return PathBuf::from(p);
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    {
        if let Some(h) = &home {
            return h.join("Library").join("Application Support").join("llamaforge");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(xdg) = env::var("XDG_STATE_HOME") {
            return PathBuf::from(xdg).join("llamaforge");
        }
        if let Some(h) = &home {
            return h.join(".local").join("state").join("llamaforge");
        }
    }
    // Fallback for container / root-managed setups where neither
    // HOME nor XDG_STATE_HOME is meaningful.
    PathBuf::from("/var/run/llamaforge/")
}

fn check_state_dir(path: &Path) {
    if !path.exists() || !path.is_dir() {
        print_state_dir_error(path);
        process::exit(78);
    }

    let lock_file = path.join("config.lock");
    match fs::OpenOptions::new().create(true).write(true).truncate(true).open(&lock_file) {
        Ok(_) => {}
        Err(_) => {
            print_state_dir_error(path);
            process::exit(78);
        }
    }
}

fn emit_boot_error(code: &str, message: &str) {
    let ts = chrono::Utc::now().to_rfc3339();
    let json = serde_json::json!({
        "timestamp": ts,
        "level": "ERROR",
        "code": code,
        "message": message
    });
    eprintln!("{}", json);
}

fn print_state_dir_error(path: &Path) {
    let msg = format!("state directory not writable: {}. LlamaForge needs a writable directory for its lock file (config.lock) and writeback temp files. Create it: mkdir -p {} ; chown <user>:<group> {}  Or pass --state-dir <other-path> / set $LLAMAFORGE_STATE_DIR.", path.display(), path.display(), path.display());
    emit_boot_error("STATE_DIR_NOT_WRITABLE", &msg);
}

pub(crate) fn handle_config_error(err: ConfigLoadError) -> ! {
    match err {
        ConfigLoadError::NotFound { path } => {
            let msg = format!("config file not found: {}. To get started: 1. Run `llamaforge init` to generate a default config. 2. Edit `binary_path`, `model_path`, `port`, `ctx_size` for your instances. 3. Re-run llamaforge. Search order: --config, $LLAMAFORGE_CONFIG, $XDG_CONFIG_HOME/llamaforge/, /etc/llamaforge/", path.display());
            emit_boot_error("CONFIG_NOT_FOUND", &msg);
        }
        ConfigLoadError::EmptyFile { path } => {
            let msg = format!("config file is empty: {}. This can happen if a previous writeback was killed mid-write. Recovery: 1. Restore the file from version control. 2. Or restore from backup. 3. Or run `llamaforge init --force`.", path.display());
            emit_boot_error("CONFIG_EMPTY", &msg);
        }
        ConfigLoadError::ParseError { path, source } => {
            let msg = format!("failed to parse {}: {}. Common causes: mismatched quotes/brackets, invalid TOML syntax, wrong field types.", path.display(), source);
            emit_boot_error("CONFIG_PARSE_ERROR", &msg);
        }
        ConfigLoadError::ValidationError { message } => {
            let msg = format!(
                "config validation failed: {}. Edit the config file to fix the issue and re-run.",
                message
            );
            emit_boot_error("CONFIG_VALIDATION_ERROR", &msg);
        }
        ConfigLoadError::IoError { path, source } => {
            let msg = format!("I/O error reading {}: {}. Check file permissions and that the path is on a local filesystem.", path.display(), source);
            emit_boot_error("CONFIG_IO_ERROR", &msg);
        }
        _ => {
            let msg = format!("config load failed: {}", err);
            emit_boot_error("CONFIG_LOAD_FAILED", &msg);
        }
    }
    process::exit(78);
}

fn main() {
    let cli = Cli::parse();

    // Resolve top-level overrides once; per-subcommand flags take
    // precedence over them where the subcommand declares its own.
    let cli_state_dir = cli.state_dir.clone();
    let cli_config = cli.config.clone();
    let cli_priority_rules = cli.priority_rules.clone();

    match cli.command {
        Some(Commands::Init { out, rules, state_dir, force, interactive, timeout }) => {
            if interactive {
                // Browser-based setup wizard (Section 12); never returns.
                init::run_interactive(timeout.into());
            }
            let out_path =
                out.unwrap_or_else(|| xdg_config_home().join("llamaforge").join("llamaforge.toml"));
            let rules_path =
                rules.unwrap_or_else(|| out_path.with_file_name("priority_rules.toml"));
            // Per-subcommand `--state-dir` wins; top-level
            // `--state-dir` is the fallback so `llamaforge --state-dir
            // /tmp/foo init` works as documented in `--help`.
            let s_dir = state_dir.or(cli_state_dir).unwrap_or_else(find_state_dir);
            init::run(&out_path, Some(&rules_path), &s_dir, force);
        }
        Some(Commands::Validate { config, rules }) => {
            // `--config` is honored verbatim — do not fall back to
            // `find_config()`'s default search path if the user
            // supplied a missing file. A typo silently routing to a
            // working default would let `validate` lie.
            let config_path = config.or(cli_config).unwrap_or_else(find_config);
            let rules_path = rules.or(cli_priority_rules).unwrap_or_else(find_rules);
            validate::run(&config_path, &rules_path, handle_config_error);
        }
        Some(Commands::Serve { config, rules, state_dir }) => {
            let config_path = config.or(cli_config).unwrap_or_else(find_config);
            let rules_path = rules.or(cli_priority_rules).unwrap_or_else(find_rules);
            let state_dir = state_dir.or(cli_state_dir).unwrap_or_else(find_state_dir);

            run_serve(config_path, rules_path, state_dir);
        }
        None => {
            let config_path = cli_config.unwrap_or_else(find_config);
            let rules_path = cli_priority_rules.unwrap_or_else(find_rules);
            let state_dir = cli_state_dir.unwrap_or_else(find_state_dir);

            run_serve(config_path, rules_path, state_dir);
        }
    }
}

fn run_serve(config_path: PathBuf, rules_path: PathBuf, state_dir: PathBuf) {
    check_state_dir(&state_dir);

    let paths = ConfigPaths::new(config_path, rules_path, state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths.clone()));

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            emit_boot_error(
                "RUNTIME_INIT_FAILED",
                &format!("failed to construct tokio runtime: {e}"),
            );
            process::exit(70);
        }
    };

    // Config-load triage (08_SETUP_WIZARD.md Section 13.1): a missing or empty
    // config means first-run → launch the setup wizard in-process; a parse
    // error is a real misconfiguration and exits 78.
    let result = match ConfigHandle::load(source) {
        Ok(handle) => rt.block_on(llamaforge::run(Arc::new(handle))),
        Err(ConfigLoadError::NotFound { .. }) | Err(ConfigLoadError::EmptyFile { .. }) => {
            rt.block_on(llamaforge::run_first_run(paths))
        }
        Err(err) => handle_config_error(err),
    };

    if let Err(e) = result {
        emit_boot_error("FATAL_ERROR", &format!("{e}"));
        process::exit(1);
    }
}
