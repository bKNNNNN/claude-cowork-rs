use std::sync::Arc;

use serde_json::json;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use claude_cowork_rs::backend::native::NativeBackend;
use claude_cowork_rs::events::Event;
use claude_cowork_rs::protocol;

/// Helper: start the daemon on a temp socket, return socket path and shutdown sender.
async fn start_server() -> (String, mpsc::Sender<()>) {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();

    let (event_tx, event_rx) = mpsc::unbounded_channel::<Event>();
    let backend = Arc::new(NativeBackend::new(event_tx));

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

    let path = socket_path.clone();
    tokio::spawn(async move {
        claude_cowork_rs::server::start_event_relay(event_rx);
        claude_cowork_rs::server::run(&path, backend, shutdown_rx)
            .await
            .unwrap();
    });

    // Wait for socket to be ready
    for _ in 0..50 {
        if std::path::Path::new(&socket_path).exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Keep tempdir alive by leaking it (cleaned up by OS)
    std::mem::forget(dir);

    (socket_path, shutdown_tx)
}

/// Helper: send an RPC request and read the response.
async fn rpc_call(stream: &mut UnixStream, method: &str, params: serde_json::Value) -> serde_json::Value {
    let request = json!({
        "method": method,
        "params": params,
        "id": 1
    });
    let data = serde_json::to_vec(&request).unwrap();

    let (mut reader, mut writer) = stream.split();
    protocol::write_message(&mut writer, &data).await.unwrap();
    let resp_bytes = protocol::read_message(&mut reader).await.unwrap();
    serde_json::from_slice(&resp_bytes).unwrap()
}

// --- Integration Tests ---

#[tokio::test]
async fn test_configure() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "configure", json!({"memoryMb": 4096, "cpuCount": 2})).await;
    assert_eq!(resp["success"], true);
}

#[tokio::test]
async fn test_get_download_status() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "getDownloadStatus", json!({})).await;
    assert_eq!(resp["success"], true);
    assert_eq!(resp["result"]["status"], "ready");
}

#[tokio::test]
async fn test_is_running_before_start() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "isRunning", json!({})).await;
    assert_eq!(resp["success"], true);
    assert_eq!(resp["result"]["running"], false);
}

#[tokio::test]
async fn test_is_guest_connected_before_start() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "isGuestConnected", json!({})).await;
    assert_eq!(resp["success"], true);
    assert_eq!(resp["result"]["connected"], false);
}

#[tokio::test]
async fn test_vm_lifecycle() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // createVM
    let resp = rpc_call(&mut stream, "createVM", json!({"name": "test-vm"})).await;
    assert_eq!(resp["success"], true);

    // startVM
    let resp = rpc_call(&mut stream, "startVM", json!({"name": "test-vm"})).await;
    assert_eq!(resp["success"], true);

    // isRunning should be true now
    let resp = rpc_call(&mut stream, "isRunning", json!({})).await;
    assert_eq!(resp["result"]["running"], true);

    // isGuestConnected should be true now
    let resp = rpc_call(&mut stream, "isGuestConnected", json!({})).await;
    assert_eq!(resp["result"]["connected"], true);

    // stopVM
    let resp = rpc_call(&mut stream, "stopVM", json!({"name": "test-vm"})).await;
    assert_eq!(resp["success"], true);

    // isRunning should be false again
    let resp = rpc_call(&mut stream, "isRunning", json!({})).await;
    assert_eq!(resp["result"]["running"], false);
}

#[tokio::test]
async fn test_set_debug_logging() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "setDebugLogging", json!({"enabled": true})).await;
    assert_eq!(resp["success"], true);

    let resp = rpc_call(&mut stream, "setDebugLogging", json!({"enabled": false})).await;
    assert_eq!(resp["success"], true);
}

#[tokio::test]
async fn test_install_sdk_noop() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "installSdk", json!({})).await;
    assert_eq!(resp["success"], true);
}

#[tokio::test]
async fn test_add_oauth_token_noop() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "addApprovedOauthToken", json!({"name": "test", "token": "abc"})).await;
    assert_eq!(resp["success"], true);
}

#[tokio::test]
async fn test_mount_path_noop() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "mountPath", json!({"name": "test", "hostPath": "/tmp", "guestPath": "/mnt"})).await;
    assert_eq!(resp["success"], true);
}

#[tokio::test]
async fn test_unknown_method() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "nonExistentMethod", json!({})).await;
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("unknown method"));
}

#[tokio::test]
async fn test_read_file() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // Create a temp file to read
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("test.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let resp = rpc_call(&mut stream, "readFile", json!({
        "name": "test",
        "path": file_path.to_str().unwrap()
    })).await;
    assert_eq!(resp["success"], true);
    assert_eq!(resp["result"]["data"], "hello world");
}

#[tokio::test]
async fn test_read_file_not_found() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "readFile", json!({
        "name": "test",
        "path": "/tmp/nonexistent-file-12345.txt"
    })).await;
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("read file"));
}

#[tokio::test]
async fn test_spawn_echo() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // Start VM first
    rpc_call(&mut stream, "createVM", json!({"name": "spawn-test"})).await;
    rpc_call(&mut stream, "startVM", json!({"name": "spawn-test"})).await;

    // Spawn echo
    let resp = rpc_call(&mut stream, "spawn", json!({
        "name": "spawn-test",
        "id": "echo-1",
        "command": "echo",
        "args": ["hello"],
        "env": {},
        "cwd": "/tmp",
        "additionalMounts": {}
    })).await;
    assert_eq!(resp["success"], true);
    assert_eq!(resp["result"]["id"], "echo-1");

    // Wait for process to complete
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Should no longer be running
    let resp = rpc_call(&mut stream, "isProcessRunning", json!({"id": "echo-1"})).await;
    assert_eq!(resp["result"]["running"], false);
}

#[tokio::test]
async fn test_kill_process() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    rpc_call(&mut stream, "createVM", json!({"name": "kill-test"})).await;
    rpc_call(&mut stream, "startVM", json!({"name": "kill-test"})).await;

    // Spawn a long-running process
    let resp = rpc_call(&mut stream, "spawn", json!({
        "name": "kill-test",
        "id": "sleep-1",
        "command": "sleep",
        "args": ["60"],
        "env": {},
        "cwd": "/tmp",
        "additionalMounts": {}
    })).await;
    assert_eq!(resp["success"], true);

    // Should be running
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let resp = rpc_call(&mut stream, "isProcessRunning", json!({"id": "sleep-1"})).await;
    assert_eq!(resp["result"]["running"], true);

    // Kill it
    let resp = rpc_call(&mut stream, "kill", json!({"id": "sleep-1", "signal": "SIGTERM"})).await;
    assert_eq!(resp["success"], true);

    // Wait and check it's dead
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let resp = rpc_call(&mut stream, "isProcessRunning", json!({"id": "sleep-1"})).await;
    assert_eq!(resp["result"]["running"], false);
}

#[tokio::test]
async fn test_kill_nonexistent_process() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "kill", json!({"id": "nope"})).await;
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_is_process_running_nonexistent() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    let resp = rpc_call(&mut stream, "isProcessRunning", json!({"id": "nope"})).await;
    assert_eq!(resp["result"]["running"], false);
}

#[tokio::test]
async fn test_subscribe_events_receives_vm_started() {
    let (socket_path, _shutdown) = start_server().await;

    // Open event connection
    let mut event_stream = UnixStream::connect(&socket_path).await.unwrap();
    let subscribe_req = json!({"method": "subscribeEvents", "params": {}, "id": 1});
    let data = serde_json::to_vec(&subscribe_req).unwrap();
    let (mut event_reader, mut event_writer) = event_stream.split();
    protocol::write_message(&mut event_writer, &data).await.unwrap();

    // Read ack
    let ack_bytes = protocol::read_message(&mut event_reader).await.unwrap();
    let ack: serde_json::Value = serde_json::from_slice(&ack_bytes).unwrap();
    assert_eq!(ack["success"], true);
    assert_eq!(ack["result"]["subscribed"], true);

    // Open RPC connection and start VM
    let mut rpc_stream = UnixStream::connect(&socket_path).await.unwrap();
    rpc_call(&mut rpc_stream, "createVM", json!({"name": "event-test"})).await;
    rpc_call(&mut rpc_stream, "startVM", json!({"name": "event-test"})).await;

    // Read events until we find vmStarted for our session (other tests may emit events too)
    let mut found_vm_started = false;
    let mut found_api_reachable = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    while (!found_vm_started || !found_api_reachable) && tokio::time::Instant::now() < deadline {
        let event_bytes = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            protocol::read_message(&mut event_reader),
        )
        .await
        .expect("timeout waiting for events")
        .unwrap();
        let event: serde_json::Value = serde_json::from_slice(&event_bytes).unwrap();

        match event["type"].as_str().unwrap_or("") {
            "vmStarted" if event["name"].as_str().unwrap_or("").contains("event-test") => {
                found_vm_started = true;
            }
            "apiReachability" if event["reachability"] == "reachable" => {
                found_api_reachable = true;
            }
            _ => {} // skip events from other tests
        }
    }

    assert!(found_vm_started, "did not receive vmStarted event");
    assert!(found_api_reachable, "did not receive apiReachability event");
}

#[tokio::test]
async fn test_multiple_rpc_calls_on_same_connection() {
    let (socket_path, _shutdown) = start_server().await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // Multiple calls on same connection should work
    for _ in 0..5 {
        let resp = rpc_call(&mut stream, "isRunning", json!({})).await;
        assert_eq!(resp["success"], true);
    }
}

// --- Unit tests for event serialization ---

#[test]
fn test_event_vm_started_serialization() {
    let event = Event::vm_started("my-session");
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "vmStarted");
    assert_eq!(json["name"], "my-session");
}

#[test]
fn test_event_vm_stopped_serialization() {
    let event = Event::vm_stopped("my-session");
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "vmStopped");
    assert_eq!(json["name"], "my-session");
}

#[test]
fn test_event_api_reachable_serialization() {
    let event = Event::api_reachable();
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "apiReachability");
    assert_eq!(json["reachability"], "reachable");
    assert_eq!(json["willTryRecover"], false);
}

#[test]
fn test_event_stdout_serialization() {
    let event = Event::stdout("proc-1", "hello\n".to_string());
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "stdout");
    assert_eq!(json["id"], "proc-1");
    assert_eq!(json["data"], "hello\n");
}

#[test]
fn test_event_exit_serialization() {
    let event = Event::exit("proc-1", 0, None);
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "exit");
    assert_eq!(json["id"], "proc-1");
    assert_eq!(json["exitCode"], 0);
    assert!(json.get("signal").is_none());
}

#[test]
fn test_event_exit_with_signal_serialization() {
    let event = Event::exit("proc-1", 137, Some("SIGKILL".to_string()));
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "exit");
    assert_eq!(json["exitCode"], 137);
    assert_eq!(json["signal"], "SIGKILL");
}

#[test]
fn test_event_error_serialization() {
    let event = Event::error("proc-1", "something broke".to_string(), true);
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "error");
    assert_eq!(json["id"], "proc-1");
    assert_eq!(json["message"], "something broke");
    assert_eq!(json["fatal"], true);
}

// --- Unit tests for RPC types ---

#[test]
fn test_rpc_response_ok() {
    let resp = claude_cowork_rs::rpc::types::RpcResponse::ok(json!({"running": true}));
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["result"]["running"], true);
    assert!(json.get("error").is_none());
}

#[test]
fn test_rpc_response_ok_null() {
    let resp = claude_cowork_rs::rpc::types::RpcResponse::ok_null();
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], true);
    assert!(json["result"].is_null());
    assert!(json.get("error").is_none());
}

#[test]
fn test_rpc_response_err() {
    let resp = claude_cowork_rs::rpc::types::RpcResponse::err("something failed");
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], false);
    assert!(json.get("result").is_none());
    assert_eq!(json["error"], "something failed");
}

#[test]
fn test_rpc_request_deserialization() {
    let raw = json!({"method": "isRunning", "params": {}, "id": 42});
    let req: claude_cowork_rs::rpc::types::RpcRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(req.method, "isRunning");
    assert_eq!(req.id, 42);
}

#[test]
fn test_rpc_request_missing_optional_fields() {
    let raw = json!({"method": "isRunning"});
    let req: claude_cowork_rs::rpc::types::RpcRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(req.method, "isRunning");
    assert!(req.params.is_object() || req.params.is_null());
}

#[test]
fn test_spawn_params_deserialization() {
    let raw = json!({
        "name": "vm-1",
        "id": "proc-1",
        "command": "echo",
        "args": ["hello"],
        "env": {"FOO": "bar"},
        "cwd": "/tmp",
        "additionalMounts": {
            "workspace": {"path": "/home/user/proj", "mode": "readWrite"}
        }
    });
    let params: claude_cowork_rs::rpc::types::SpawnParams = serde_json::from_value(raw).unwrap();
    assert_eq!(params.name, "vm-1");
    assert_eq!(params.command, "echo");
    assert_eq!(params.args, vec!["hello"]);
    assert_eq!(params.env.get("FOO").unwrap(), "bar");
    assert!(params.additional_mounts.contains_key("workspace"));
}
