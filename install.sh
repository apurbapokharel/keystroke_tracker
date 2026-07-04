#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
ENV_FILE="$PROJECT_DIR/.env"
BIN_DIR="$HOME/.local/bin"
BIN_PATH="$BIN_DIR/tracker"
SERVICE_DIR="$HOME/.config/systemd/user"
SERVICE_PATH="$SERVICE_DIR/tracker.service"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }

# ------------------------------------------------------------------
# 1. Input group check
# ------------------------------------------------------------------
info "Checking input group membership..."
if ! groups | grep -qE '\binput\b'; then
    info "Adding user '$USER' to 'input' group..."
    sudo usermod -aG input "$USER"
    echo ""
    warn "=============================================================="
    warn "You have been added to the 'input' group."
    warn "This requires a REBOOT (or re-login) to take effect."
    warn "After rebooting, run this install script again to continue."
    warn "=============================================================="
    echo ""
    exit 1
fi
info "User is in 'input' group."

# ------------------------------------------------------------------
# 2. Prerequisite check
# ------------------------------------------------------------------
info "Checking prerequisites..."

MISSING=()

if ! command -v cargo &>/dev/null; then
    MISSING+=("cargo — install from https://rustup.rs")
fi

if ! command -v git &>/dev/null; then
    MISSING+=("git — https://git-scm.com/downloads")
fi

if ! command -v evtest &>/dev/null; then
    MISSING+=("evtest — sudo pacman -S evtest (Arch) / sudo apt install evtest (Debian/Ubuntu)")
fi

if ! systemctl --user &>/dev/null; then
    MISSING+=("systemctl --user — systemd is required")
fi

if [ ${#MISSING[@]} -gt 0 ]; then
    error "Missing prerequisites:"
    for m in "${MISSING[@]}"; do
        echo "  - $m"
    done
    exit 1
fi
info "All prerequisites found."

# ------------------------------------------------------------------
# 3. Keyboard setup
# ------------------------------------------------------------------
info "Running keyboard setup..."
if [ ! -f "$PROJECT_DIR/scripts/setup-keyboard.sh" ]; then
    error "scripts/setup-keyboard.sh not found"
    exit 1
fi
bash "$PROJECT_DIR/scripts/setup-keyboard.sh"

# ------------------------------------------------------------------
# 4. Configure .env
# ------------------------------------------------------------------
info "Configuring .env..."

# KEYBOARD_DEVICE was set by setup-keyboard.sh, read it back
if [ ! -f "$ENV_FILE" ]; then
    error ".env not found after keyboard setup"
    exit 1
fi

# Add GIT_DIR if not present
if ! grep -q "^GIT_DIR=" "$ENV_FILE" 2>/dev/null; then
    echo "GIT_DIR=.local/state/tracker_data/" >> "$ENV_FILE"
    info "GIT_DIR=.local/state/tracker_data/ added to .env"
fi

# Prompt for URL if not present
if ! grep -q "^URL=" "$ENV_FILE" 2>/dev/null; then
    echo ""
    read -r -p "Enter GitHub remote URL (e.g. git@github.com:user/tracker_data.git): " github_url
    github_url="${github_url%%#*}"
    github_url="${github_url%"${github_url##*[![:space:]]}"}"
    if [ -z "$github_url" ]; then
        error "URL cannot be empty"
        exit 1
    fi
    if [[ "$github_url" != git@* ]] && [[ "$github_url" != https://* ]]; then
        error "URL must start with 'git@' or 'https://'"
        exit 1
    fi
    echo "URL=$github_url" >> "$ENV_FILE"
    info "URL added to .env"
fi

info ".env is configured:"
while IFS= read -r line; do
    echo "  $line"
done < "$ENV_FILE"

# ------------------------------------------------------------------
# 5. Build
# ------------------------------------------------------------------
info "Building tracker (cargo build --release)..."
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

# ------------------------------------------------------------------
# 6. Install binary
# ------------------------------------------------------------------
info "Installing binary to $BIN_PATH..."
mkdir -p "$BIN_DIR"
cp "$PROJECT_DIR/target/release/tracker" "$BIN_PATH"
chmod +x "$BIN_PATH"

if ! echo "$PATH" | grep -qF "$BIN_DIR"; then
    warn "$BIN_DIR is not in \$PATH. Add this to your shell rc file:"
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ------------------------------------------------------------------
# 7. git init (run from project dir so .env is found)
# ------------------------------------------------------------------
info "Initializing git repo..."
cd "$PROJECT_DIR"
if ! "$BIN_PATH" init; then
    error "tracker init failed. Check your .env and GitHub remote."
    exit 1
fi

# ------------------------------------------------------------------
# 8. Create systemd user service
# ------------------------------------------------------------------
info "Creating systemd user service..."

mkdir -p "$SERVICE_DIR"

cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=tracker — keystroke tracker daemon
After=network.target

[Service]
ExecStart=%h/.local/bin/tracker daemon
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now tracker.service

# ------------------------------------------------------------------
# 9. Verify service is running
# ------------------------------------------------------------------
sleep 1
if systemctl --user is-active --quiet tracker.service; then
    info "tracker.service is active and running."
else
    warn "tracker.service was created but is not running. Check:"
    echo "  systemctl --user status tracker.service"
fi

# ------------------------------------------------------------------
# Done
# ------------------------------------------------------------------
echo ""
info "==========================================="
info "  Installation complete!"
info "==========================================="
echo ""
echo "  Binary:       $BIN_PATH"
echo "  Service:      tracker.service (systemctl --user)"
echo "  Data:         ~/.local/state/tracker_data/"
echo ""
echo "  Commands:"
echo "    tracker status   — view keystroke counts"
echo "    tracker push     — push data to GitHub"
echo "    tracker pull     — pull from GitHub"
echo "    tracker report   — generate daily report"
echo ""
echo "  The daemon is running and will auto-start on login."
