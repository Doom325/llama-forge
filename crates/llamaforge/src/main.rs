mod init;
mod validate;

use clap::Parser;
use llamaforge_config::{ConfigHandle, ConfigLoadError, ConfigPaths, TomlFileConfigSource};
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
    /// matches the README and operator muscle-memory.
    Serve,
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

fn print_state_dir_error(path: &Path) {
    eprintln!("error: state directory not writable: {}", path.display());
    eprintln!();
    eprintln!("LlamaForge needs a writable directory for its lock file (config.lock) and");
    eprintln!("writeback temp files. Create it:");
    eprintln!();
    eprintln!("  mkdir -p {}", path.display());
    eprintln!("  # If running as a non-root service user:");
    eprintln!("  chown <user>:<group> {}", path.display());
    eprintln!();
    eprintln!("Or pass --state-dir <other-path> / set $LLAMAFORGE_STATE_DIR.");
}

pub(crate) fn handle_config_error(err: ConfigLoadError) -> ! {
    match err {
        ConfigLoadError::NotFound { path } => {
            eprintln!("error: config file not found: {}", path.display());
            eprintln!();
            eprintln!("To get started:");
            eprintln!("  1. Run `llamaforge init` to generate a default config");
            eprintln!("  2. Edit `model_path`, `port`, `ctx_size` for your llama-server instances");
            eprintln!("  3. Re-run llamaforge");
            eprintln!();
            eprintln!("Search order: --config, $LLAMAFORGE_CONFIG, $XDG_CONFIG_HOME/llamaforge/, /etc/llamaforge/");
            eprintln!("See: docs/configuration.md");
        }
        ConfigLoadError::EmptyFile { path } => {
            eprintln!("error: config file is empty: {}", path.display());
            eprintln!();
            eprintln!(
                "This can happen if a previous writeback was killed mid-write. The atomic-rename"
            );
            eprintln!(
                "protocol should prevent this, but the truncation fallback path has a microsecond"
            );
            eprintln!("crash window.");
            eprintln!();
            eprintln!("Recovery:");
            eprintln!("  1. Restore the file from version control (it should be checked in)");
            eprintln!("  2. Or restore from your most recent backup");
            eprintln!("  3. Or run `llamaforge init --force` to generate a new default config");
        }
        ConfigLoadError::ParseError { path, source } => {
            eprintln!("error: failed to parse {}:", path.display());
            eprintln!("  {}", source);
            eprintln!();
            eprintln!("Common causes:");
            eprintln!("  - Mismatched quotes or brackets");
            eprintln!(
                "  - Invalid TOML syntax (e.g. duplicate [[servers]] entries with the same key)"
            );
            eprintln!(
                "  - Wrong field types (e.g. ctx_size = \"2048\" instead of ctx_size = 2048)"
            );
            eprintln!();
            eprintln!("Try: toml-cli check {}  (or your editor's TOML linter)", path.display());
        }
        ConfigLoadError::ValidationError { message } => {
            eprintln!("error: config validation failed: {}", message);
            eprintln!();
            eprintln!("Validation rules (design §6.1):");
            eprintln!("  - Server ports must be unique across all [servers] entries");
            eprintln!("  - Every server must have a non-empty model_path, ctx_size > 0, parallel_slots > 0");
            eprintln!("  - device.gpu_id must be unique within each server");
            eprintln!("  - global.queue_default_timeout_secs must be > 0");
            eprintln!();
            eprintln!("Edit the config file to fix the issue and re-run.");
        }
        ConfigLoadError::IoError { path, source } => {
            eprintln!("error: I/O error reading {}: {}", path.display(), source);
            eprintln!();
            eprintln!("Check file permissions and that the path is on a local filesystem (NFS/EFS");
            eprintln!("is unsupported because flock semantics differ).");
        }
        _ => {
            eprintln!("error: config load failed: {}", err);
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
        Some(Commands::Init { out, rules, state_dir, force }) => {
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
        Some(Commands::Serve) | None => {
            let config_path = cli_config.unwrap_or_else(find_config);
            let rules_path = cli_priority_rules.unwrap_or_else(find_rules);
            let state_dir = cli_state_dir.unwrap_or_else(find_state_dir);

            check_state_dir(&state_dir);

            let paths = ConfigPaths::new(config_path, rules_path, state_dir);
            let source = Arc::new(TomlFileConfigSource::new(paths));

            match ConfigHandle::load(source) {
                Ok(handle) => {
                    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            eprintln!("error: failed to construct tokio runtime: {}", e);
                            process::exit(70);
                        }
                    };
                    if let Err(e) = rt.block_on(llamaforge::run(Arc::new(handle))) {
                        eprintln!("error: {}", e);
                        process::exit(1);
                    }
                }
                Err(err) => {
                    handle_config_error(err);
                }
            }
        }
    }
}
