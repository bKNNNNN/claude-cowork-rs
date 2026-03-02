use std::sync::Arc;

use serde_json::json;
use tracing::{debug, info};

use crate::backend::native::NativeBackend;
use crate::rpc::types::*;

/// Dispatch an RPC request to the appropriate handler.
pub async fn dispatch(
    method: &str,
    params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    debug!(method, "dispatching RPC");

    match method {
        "configure" => handle_configure(params, backend).await,
        "createVM" => handle_create_vm(params, backend).await,
        "startVM" => handle_start_vm(params, backend).await,
        "stopVM" => handle_stop_vm(params, backend).await,
        "isRunning" => handle_is_running(params, backend).await,
        "isGuestConnected" => handle_is_guest_connected(params, backend).await,
        "spawn" => handle_spawn(params, backend).await,
        "kill" => handle_kill(params, backend).await,
        "writeStdin" => handle_write_stdin(params, backend).await,
        "isProcessRunning" => handle_is_process_running(params, backend).await,
        "mountPath" => handle_mount_path(params, backend).await,
        "readFile" => handle_read_file(params, backend).await,
        "installSdk" => handle_install_sdk(params, backend).await,
        "addApprovedOauthToken" => handle_add_oauth_token(params, backend).await,
        "setDebugLogging" => handle_set_debug_logging(params, backend).await,
        "subscribeEvents" => {
            // subscribeEvents is handled specially in the server layer
            // because it needs to hold the connection open for streaming.
            // This should never be reached.
            RpcResponse::ok(json!({ "subscribed": true }))
        }
        "getDownloadStatus" => handle_get_download_status(backend).await,
        _ => RpcResponse::err(format!("unknown method: {method}")),
    }
}

async fn handle_configure(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: ConfigureParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.configure(p.memory_mb, p.cpu_count).await;
    RpcResponse::ok_null()
}

async fn handle_create_vm(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: CreateVmParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.create_vm(&p.name, p.bundle_path.as_deref()).await;
    RpcResponse::ok_null()
}

async fn handle_start_vm(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: StartVmParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.start_vm(&p.name, p.bundle_path.as_deref()).await;
    RpcResponse::ok_null()
}

async fn handle_stop_vm(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: StopVmParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.stop_vm(&p.name).await;
    RpcResponse::ok_null()
}

async fn handle_is_running(
    _params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let running = backend.is_running().await;
    RpcResponse::ok(json!({ "running": running }))
}

async fn handle_is_guest_connected(
    _params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let connected = backend.is_guest_connected().await;
    RpcResponse::ok(json!({ "connected": connected }))
}

async fn handle_spawn(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: SpawnParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    info!(
        process_id = %p.id,
        command = %p.command,
        args = ?p.args,
        cwd = %p.cwd,
        mounts = ?p.additional_mounts.keys().collect::<Vec<_>>(),
        "spawn request"
    );
    match backend
        .spawn(
            &p.name,
            &p.id,
            &p.command,
            &p.args,
            &p.env,
            &p.cwd,
            &p.additional_mounts,
        )
        .await
    {
        Ok(id) => RpcResponse::ok(json!({ "id": id })),
        Err(e) => RpcResponse::err(e),
    }
}

async fn handle_kill(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: KillParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    match backend.kill_process(&p.id, p.signal.as_deref()).await {
        Ok(()) => RpcResponse::ok_null(),
        Err(e) => RpcResponse::err(e),
    }
}

async fn handle_write_stdin(
    params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let p: WriteStdinParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    match backend.write_stdin(&p.id, &p.data).await {
        Ok(()) => RpcResponse::ok_null(),
        Err(e) => RpcResponse::err(e),
    }
}

async fn handle_is_process_running(
    params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let p: ProcessIdParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    let running = backend.is_process_running(&p.id).await;
    RpcResponse::ok(json!({ "running": running }))
}

async fn handle_mount_path(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: MountPathParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend
        .mount_path(&p.name, &p.host_path, &p.guest_path)
        .await;
    RpcResponse::ok_null()
}

async fn handle_read_file(params: serde_json::Value, backend: &Arc<NativeBackend>) -> RpcResponse {
    let p: ReadFileParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    match backend.read_file(&p.name, &p.path).await {
        Ok(data) => RpcResponse::ok(json!({ "data": data })),
        Err(e) => RpcResponse::err(e),
    }
}

async fn handle_install_sdk(
    _params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    backend.install_sdk().await;
    RpcResponse::ok_null()
}

async fn handle_add_oauth_token(
    params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let p: OauthTokenParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.add_approved_oauth_token(&p.name, &p.token).await;
    RpcResponse::ok_null()
}

async fn handle_set_debug_logging(
    params: serde_json::Value,
    backend: &Arc<NativeBackend>,
) -> RpcResponse {
    let p: DebugLoggingParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => return RpcResponse::err(format!("invalid params: {e}")),
    };
    backend.set_debug_logging(p.enabled).await;
    RpcResponse::ok_null()
}

async fn handle_get_download_status(backend: &Arc<NativeBackend>) -> RpcResponse {
    let status = backend.get_download_status().await;
    RpcResponse::ok(json!({ "status": status }))
}
