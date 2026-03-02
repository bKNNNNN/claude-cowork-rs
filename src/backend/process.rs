use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::backend::remap::{self, PathRemap};
use crate::events::Event;

/// A managed process with stdin access and event streaming.
pub struct ManagedProcess {
    pub id: String,
    pub child: Child,
    stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    remaps: Vec<PathRemap>,
}

impl ManagedProcess {
    /// Spawn a new process and start streaming its output as events.
    pub fn spawn(
        id: String,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &str,
        event_tx: mpsc::UnboundedSender<Event>,
        remaps: Vec<PathRemap>,
    ) -> Result<Self, String> {
        let resolved_cmd = resolve_command(command);
        info!(process_id = %id, command = %resolved_cmd, "spawning process");

        let mut cmd = Command::new(&resolved_cmd);
        // env is pre-built (daemon env + overlay + filtering), so clear and set explicitly
        cmd.args(args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set process group for group signaling
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                    .map_err(std::io::Error::other)?;
                Ok(())
            });
        }

        if !cwd.is_empty() {
            let real_cwd = remap::remap_cwd(cwd, &remaps);
            debug!(original_cwd = %cwd, resolved_cwd = %real_cwd, "cwd resolution");
            if std::path::Path::new(&real_cwd).is_dir() {
                cmd.current_dir(&real_cwd);
            } else {
                warn!(original_cwd = %cwd, resolved_cwd = %real_cwd, "working directory does not exist, using default");
            }
        }

        let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        // Spawn stdin writer task
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        if let Some(mut stdin_handle) = stdin {
            tokio::spawn(async move {
                while let Some(data) = stdin_rx.recv().await {
                    if let Err(e) = stdin_handle.write_all(&data).await {
                        debug!(error = %e, "stdin write failed (process likely exited)");
                        break;
                    }
                    if let Err(e) = stdin_handle.flush().await {
                        debug!(error = %e, "stdin flush failed");
                        break;
                    }
                }
            });
        }

        // Stream stdout as events
        let proc_id = id.clone();
        if let Some(stdout) = stdout {
            let tx = event_tx.clone();
            let pid = proc_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::with_capacity(10 * 1024 * 1024, stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(process_id = %pid, stream = "stdout", "{}", line);
                    if tx.send(Event::stdout(&pid, format!("{line}\n"))).is_err() {
                        break;
                    }
                }
            });
        }

        // Stream stderr as stdout events (Claude Code reads stdout only)
        if let Some(stderr) = stderr {
            let tx = event_tx.clone();
            let pid = proc_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::with_capacity(10 * 1024 * 1024, stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(process_id = %pid, stream = "stderr", "{}", line);
                    if tx.send(Event::stdout(&pid, format!("{line}\n"))).is_err() {
                        break;
                    }
                }
            });
        }

        // Wait for exit and emit exit event
        {
            let tx = event_tx;
            let pid = proc_id;
            let child_id = child.id();
            tokio::spawn(async move {
                // We need to wait a bit for the child to be fully set up
                // The actual wait happens via the process handle in the manager
                if let Some(pid_val) = child_id {
                    debug!(pid = pid_val, process_id = %pid, "process spawned, monitoring");
                }
                // Exit event is emitted by the process manager when it detects exit
                let _ = tx; // keep tx alive for stdout/stderr tasks
                let _ = pid;
            });
        }

        Ok(Self {
            id,
            child,
            stdin_tx: Some(stdin_tx),
            remaps,
        })
    }

    /// Write data to the process stdin with path remapping and skill prefix stripping.
    pub async fn write_stdin(&self, data: &str) -> Result<(), String> {
        let remapped = remap::remap_paths(data, &self.remaps);
        let stripped = remap::strip_skill_prefix(&remapped);

        if let Some(tx) = &self.stdin_tx {
            tx.send(stripped.into_bytes())
                .await
                .map_err(|_| "process stdin closed".to_string())
        } else {
            Err("no stdin handle".to_string())
        }
    }

    /// Check if the process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kill the process with a signal. Kills the entire process group.
    pub fn kill(&self, signal: nix::sys::signal::Signal) -> Result<(), String> {
        if let Some(pid) = self.child.id() {
            let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
            nix::sys::signal::kill(pgid, signal).map_err(|e| format!("kill failed: {e}"))
        } else {
            Err("process already exited".to_string())
        }
    }

    /// Wait for the process to exit and return the exit code.
    pub async fn wait(&mut self) -> (i32, Option<String>) {
        match self.child.wait().await {
            Ok(status) => {
                let code = status.code().unwrap_or(-1);
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(sig) = status.signal() {
                        return (128 + sig, Some(signal_name(sig)));
                    }
                }
                (code, None)
            }
            Err(e) => {
                error!(error = %e, "failed to wait for process");
                (-1, None)
            }
        }
    }
}

/// Resolve a command name to its full path.
fn resolve_command(command: &str) -> String {
    // If it's already an absolute path, use it directly
    if command.starts_with('/') {
        return command.to_string();
    }

    // Try which via PATH
    if let Ok(output) = std::process::Command::new("which").arg(command).output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }

    // Try via login shell (gets full user PATH)
    if let Ok(output) = std::process::Command::new("bash")
        .args(["-lc", &format!("which {command}")])
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return path;
        }
    }

    // Fallback: return as-is
    command.to_string()
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".to_string(),
        2 => "SIGINT".to_string(),
        3 => "SIGQUIT".to_string(),
        6 => "SIGABRT".to_string(),
        9 => "SIGKILL".to_string(),
        15 => "SIGTERM".to_string(),
        _ => format!("SIG{sig}"),
    }
}
