use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::backend::native::NativeBackend;
use crate::events::Event;
use crate::protocol;
use crate::rpc::handlers;
use crate::rpc::types::RpcRequest;

/// Start the Unix socket server.
pub async fn run(
    socket_path: &str,
    backend: Arc<NativeBackend>,
    mut shutdown_rx: mpsc::Receiver<()>,
) -> std::io::Result<()> {
    // Remove stale socket file
    if std::path::Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(socket_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    info!(socket = %socket_path, "listening");

    // Set socket permissions (world-readable/writable for Claude Desktop)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o777))?;
    }

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        info!("client connected");
                        let backend = Arc::clone(&backend);
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, backend).await {
                                debug!(error = %e, "connection closed");
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "accept failed");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("shutting down server");
                break;
            }
        }
    }

    // Clean up socket file
    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

async fn handle_connection(stream: UnixStream, backend: Arc<NativeBackend>) -> std::io::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        // Read next message
        let msg = match protocol::read_message(&mut reader).await {
            Ok(msg) => msg,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    debug!("client disconnected");
                } else {
                    warn!(error = %e, "read error");
                }
                return Err(e);
            }
        };

        // Parse RPC request
        let request: RpcRequest = match serde_json::from_slice(&msg) {
            Ok(req) => req,
            Err(e) => {
                let resp = crate::rpc::types::RpcResponse::err(format!("parse error: {e}"));
                let resp_data = serde_json::to_vec(&resp).unwrap();
                protocol::write_message(&mut writer, &resp_data).await?;
                continue;
            }
        };

        debug!(method = %request.method, "received request");

        // Special handling for subscribeEvents — blocks the connection
        if request.method == "subscribeEvents" {
            return handle_subscribe_events(&mut writer, &backend).await;
        }

        // Dispatch to handler
        let response = handlers::dispatch(&request.method, request.params, &backend).await;
        let resp_data = serde_json::to_vec(&response).unwrap();
        protocol::write_message(&mut writer, &resp_data).await?;
    }
}

/// Handle subscribeEvents: send initial ack, then stream events until disconnect.
async fn handle_subscribe_events(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    _backend: &Arc<NativeBackend>,
) -> std::io::Result<()> {
    info!("client subscribed to events");

    // Send initial acknowledgement
    let ack = crate::rpc::types::RpcResponse::ok(serde_json::json!({ "subscribed": true }));
    let ack_data = serde_json::to_vec(&ack).unwrap();
    protocol::write_message(writer, &ack_data).await?;

    // Create a dedicated event receiver for this subscription.
    // We'll tap into the backend's broadcast channel.
    let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<Event>();

    // Register this subscriber
    // For simplicity, we use SUBSCRIBE_TX as a global relay.
    // The backend's event_tx feeds into the global event bus,
    // and we fork events to each subscriber here.
    let _handle = {
        let sub_tx = sub_tx;
        tokio::spawn(async move {
            // This task relays from the global event bus to this subscriber.
            // It's started by the server and lives for the duration of the subscription.
            // We use the SUBSCRIBERS list to register.
            SUBSCRIBERS.lock().await.push(sub_tx);
        })
    };

    // Stream events until the client disconnects or we get an error
    loop {
        match sub_rx.recv().await {
            Some(event) => {
                let event_data = match serde_json::to_vec(&event) {
                    Ok(d) => d,
                    Err(e) => {
                        error!(error = %e, "failed to serialize event");
                        continue;
                    }
                };
                if let Err(e) = protocol::write_message(writer, &event_data).await {
                    debug!(error = %e, "event write failed, unsubscribing");
                    break;
                }
            }
            None => {
                debug!("event channel closed");
                break;
            }
        }
    }

    Ok(())
}

// Global subscriber list for event fan-out.
use tokio::sync::Mutex;
static SUBSCRIBERS: Mutex<Vec<mpsc::UnboundedSender<Event>>> = Mutex::const_new(Vec::new());

/// Start the event relay task that fans out events from the backend to all subscribers.
pub fn start_event_relay(mut event_rx: mpsc::UnboundedReceiver<Event>) {
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let mut subs = SUBSCRIBERS.lock().await;
            // Remove closed senders
            subs.retain(|tx| !tx.is_closed());
            // Fan out event to all subscribers
            for tx in subs.iter() {
                let _ = tx.send(event.clone());
            }
        }
    });
}
