use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static SKILL_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""content":"/[a-zA-Z0-9_-]+:"#).expect("invalid skill prefix regex")
});

#[derive(Debug, Clone)]
pub struct PathRemap {
    pub from: String,
    pub to: String,
}

/// Remap VM paths in a string to real host paths.
///
/// VM paths look like `/sessions/<name>/mnt/<mount>` and get mapped
/// to the real host path where the mount points to.
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

/// Filter environment variables: remove empty values and blocked keys.
pub fn filter_env(env: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    const BLOCKED_KEYS: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

    env.iter()
        .filter(|(k, v)| !v.is_empty() && !BLOCKED_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Remap a working directory from VM path to real path.
pub fn remap_cwd(cwd: &str, remaps: &[PathRemap]) -> String {
    if cwd.is_empty() {
        return cwd.to_string();
    }
    remap_paths(cwd, remaps)
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
    fn test_filter_env() {
        let mut env = std::collections::HashMap::new();
        env.insert("HOME".to_string(), "/home/user".to_string());
        env.insert("EMPTY".to_string(), String::new());
        env.insert("CLAUDECODE".to_string(), "1".to_string());
        env.insert("CLAUDE_CODE_ENTRYPOINT".to_string(), "/foo".to_string());
        env.insert("PATH".to_string(), "/usr/bin".to_string());

        let filtered = filter_env(&env);
        let keys: Vec<&str> = filtered.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"HOME"));
        assert!(keys.contains(&"PATH"));
        assert!(!keys.contains(&"EMPTY"));
        assert!(!keys.contains(&"CLAUDECODE"));
        assert!(!keys.contains(&"CLAUDE_CODE_ENTRYPOINT"));
    }
}
