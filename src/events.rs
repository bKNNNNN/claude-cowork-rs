use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub enum Event {
    #[serde(rename = "stdout")]
    Stdout { id: String, data: String },

    #[serde(rename = "exit")]
    Exit {
        id: String,
        #[serde(rename = "exitCode")]
        exit_code: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        signal: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "oomKillCount")]
        oom_kill_count: Option<i32>,
    },

    #[serde(rename = "vmStarted")]
    VmStarted { name: String },

    #[serde(rename = "vmStopped")]
    VmStopped { name: String },

    #[serde(rename = "apiReachability")]
    ApiReachability {
        reachability: String,
        #[serde(rename = "willTryRecover")]
        will_try_recover: bool,
    },

    #[serde(rename = "error")]
    Error {
        id: String,
        message: String,
        fatal: bool,
    },
}

impl Event {
    pub fn vm_started(name: &str) -> Self {
        Self::VmStarted {
            name: name.to_string(),
        }
    }

    pub fn vm_stopped(name: &str) -> Self {
        Self::VmStopped {
            name: name.to_string(),
        }
    }

    pub fn api_reachable() -> Self {
        Self::ApiReachability {
            reachability: "reachable".to_string(),
            will_try_recover: false,
        }
    }

    pub fn stdout(id: &str, data: String) -> Self {
        Self::Stdout {
            id: id.to_string(),
            data,
        }
    }

    pub fn exit(id: &str, exit_code: i32, signal: Option<String>) -> Self {
        Self::Exit {
            id: id.to_string(),
            exit_code,
            signal,
            oom_kill_count: None,
        }
    }

    pub fn error(id: &str, message: String, fatal: bool) -> Self {
        Self::Error {
            id: id.to_string(),
            message,
            fatal,
        }
    }
}
