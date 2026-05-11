use llamaforge_config::ServerConfig;
use std::process::Stdio;
use tokio::process::Command;

/// Spawns the underlying `llama-server` process with the given configuration.
/// Ensures the process is put into its own process group on Unix-like systems.
pub(crate) fn spawn_child_process(
    config: &ServerConfig,
) -> Result<tokio::process::Child, std::io::Error> {
    let mut cmd = Command::new(&config.model_path);
    cmd.arg("--port").arg(config.port.to_string());
    cmd.arg("--ctx-size").arg(config.ctx_size.to_string());
    cmd.arg("--parallel").arg(config.parallel_slots.to_string());
    if let Some(alias) = &config.alias {
        cmd.arg("--alias").arg(alias);
    }
    // In a real implementation, more arguments would be mapped here based on
    // backend, capabilities, lora adapters, etc.

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    cmd.spawn()
}
