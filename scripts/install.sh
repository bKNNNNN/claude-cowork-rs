#!/usr/bin/env bash
set -euo pipefail

REPO="bKNNNNN/claude-cowork-linux"
BINARY_NAME="claude-cowork-linux"
INSTALL_DIR="$HOME/.local/bin"
SERVICE_DIR="$HOME/.config/systemd/user"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Install claude-cowork-linux daemon for Claude Desktop Cowork on Linux.

Options:
    --uninstall    Remove claude-cowork-linux
    --no-service   Skip systemd service setup
    --help         Show this help

One-liner:
    curl -fsSL https://raw.githubusercontent.com/$REPO/main/scripts/install.sh | bash
EOF
    exit 0
}

detect_arch() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64)  echo "x86_64-unknown-linux-musl" ;;
        aarch64) echo "aarch64-unknown-linux-musl" ;;
        *)
            error "Unsupported architecture: $arch"
            exit 1
            ;;
    esac
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' \
        | head -1 \
        | sed 's/.*"v\?\([^"]*\)".*/\1/'
}

download_binary() {
    local version="$1"
    local target="$2"
    local url="https://github.com/$REPO/releases/download/v${version}/${BINARY_NAME}-${target}"

    info "Downloading $BINARY_NAME v$version for $target..."
    mkdir -p "$INSTALL_DIR"
    curl -fsSL "$url" -o "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    ok "Binary installed to $INSTALL_DIR/$BINARY_NAME"
}

install_service() {
    info "Setting up systemd user service..."
    mkdir -p "$SERVICE_DIR"
    cat > "$SERVICE_DIR/claude-cowork.service" <<UNIT
[Unit]
Description=Claude Desktop Cowork Linux Daemon
After=default.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/$BINARY_NAME
Restart=on-failure
RestartSec=3
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
UNIT

    systemctl --user daemon-reload
    systemctl --user enable claude-cowork.service
    systemctl --user start claude-cowork.service
    ok "Systemd service enabled and started"
}

verify_install() {
    echo ""
    info "Verifying installation..."

    # Check binary
    if command -v "$BINARY_NAME" &>/dev/null; then
        ok "Binary found: $(command -v "$BINARY_NAME")"
    elif [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
        ok "Binary found: $INSTALL_DIR/$BINARY_NAME"
        warn "Make sure $INSTALL_DIR is in your PATH"
    else
        error "Binary not found!"
        return 1
    fi

    # Check service
    if systemctl --user is-active claude-cowork.service &>/dev/null; then
        ok "Service is running"
    else
        warn "Service is not running (start with: systemctl --user start claude-cowork)"
    fi

    # Check socket
    local socket_path="${XDG_RUNTIME_DIR:-/tmp}/cowork-vm-service.sock"
    if [ -S "$socket_path" ]; then
        ok "Socket exists: $socket_path"
    else
        warn "Socket not found yet (will be created when service starts)"
    fi

    # Check Claude Code
    if command -v claude &>/dev/null; then
        ok "Claude Code found: $(command -v claude)"
    else
        warn "Claude Code CLI not found (install from https://docs.anthropic.com/en/docs/claude-code)"
    fi

    echo ""
    ok "Installation complete! Restart Claude Desktop to enable Cowork."
}

do_uninstall() {
    info "Uninstalling claude-cowork-linux..."

    # Stop service
    if systemctl --user is-active claude-cowork.service &>/dev/null; then
        systemctl --user stop claude-cowork.service
    fi
    systemctl --user disable claude-cowork.service 2>/dev/null || true
    rm -f "$SERVICE_DIR/claude-cowork.service"
    systemctl --user daemon-reload

    # Remove binary
    rm -f "$INSTALL_DIR/$BINARY_NAME"

    # Remove sessions
    rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/claude-cowork"

    ok "Uninstalled successfully"
}

# --- Main ---

NO_SERVICE=false

for arg in "$@"; do
    case "$arg" in
        --uninstall)  do_uninstall; exit 0 ;;
        --no-service) NO_SERVICE=true ;;
        --help|-h)    usage ;;
        *)            error "Unknown option: $arg"; usage ;;
    esac
done

info "Installing claude-cowork-linux..."

target=$(detect_arch)
version=$(get_latest_version)

if [ -z "$version" ]; then
    error "Failed to get latest version. Building from source..."
    if command -v cargo &>/dev/null; then
        info "Building from source with cargo..."
        tmpdir=$(mktemp -d)
        git clone "https://github.com/$REPO.git" "$tmpdir/$BINARY_NAME"
        cd "$tmpdir/$BINARY_NAME"
        cargo build --release
        mkdir -p "$INSTALL_DIR"
        cp "target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
        chmod +x "$INSTALL_DIR/$BINARY_NAME"
        rm -rf "$tmpdir"
        ok "Built and installed from source"
    else
        error "Neither a release nor cargo found. Install Rust first: https://rustup.rs"
        exit 1
    fi
else
    download_binary "$version" "$target"
fi

if [ "$NO_SERVICE" = false ] && command -v systemctl &>/dev/null; then
    install_service
else
    warn "Skipping systemd service setup"
fi

verify_install
