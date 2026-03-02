# claude-cowork-linux

Linux daemon for **Claude Desktop Cowork** (Local Agent Mode). Lets Claude Desktop delegate coding tasks to a local Claude Code instance on Linux — no VM required.

## How it works

Claude Desktop's Cowork feature communicates with a local VM service over a Unix socket using length-prefixed JSON-RPC. On macOS it uses Apple's Virtualization Framework, on Windows Hyper-V. On Linux there's no official VM backend.

This daemon implements the same protocol, but runs commands **directly on the host** instead of inside a VM. Single static binary, zero dependencies.

```
Claude Desktop (Electron)
    | (Length-prefixed JSON-RPC over Unix socket)
    v
claude-cowork-linux daemon
    |
    +-- Process spawning (tokio)
    +-- Path remapping (VM paths -> real paths)
    +-- Event streaming (stdout/stderr/exit)
    +-- Session management
```

## Requirements

- Linux (x86_64 or aarch64)
- [Claude Desktop](https://claude.ai/download) for Linux
- Claude Pro subscription (or higher) for Cowork access
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed

## Install

### One-liner

```bash
curl -fsSL https://raw.githubusercontent.com/bKNNNNN/claude-cowork-linux/main/scripts/install.sh | bash
```

### Arch Linux (AUR)

```bash
yay -S claude-cowork-linux
```

### From source

```bash
git clone https://github.com/bKNNNNN/claude-cowork-linux.git
cd claude-cowork-linux
cargo build --release
cp target/release/claude-cowork-linux ~/.local/bin/
```

### Systemd service

```bash
# Copy the service file
mkdir -p ~/.config/systemd/user
cp packaging/claude-cowork.service ~/.config/systemd/user/

# Enable and start
systemctl --user daemon-reload
systemctl --user enable --now claude-cowork
```

## Usage

```bash
# Run in foreground (debug mode)
claude-cowork-linux --debug

# Health check
claude-cowork-linux --health

# Show status
claude-cowork-linux --status

# Custom socket path
claude-cowork-linux --socket-path /tmp/my-socket.sock

# Clean up stale sessions
claude-cowork-linux --cleanup
```

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/bKNNNNN/claude-cowork-linux/main/scripts/install.sh | bash -s -- --uninstall
```

Or manually:

```bash
systemctl --user disable --now claude-cowork
rm ~/.local/bin/claude-cowork-linux
rm ~/.config/systemd/user/claude-cowork.service
rm -rf ~/.local/share/claude-cowork
```

## Protocol

The daemon implements 17 RPC methods over a Unix socket at `$XDG_RUNTIME_DIR/cowork-vm-service.sock`:

| Method | Description |
|--------|-------------|
| `configure` | Accept VM config (no-op) |
| `createVM` | Create session directory |
| `startVM` | Emit vmStarted + apiReachability events |
| `stopVM` | Kill all processes, cleanup |
| `isRunning` | Return running state |
| `isGuestConnected` | Return connected state |
| `spawn` | Run command with path remapping |
| `kill` | Signal process (group kill) |
| `writeStdin` | Write to process stdin with remapping |
| `isProcessRunning` | Check process status |
| `mountPath` | No-op (native paths) |
| `readFile` | Read file contents |
| `installSdk` | No-op |
| `addApprovedOauthToken` | No-op |
| `setDebugLogging` | Toggle verbose logging |
| `subscribeEvents` | Stream events to client |
| `getDownloadStatus` | Return "ready" |

## Troubleshooting

### Daemon not starting

```bash
# Check if socket exists
ls -la ${XDG_RUNTIME_DIR}/cowork-vm-service.sock

# Check systemd logs
journalctl --user -u claude-cowork -f

# Run in debug mode
claude-cowork-linux --debug
```

### Claude Desktop doesn't see Cowork

Make sure the daemon is running and the socket is at the expected path. Restart Claude Desktop after starting the daemon.

### Process spawn fails

Check that `claude` CLI is in your PATH:
```bash
which claude
```

## Credits

Inspired by:
- [patrickjaja/claude-cowork-service](https://github.com/patrickjaja/claude-cowork-service) — Go daemon implementation
- [johnzfitch/claude-cowork-linux](https://github.com/johnzfitch/claude-cowork-linux) — JS Electron patch approach

## License

MIT
