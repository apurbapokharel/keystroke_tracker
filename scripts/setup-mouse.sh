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
    echo "move the mouse and click to verify it's your mouse,"
    echo "then press Ctrl+C to exit evtest."
    echo ""
    printf "Press Enter to continue..."
    read -r _
    echo ""

    sudo evtest || true

    echo ""
    printf "Did you see your mouse movement/clicks in the logs? (y/n): "
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
    printf "Enter the event number you tested (e.g., 5): "
    read -r event_num

    device="/dev/input/event${event_num}"
    if [ ! -e "$device" ]; then
        echo "Device $device does not exist." >&2
        continue
    fi

    if grep -q "^MOUSE_DEVICE=" "$env_file" 2>/dev/null; then
        sed -i "s|^MOUSE_DEVICE=.*|MOUSE_DEVICE=$device|" "$env_file"
    else
        echo "MOUSE_DEVICE=$device" >> "$env_file"
    fi
    echo ""
    echo "Saved $device to $env_file"

    # DPI is needed to convert raw REL_X/REL_Y counts into inches of travel.
    # Check your mouse's spec sheet (e.g. 800, 1600). Defaults to 800.
    echo ""
    printf "Enter your mouse DPI (press Enter for 800): "
    read -r dpi
    case "$dpi" in
        ''|*[!0-9]*)
            dpi="800"
            echo "Using default DPI: $dpi"
            ;;
    esac

    if grep -q "^MOUSE_DPI=" "$env_file" 2>/dev/null; then
        sed -i "s|^MOUSE_DPI=.*|MOUSE_DPI=$dpi|" "$env_file"
    else
        echo "MOUSE_DPI=$dpi" >> "$env_file"
    fi
    echo "Saved MOUSE_DPI=$dpi to $env_file"
    echo "Continuing with setup..."
    exit 0
done
