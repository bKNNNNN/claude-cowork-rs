mod backend;
mod events;
mod health;
mod protocol;
mod rpc;
mod server;

use std::sync::Arc;

use clap::Parser;
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "claude-cowork-rs",
    version,
    about = "Linux daemon for Claude Desktop Cowork (Local Agent Mode)"
)]
struct Cli {
    /// Enable debug logging
    #[arg(long)]
    debug: bool,

    /// Custom socket path (default: $XDG_RUNTIME_DIR/cowork-vm-service.sock)
    #[arg(long)]
    socket_path: Option<String>,

    /// Run a health check against the running daemon
    #[arg(long)]
    health: bool,

    /// Show daemon status
    #[arg(long)]
    status: bool,

    /// Clean up stale sessions and exit
    #[arg(long)]
    cleanup: bool,
}

fn default_socket_path() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| format!("{dir}/cowork-vm-service.sock"))
        .unwrap_or_else(|_| "/tmp/cowork-vm-service.sock".to_string())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let socket_path = cli.socket_path.unwrap_or_else(default_socket_path);

    // Set up logging
    let filter = if cli.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Handle subcommands
    if cli.health {
        match health::check(&socket_path).await {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Health check failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if cli.status {
        match health::status(&socket_path).await {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Status check failed: {e}");
                std::process::exit(1);
            }
        }
    }

    // Create the event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Create the native backend
    let backend = Arc::new(backend::native::NativeBackend::new(event_tx));

    // Clean up stale sessions
    backend.cleanup_stale_sessions().await;

    if cli.cleanup {
        info!("cleanup complete");
        return;
    }

    // Start the event relay (fans out to subscribed clients)
    server::start_event_relay(event_rx);

    // Set up graceful shutdown
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    tokio::spawn(async move {
        let ctrl_c = signal::ctrl_c();
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => info!("received SIGINT"),
            _ = sigterm.recv() => info!("received SIGTERM"),
        }

        let _ = shutdown_tx.send(()).await;
    });

    // Run the server
    info!(
        version = env!("CARGO_PKG_VERSION"),
        socket = %socket_path,
        "starting claude-cowork-linux"
    );

    if let Err(e) = server::run(&socket_path, backend, shutdown_rx).await {
        error!(error = %e, "server error");
        std::process::exit(1);
    }
}
