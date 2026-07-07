#!/bin/sh

script_dir="$(dirname "$0")"
project_dir="$(cd "$script_dir/.." && pwd)"
env_file="$project_dir/.env"

if ! command -v evtest >/dev/null 2>&1; then
    echo "evtest is required. Install it:"
    echo "  sudo pacman -S evtest        (Arch)"
    echo "  sudo apt install evtest      (Debian/Ubuntu)"
    exit 1
fi

while true; do
    echo ""
    echo "Launching evtest. Select a device from the list,"
    echo "press some keys to verify it's your keyboard,"
    echo "then press Ctrl+C to exit evtest."
    echo ""
    printf "Press Enter to continue..."
    read -r _
    echo ""

    sudo evtest || true

    echo ""
    printf "Did you see your keystrokes in the logs? (y/n): "
    read -r seen

    case "$seen" in
        [yY]|[yY][eE][sS])
            ;;
        *)
            echo "OK, let's try another device."
            continue
            ;;
    esac

    echo ""
    printf "Enter the event number you tested (e.g., 8): "
    read -r event_num

    device="/dev/input/event${event_num}"
    if [ ! -e "$device" ]; then
        echo "Device $device does not exist." >&2
        continue
    fi

    if grep -q "^KEYBOARD_DEVICE=" "$env_file" 2>/dev/null; then
        sed -i "s|^KEYBOARD_DEVICE=.*|KEYBOARD_DEVICE=$device|" "$env_file"
    else
        echo "KEYBOARD_DEVICE=$device" >> "$env_file"
    fi
    echo ""
    echo "Saved $device to $env_file"
    echo "Continuing with setup..."
    exit 0
done
