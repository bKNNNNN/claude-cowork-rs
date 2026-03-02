use std::path::Path;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

use crate::protocol;

/// Run a health check against the running daemon.
pub async fn check(socket_path: &str) -> Result<(), String> {
    println!("Checking daemon at {socket_path}...");

    // Check socket exists
    if !Path::new(socket_path).exists() {
        return Err(format!("Socket not found: {socket_path}"));
    }
    println!("  Socket exists: OK");

    // Try to connect
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("Cannot connect to socket: {e}"))?;
    println!("  Connection: OK");

    // Send isRunning request
    let req = serde_json::json!({
        "method": "isRunning",
        "params": {},
        "id": 1
    });
    let data = serde_json::to_vec(&req).unwrap();
    protocol::write_message(&mut stream, &data)
        .await
        .map_err(|e| format!("Failed to write: {e}"))?;

    // Read response
    let resp_data = protocol::read_message(&mut stream)
        .await
        .map_err(|e| format!("Failed to read: {e}"))?;
    let resp: serde_json::Value =
        serde_json::from_slice(&resp_data).map_err(|e| format!("Invalid response: {e}"))?;
    println!("  RPC response: {resp}");

    // Shutdown write side
    stream.shutdown().await.ok();

    // Check Claude Code binary
    let claude_code = which_claude_code();
    match claude_code {
        Some(path) => println!("  Claude Code binary: {path}"),
        None => println!("  Claude Code binary: NOT FOUND (optional)"),
    }

    println!("\nDaemon is healthy!");
    Ok(())
}

/// Show daemon status.
pub async fn status(socket_path: &str) -> Result<(), String> {
    if !Path::new(socket_path).exists() {
        println!("Daemon is NOT running (socket not found)");
        return Ok(());
    }

    match UnixStream::connect(socket_path).await {
        Ok(mut stream) => {
            println!("Daemon is running at {socket_path}");

            // Query isRunning
            let req = serde_json::json!({
                "method": "isRunning",
                "params": {},
                "id": 1
            });
            let data = serde_json::to_vec(&req).unwrap();
            if protocol::write_message(&mut stream, &data).await.is_ok()
                && let Ok(resp_data) = protocol::read_message(&mut stream).await
                && let Ok(resp) = serde_json::from_slice::<serde_json::Value>(&resp_data)
            {
                let running = resp
                    .get("result")
                    .and_then(|r| r.get("running"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                println!("  VM started: {running}");
            }

            stream.shutdown().await.ok();
        }
        Err(e) => {
            println!("Daemon socket exists but cannot connect: {e}");
            println!("  (stale socket file? try removing {socket_path})");
        }
    }

    Ok(())
}

fn which_claude_code() -> Option<String> {
    std::process::Command::new("which")
        .arg("claude")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}
