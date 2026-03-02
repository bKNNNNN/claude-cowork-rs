use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RpcRequest {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn ok(result: serde_json::Value) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn ok_null() -> Self {
        Self {
            success: true,
            result: Some(serde_json::Value::Null),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(message.into()),
        }
    }
}

// --- Parameter types for each RPC method ---
// These structs match the wire format from Claude Desktop.
// Fields may appear unused in Rust but exist for deserialization.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureParams {
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub cpu_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CreateVmParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bundle_path: Option<String>,
    #[serde(default)]
    pub disk_size_gb: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct StartVmParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bundle_path: Option<String>,
    #[serde(default)]
    pub memory_gb: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopVmParams {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct MountInfo {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub additional_mounts: HashMap<String, MountInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillParams {
    pub id: String,
    #[serde(default)]
    pub signal: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteStdinParams {
    pub id: String,
    pub data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MountPathParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub host_path: String,
    #[serde(default)]
    pub guest_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileParams {
    #[serde(default)]
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct OauthTokenParams {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugLoggingParams {
    #[serde(default)]
    pub enabled: bool,
}
