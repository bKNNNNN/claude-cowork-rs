use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use tracing::{debug, info};

use crate::rpc::types::MountInfo;

static SKILL_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""content":"/[a-zA-Z0-9_-]+:"#).expect("invalid skill prefix regex")
});

#[derive(Debug, Clone)]
pub struct PathRemap {
    pub from: String,
    pub to: String,
}

/// Remap VM paths in a string to real host paths.
pub fn remap_paths(input: &str, remaps: &[PathRemap]) -> String {
    let mut result = input.to_string();
    for r in remaps {
        result = result.replace(&r.from, &r.to);
    }
    result
}

/// Strip skill plugin prefixes from stdin data.
///
/// Transforms `"content":"/plugin-name:some-skill"` to `"content":"/some-skill"`
pub fn strip_skill_prefix(input: &str) -> String {
    SKILL_PREFIX_RE
        .replace_all(input, "\"content\":\"/")
        .to_string()
}

/// Build path remappings for a session's mounts.
///
/// Maps `/sessions/<name>/mnt/<mount_name>` -> actual target path of the symlink.
pub fn build_mount_remaps(session_name: &str, session_dir: &Path) -> Vec<PathRemap> {
    let mnt_dir = session_dir.join("mnt");
    let mut remaps = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&mnt_dir) {
        for entry in entries.flatten() {
            let mount_name = entry.file_name().to_string_lossy().to_string();
            let vm_path = format!("/sessions/{session_name}/mnt/{mount_name}");
            let real_path = std::fs::read_link(entry.path())
                .unwrap_or_else(|_| entry.path())
                .to_string_lossy()
                .to_string();
            remaps.push(PathRemap {
                from: vm_path,
                to: real_path,
            });
        }
    }

    // Also remap the bare session prefix
    let session_vm_prefix = format!("/sessions/{session_name}");
    let session_real = session_dir.to_string_lossy().to_string();
    remaps.push(PathRemap {
        from: session_vm_prefix,
        to: session_real,
    });

    remaps
}

/// Derive a session name from a bundle path if the name is empty.
pub fn derive_session_name(name: &str, bundle_path: Option<&str>) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    bundle_path
        .and_then(|p| Path::new(p).file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string())
}

/// Get the base directory for sessions.
pub fn sessions_base_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("claude-cowork")
        .join("sessions")
}

/// Get the session directory for a given session name.
pub fn session_dir(name: &str) -> PathBuf {
    sessions_base_dir().join(name)
}

/// Create session directory structure including mnt/.
pub fn ensure_session_dir(name: &str) -> std::io::Result<PathBuf> {
    let dir = session_dir(name);
    std::fs::create_dir_all(dir.join("mnt"))?;
    Ok(dir)
}

/// Build process environment: inherit daemon env, overlay requested vars, filter blocked keys.
///
/// Matches the Go implementation: starts with the current process environment,
/// overlays the env vars from the spawn request, then strips blocked keys and empty values.
pub fn build_env(
    requested_env: &HashMap<String, String>,
    remaps: &[PathRemap],
) -> Vec<(String, String)> {
    const BLOCKED_KEYS: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

    // Start with the daemon's own environment
    let mut env: HashMap<String, String> = std::env::vars().collect();

    // Overlay requested env vars
    for (k, v) in requested_env {
        if v.is_empty() {
            env.remove(k);
        } else {
            env.insert(k.clone(), v.clone());
        }
    }

    // Remove blocked keys
    for key in BLOCKED_KEYS {
        env.remove(*key);
    }

    // Remap env var values containing VM paths
    let env: Vec<(String, String)> = env
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            if v.starts_with("/sessions/") {
                let remapped = remap_paths(&v, remaps);
                debug!(key = %k, original = %v, remapped = %remapped, "remap env var");
                (k, remapped)
            } else {
                (k, v)
            }
        })
        .collect();

    env
}

/// Select the real workspace directory as cwd.
///
/// Instead of using the session dir (which has symlinked mounts that Glob can't follow),
/// use the actual path of the first non-hidden, non-utility mount.
pub fn select_workspace_cwd(
    cwd: &str,
    additional_mounts: &HashMap<String, MountInfo>,
    remaps: &[PathRemap],
) -> String {
    // First, try to find a real workspace mount
    let home = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .to_string_lossy()
        .to_string();

    for (mount_name, mount_info) in additional_mounts {
        // Skip hidden mounts, uploads, outputs
        if mount_name.starts_with('.')
            || mount_name == "uploads"
            || mount_name == "outputs"
        {
            continue;
        }

        let ws_path = &mount_info.path;
        if Path::new(ws_path).is_dir() {
            info!(mount = %mount_name, cwd = %ws_path, "using workspace mount as cwd");
            return ws_path.clone();
        }

        // Try relative to home
        let abs_path = format!("{home}/{ws_path}");
        if Path::new(&abs_path).is_dir() {
            info!(mount = %mount_name, cwd = %abs_path, "using workspace mount as cwd");
            return abs_path;
        }
    }

    // Fallback: remap the cwd if it's a VM path
    if !cwd.is_empty() {
        let needs_remap = remaps.iter().any(|r| cwd.starts_with(&r.from));
        if needs_remap {
            return remap_paths(cwd, remaps);
        }
        return cwd.to_string();
    }

    // Last resort: home directory
    home
}

/// Remap command args: VM paths and strip SDK servers from --mcp-config.
///
/// The Go implementation replaces --mcp-config with `{"mcpServers":{}}` because
/// SDK-type MCP servers require stdio proxying that the native backend can't provide.
/// Without this, Claude Code hangs trying to connect to SDK servers.
pub fn remap_args(args: &[String], remaps: &[PathRemap]) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;

    while i < args.len() {
        if args[i] == "--mcp-config" && i + 1 < args.len() {
            // Strip SDK servers from MCP config
            result.push(args[i].clone());
            result.push(r#"{"mcpServers":{}}"#.to_string());
            info!("stripped SDK MCP servers from --mcp-config");
            i += 2;
            continue;
        }

        // Remap VM paths in args
        let arg = &args[i];
        if arg.starts_with("/sessions/") {
            let remapped = remap_paths(arg, remaps);
            debug!(original = %arg, remapped = %remapped, "remap arg");
            result.push(remapped);
        } else {
            result.push(arg.clone());
        }
        i += 1;
    }

    result
}

/// Remap a working directory from VM path to real path.
/// Only remaps if the path starts with a known VM prefix.
pub fn remap_cwd(cwd: &str, remaps: &[PathRemap]) -> String {
    if cwd.is_empty() {
        return cwd.to_string();
    }
    let needs_remap = remaps.iter().any(|r| cwd.starts_with(&r.from));
    if needs_remap {
        remap_paths(cwd, remaps)
    } else {
        cwd.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remap_paths() {
        let remaps = vec![PathRemap {
            from: "/sessions/test/mnt/workspace".to_string(),
            to: "/home/user/project".to_string(),
        }];
        let input = "cd /sessions/test/mnt/workspace/src";
        let result = remap_paths(input, &remaps);
        assert_eq!(result, "cd /home/user/project/src");
    }

    #[test]
    fn test_strip_skill_prefix() {
        let input = r#"{"content":"/my-plugin:some-skill arg1"}"#;
        let result = strip_skill_prefix(input);
        assert_eq!(result, r#"{"content":"/some-skill arg1"}"#);
    }

    #[test]
    fn test_strip_skill_prefix_no_match() {
        let input = r#"{"content":"/some-skill arg1"}"#;
        let result = strip_skill_prefix(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_derive_session_name() {
        assert_eq!(derive_session_name("myvm", None), "myvm");
        assert_eq!(
            derive_session_name("", Some("/path/to/bundle.app")),
            "bundle.app"
        );
        assert_eq!(derive_session_name("", None), "default");
    }

    #[test]
    fn test_build_env_inherits_and_filters() {
        let mut requested = HashMap::new();
        requested.insert("CUSTOM_VAR".to_string(), "value".to_string());
        requested.insert("CLAUDECODE".to_string(), "1".to_string());
        requested.insert("EMPTY_VAR".to_string(), String::new());

        let env = build_env(&requested, &[]);
        let map: HashMap<&str, &str> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        // Should have inherited HOME from daemon
        assert!(map.contains_key("HOME"));
        // Should have custom var
        assert_eq!(map.get("CUSTOM_VAR"), Some(&"value"));
        // Should NOT have blocked keys
        assert!(!map.contains_key("CLAUDECODE"));
        // Should NOT have empty vars
        assert!(!map.contains_key("EMPTY_VAR"));
    }

    #[test]
    fn test_remap_args_strips_mcp_config() {
        let args = vec![
            "--verbose".to_string(),
            "--mcp-config".to_string(),
            r#"{"mcpServers":{"sdk1":{"type":"sdk"}}}"#.to_string(),
            "--model".to_string(),
            "claude-opus-4-6".to_string(),
        ];
        let result = remap_args(&args, &[]);
        assert_eq!(result[0], "--verbose");
        assert_eq!(result[1], "--mcp-config");
        assert_eq!(result[2], r#"{"mcpServers":{}}"#);
        assert_eq!(result[3], "--model");
    }

    #[test]
    fn test_remap_args_remaps_paths() {
        let remaps = vec![PathRemap {
            from: "/sessions/test".to_string(),
            to: "/home/user/.local/share/claude-cowork/sessions/test".to_string(),
        }];
        let args = vec![
            "--add-dir".to_string(),
            "/sessions/test/mnt/app.asar".to_string(),
        ];
        let result = remap_args(&args, &remaps);
        assert_eq!(result[1], "/home/user/.local/share/claude-cowork/sessions/test/mnt/app.asar");
    }
}
