use llamaforge_config::ServerConfig;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::process::Command;

/// A handle to a running server process, which might be a spawned child
/// or an adopted orphan process.
pub enum ProcessHandle {
    Child(tokio::process::Child),
    Orphan(u32),
}

impl ProcessHandle {
    pub fn id(&self) -> Option<u32> {
        match self {
            ProcessHandle::Child(c) => c.id(),
            ProcessHandle::Orphan(pid) => Some(*pid),
        }
    }

    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        match self {
            ProcessHandle::Child(c) => c.stdout.take(),
            ProcessHandle::Orphan(_) => None,
        }
    }

    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        match self {
            ProcessHandle::Child(c) => c.stderr.take(),
            ProcessHandle::Orphan(_) => None,
        }
    }

    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        match self {
            ProcessHandle::Child(c) => c.wait().await,
            ProcessHandle::Orphan(pid) => {
                #[cfg(unix)]
                {
                    use nix::sys::signal::kill;
                    use nix::unistd::Pid;
                    use std::os::unix::process::ExitStatusExt;

                    loop {
                        match kill(Pid::from_raw(*pid as i32), None) {
                            Ok(_) => {
                                // Alive
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                            Err(nix::errno::Errno::ESRCH) => {
                                // Dead
                                return Ok(ExitStatus::from_raw(0));
                            }
                            Err(e) => {
                                return Err(std::io::Error::other(e.to_string()));
                            }
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    // Fallback for non-unix, just wait for it to disappear
                    loop {
                        // Not supported on non-unix easily, just sleep forever
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

/// Spawns the underlying `llama-server` process with the given configuration.
/// Ensures the process is put into its own process group on Unix-like systems.
pub(crate) fn spawn_child_process(config: &ServerConfig) -> Result<ProcessHandle, std::io::Error> {
    let mut cmd = Command::new(&config.binary_path);
    cmd.arg("--model").arg(&config.model_path);
    cmd.arg("--port").arg(config.port.to_string());
    if config.ctx_size > 0 {
        cmd.arg("--ctx-size").arg(config.ctx_size.to_string());
    }
    cmd.arg("--parallel").arg(config.parallel_slots.to_string());
    if let Some(alias) = &config.alias {
        cmd.arg("--alias").arg(alias);
    }
    // In a real implementation, more arguments would be mapped here based on
    // backend, capabilities, lora adapters, etc.

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    cmd.kill_on_drop(true);

    let child = cmd.spawn()?;
    Ok(ProcessHandle::Child(child))
}
