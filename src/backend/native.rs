use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, info, warn};

use crate::backend::process::ManagedProcess;
use crate::backend::remap;
use crate::events::Event;

/// Native Linux backend — runs commands directly on the host.
pub struct NativeBackend {
    /// Whether the VM has been "started" (always native, just a flag).
    started: RwLock<bool>,
    /// Current session name.
    session_name: RwLock<String>,
    /// Active processes indexed by their ID.
    processes: Arc<Mutex<HashMap<String, ManagedProcess>>>,
    /// Channel for broadcasting events to subscribed clients.
    event_tx: mpsc::UnboundedSender<Event>,
    /// Debug logging enabled.
    debug_logging: RwLock<bool>,
}

impl NativeBackend {
    pub fn new(event_tx: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            started: RwLock::new(false),
            session_name: RwLock::new(String::new()),
            processes: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            debug_logging: RwLock::new(false),
        }
    }

    // --- VM lifecycle ---

    pub async fn configure(&self, _memory_mb: Option<u64>, _cpu_count: Option<u32>) {
        debug!("configure called (no-op for native backend)");
    }

    pub async fn create_vm(&self, name: &str, bundle_path: Option<&str>) {
        let session_name = remap::derive_session_name(name, bundle_path);
        debug!(session = %session_name, "createVM");
        if let Err(e) = remap::ensure_session_dir(&session_name) {
            warn!(error = %e, "failed to create session directory");
        }
        *self.session_name.write().await = session_name;
    }

    pub async fn start_vm(&self, name: &str, bundle_path: Option<&str>) {
        let session_name = remap::derive_session_name(name, bundle_path);
        info!(session = %session_name, "startVM");
        if let Err(e) = remap::ensure_session_dir(&session_name) {
            warn!(error = %e, "failed to create session directory");
        }
        *self.session_name.write().await = session_name.clone();
        *self.started.write().await = true;

        // Emit vmStarted and apiReachability events after a short delay
        let tx = self.event_tx.clone();
        let sname = session_name.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = tx.send(Event::vm_started(&sname));
            let _ = tx.send(Event::api_reachable());
        });
    }

    pub async fn stop_vm(&self, name: &str) {
        let session_name = if name.is_empty() {
            self.session_name.read().await.clone()
        } else {
            name.to_string()
        };
        info!(session = %session_name, "stopVM — killing all processes");

        // Kill all tracked processes
        let mut procs = self.processes.lock().await;
        for (id, proc) in procs.iter() {
            info!(process_id = %id, "killing process on stop");
            let _ = proc.kill(nix::sys::signal::Signal::SIGTERM);
        }
        procs.clear();

        *self.started.write().await = false;
        let _ = self.event_tx.send(Event::vm_stopped(&session_name));
    }

    pub async fn is_running(&self) -> bool {
        *self.started.read().await
    }

    pub async fn is_guest_connected(&self) -> bool {
        *self.started.read().await
    }

    // --- Process management ---

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn(
        &self,
        name: &str,
        id: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: &str,
        additional_mounts: &HashMap<String, crate::rpc::types::MountInfo>,
    ) -> Result<String, String> {
        let session_name = if name.is_empty() {
            self.session_name.read().await.clone()
        } else {
            name.to_string()
        };

        let sess_dir =
            remap::ensure_session_dir(&session_name).map_err(|e| format!("session dir: {e}"))?;

        // Create mount symlinks
        for (mount_name, mount_info) in additional_mounts {
            let link = sess_dir.join("mnt").join(mount_name);
            if !link.exists()
                && let Err(e) = std::os::unix::fs::symlink(&mount_info.path, &link)
            {
                warn!(
                    mount = %mount_name,
                    target = %mount_info.path,
                    error = %e,
                    "failed to create mount symlink"
                );
            }
        }

        // Build path remappings
        let remaps = remap::build_mount_remaps(&session_name, &sess_dir);

        // Filter and prepare environment
        let filtered_env = remap::filter_env(env);

        // Remap cwd
        let real_cwd = if cwd.is_empty() {
            // Try to use first non-hidden mount as workspace
            additional_mounts
                .iter()
                .find(|(name, _)| !name.starts_with('.'))
                .map(|(_, info)| info.path.clone())
                .unwrap_or_default()
        } else {
            remap::remap_cwd(cwd, &remaps)
        };

        // Spawn the process
        let proc = ManagedProcess::spawn(
            id.to_string(),
            command,
            args,
            &filtered_env,
            &real_cwd,
            self.event_tx.clone(),
            remaps,
        )?;

        let proc_id = proc.id.clone();

        // Track the process and set up exit monitoring
        let processes = Arc::clone(&self.processes);
        let event_tx = self.event_tx.clone();
        let pid_for_wait = proc_id.clone();

        self.processes.lock().await.insert(proc_id.clone(), proc);

        // Spawn a task to monitor process exit
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                let mut procs = processes.lock().await;
                if let Some(p) = procs.get_mut(&pid_for_wait) {
                    if !p.is_running() {
                        let (code, signal) = p.wait().await;
                        info!(
                            process_id = %pid_for_wait,
                            exit_code = code,
                            "process exited"
                        );
                        let _ = event_tx.send(Event::exit(&pid_for_wait, code, signal));
                        procs.remove(&pid_for_wait);
                        break;
                    }
                } else {
                    break;
                }
                drop(procs);
            }
        });

        Ok(proc_id)
    }

    pub async fn kill_process(&self, id: &str, signal_name: Option<&str>) -> Result<(), String> {
        let signal = parse_signal(signal_name.unwrap_or("SIGTERM"))?;
        let procs = self.processes.lock().await;
        if let Some(proc) = procs.get(id) {
            proc.kill(signal)
        } else {
            Err(format!("process {id} not found"))
        }
    }

    pub async fn write_stdin(&self, id: &str, data: &str) -> Result<(), String> {
        let procs = self.processes.lock().await;
        if let Some(proc) = procs.get(id) {
            proc.write_stdin(data).await
        } else {
            Err(format!("process {id} not found"))
        }
    }

    pub async fn is_process_running(&self, id: &str) -> bool {
        let mut procs = self.processes.lock().await;
        if let Some(proc) = procs.get_mut(id) {
            proc.is_running()
        } else {
            false
        }
    }

    // --- File & path methods ---

    pub async fn mount_path(&self, _name: &str, _host_path: &str, _guest_path: &str) {
        debug!("mountPath called (no-op for native backend)");
    }

    pub async fn read_file(&self, _name: &str, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("read file: {e}"))
    }

    // --- Stubs ---

    pub async fn install_sdk(&self) {
        debug!("installSdk called (no-op)");
    }

    pub async fn add_approved_oauth_token(&self, _name: &str, _token: &str) {
        debug!("addApprovedOauthToken called (no-op)");
    }

    pub async fn set_debug_logging(&self, enabled: bool) {
        info!(enabled, "setDebugLogging");
        *self.debug_logging.write().await = enabled;
    }

    pub async fn get_download_status(&self) -> String {
        "ready".to_string()
    }

    /// Clean up stale sessions from previous runs.
    pub async fn cleanup_stale_sessions(&self) {
        let base = remap::sessions_base_dir();
        if !base.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    info!(session = %path.display(), "cleaning up stale session");
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
        }
    }
}

fn parse_signal(name: &str) -> Result<nix::sys::signal::Signal, String> {
    use nix::sys::signal::Signal;
    match name.to_uppercase().trim_start_matches("SIG") {
        "KILL" => Ok(Signal::SIGKILL),
        "INT" => Ok(Signal::SIGINT),
        "QUIT" => Ok(Signal::SIGQUIT),
        "HUP" => Ok(Signal::SIGHUP),
        "USR1" => Ok(Signal::SIGUSR1),
        "USR2" => Ok(Signal::SIGUSR2),
        "TERM" | "" => Ok(Signal::SIGTERM),
        other => Err(format!("unsupported signal: {other}")),
    }
}
