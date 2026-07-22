#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
ENV_FILE="$PROJECT_DIR/.env"
BIN_DIR="$HOME/.local/bin"
BIN_PATH="$BIN_DIR/tracker"
SERVICE_DIR="$HOME/.config/systemd/user"
SERVICE_PATH="$SERVICE_DIR/tracker.service"
NOTIFY_SERVICE_PATH="$SERVICE_DIR/tracker-failure-notify.service"

TRACKER_CONFIG_DIR="$HOME/.config/tracker"

FORCE=0
RECONFIGURE=0
UPDATE=0
for arg in "$@"; do
    [ "$arg" = "--force" ] && FORCE=1
    [ "$arg" = "--reconfigure" ] && RECONFIGURE=1
    [ "$arg" = "--update" ] && UPDATE=1
done

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Write (or overwrite) both systemd user units and reload the daemon so the
# definitions take effect. Idempotent — safe to call from install and update.
# Overwrites the main unit file; customize via `systemctl --user edit` drop-ins,
# which live in a separate .d/ dir and survive this.
write_systemd_units() {
    mkdir -p "$SERVICE_DIR"

    cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=tracker — keystroke tracker daemon
After=network.target
# Fire the notifier when the service gives up restarting (see StartLimit* below).
OnFailure=tracker-failure-notify.service
# Restart=always self-heals brief crashes without bothering the user; only after
# 5 crashes within 5 min does systemd mark the unit "failed" and trigger OnFailure.
StartLimitIntervalSec=300
StartLimitBurst=5

[Service]
ExecStart=%h/.local/bin/tracker daemon
WorkingDirectory=%h/.config/tracker
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
EOF

    # Companion unit: raises a critical desktop notification when tracker fails.
    # It is a --user unit, so it inherits the session bus and busctl --user can
    # reach the notification daemon.
    cat > "$NOTIFY_SERVICE_PATH" <<'EOF'
[Unit]
Description=Notify when the tracker daemon fails

[Service]
Type=oneshot
ExecStart=busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications Notify susssasa{sv}i tracker 0 dialog-error "tracker daemon failed" "It stopped and systemd gave up restarting it. Run: systemctl --user status tracker.service" 0 1 urgency y 2 0
EOF

    systemctl --user daemon-reload
}

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
# 3z. Update mode — rebuild + redeploy binary and refresh units (safe to re-run)
#     Does NOT touch git or the input group, so it never wipes tracked data.
#     Re-writes the systemd units (idempotent) so unit changes ship via update.
#     Use this after changing the source. Overwrites hand-edits to the main unit
#     (use `systemctl --user edit` drop-ins to customize — those survive).
# ------------------------------------------------------------------
if [ "$UPDATE" -eq 1 ]; then
    info "Update mode — rebuilding and redeploying binary..."
    cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

    info "Installing binary to $BIN_PATH..."
    mkdir -p "$BIN_DIR"
    cp "$PROJECT_DIR/target/release/tracker" "$BIN_PATH.new"
    chmod +x "$BIN_PATH.new"
    mv -f "$BIN_PATH.new" "$BIN_PATH"

    # Keep the daemon's copy of .env in sync in case keys changed.
    if [ -f "$ENV_FILE" ]; then
        mkdir -p "$TRACKER_CONFIG_DIR"
        cp "$ENV_FILE" "$TRACKER_CONFIG_DIR/.env"
        info "Synced .env to $TRACKER_CONFIG_DIR/.env"
    fi

    info "Refreshing systemd units..."
    write_systemd_units

    info "Restarting tracker.service..."
    systemctl --user restart tracker.service || true

    sleep 1
    if systemctl --user is-active --quiet tracker.service; then
        info "tracker.service is active and running the new build."
    else
        warn "tracker.service is not active. Check:"
        echo "  systemctl --user status tracker.service"
        echo "  journalctl --user -u tracker.service -n 50"
    fi
    exit 0
fi

# ------------------------------------------------------------------
# 3a. Reconfigure mode — keyboard + mouse device setup
# ------------------------------------------------------------------
if [ "$RECONFIGURE" -eq 1 ]; then
    info "Reconfigure mode — updating keyboard and mouse devices..."
    bash "$PROJECT_DIR/scripts/setup-keyboard.sh"
    bash "$PROJECT_DIR/scripts/setup-mouse.sh"
    mkdir -p "$TRACKER_CONFIG_DIR"
    cp "$ENV_FILE" "$TRACKER_CONFIG_DIR/.env"
    info "Restarting tracker.service..."
    systemctl --user restart tracker.service || true
    info "Reconfigure complete."
    exit 0
fi

# ------------------------------------------------------------------
# 3b. Check if already running
# ------------------------------------------------------------------
if systemctl --user is-active --quiet tracker.service 2>/dev/null; then
    if [ "$FORCE" -ne 1 ]; then
        error "tracker.service is already configured and running."
        echo ""
        echo "  To reinstall, stop and disable first:"
        echo "    systemctl --user stop tracker.service"
        echo "    systemctl --user disable tracker.service"
        echo ""
        echo "  Or re-run with --force to stop and reinstall:"
        echo "    ./install.sh --force"
        exit 1
    else
        warn "Stopping existing tracker.service..."
        systemctl --user stop tracker.service
        systemctl --user disable tracker.service
    fi
fi

# ------------------------------------------------------------------
# 4. Keyboard + mouse setup
# ------------------------------------------------------------------
info "Running keyboard setup..."
if [ ! -f "$PROJECT_DIR/scripts/setup-keyboard.sh" ]; then
    error "scripts/setup-keyboard.sh not found"
    exit 1
fi
bash "$PROJECT_DIR/scripts/setup-keyboard.sh"

info "Running mouse setup..."
if [ ! -f "$PROJECT_DIR/scripts/setup-mouse.sh" ]; then
    error "scripts/setup-mouse.sh not found"
    exit 1
fi
bash "$PROJECT_DIR/scripts/setup-mouse.sh"

# ------------------------------------------------------------------
# 5. Configure .env
# ------------------------------------------------------------------
info "Configuring .env..."

# KEYBOARD_DEVICE / MOUSE_DEVICE / MOUSE_DPI were set by the setup scripts
if [ ! -f "$ENV_FILE" ]; then
    error ".env not found after device setup"
    exit 1
fi

# Add GIT_DIR if not present
if ! grep -q "^GIT_DIR=" "$ENV_FILE" 2>/dev/null; then
    echo "GIT_DIR=.local/state/tracker_data/" >> "$ENV_FILE"
    info "GIT_DIR=.local/state/tracker_data/ added to .env"
fi

# Add PROJECT_DIR if not present
if ! grep -q "^PROJECT_DIR=" "$ENV_FILE" 2>/dev/null; then
    echo "PROJECT_DIR=$PROJECT_DIR" >> "$ENV_FILE"
    info "PROJECT_DIR=$PROJECT_DIR added to .env"
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
# 6. Build
# ------------------------------------------------------------------
info "Building tracker (cargo build --release)..."
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

# ------------------------------------------------------------------
# 7. Install binary
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
# 8. git init (run from project dir so .env is found)
# ------------------------------------------------------------------
info "Initializing git repo..."
cd "$PROJECT_DIR"
if ! "$BIN_PATH" init; then
    error "tracker init failed. Check your .env and GitHub remote."
    exit 1
fi

# ------------------------------------------------------------------
# 9. Copy .env to config dir for daemon access
# ------------------------------------------------------------------
mkdir -p "$TRACKER_CONFIG_DIR"
cp "$ENV_FILE" "$TRACKER_CONFIG_DIR/.env"
info "Copied .env to $TRACKER_CONFIG_DIR/.env"

# ------------------------------------------------------------------
# 10. Create systemd user service
# ------------------------------------------------------------------
info "Creating systemd user service..."

write_systemd_units
systemctl --user enable --now tracker.service

# ------------------------------------------------------------------
# 11. Verify service is running
# ------------------------------------------------------------------
sleep 1
if systemctl --user is-active --quiet tracker.service; then
    info "tracker.service is active and running."
else
    warn "tracker.service was created but is not running. Check:"
    echo "  systemctl --user status tracker.service"
fi

# ------------------------------------------------------------------
# 12. Done
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
echo "    tracker status   — view unpushed counts (-d for a full breakdown)"
echo "    tracker push     — push data to GitHub"
echo "    tracker pull     — pull from GitHub"
echo "    tracker report   — generate daily report"
echo ""
echo "  The daemon is running and will auto-start on login."
