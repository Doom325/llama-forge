use llamaforge_config::{ConfigLoadError, ConfigPaths, ConfigSource, TomlFileConfigSource};
use std::path::Path;
use std::process;
use std::sync::Arc;

pub fn run(config_path: &Path, rules_path: &Path, handle_error: fn(ConfigLoadError) -> !) -> ! {
    let dummy_state = std::env::temp_dir().join("llamaforge_validate");
    let paths = ConfigPaths::new(config_path.to_path_buf(), rules_path.to_path_buf(), dummy_state);
    let source = Arc::new(TomlFileConfigSource::new(paths));

    // Validate doesn't need ConfigHandle's hot-reload machinery — just call source.load() directly.
    match source.load() {
        Ok(config) => {
            let servers = config.servers.len();
            let task_types = config.priority_rules.task_types.len();
            println!(
                "config OK: {} servers, {} task types, validation passed",
                servers, task_types
            );
            process::exit(0);
        }
        Err(err) => {
            handle_error(err);
        }
    }
}
