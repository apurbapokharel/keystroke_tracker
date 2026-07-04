# tracker

A lightweight keystroke tracker daemon for Linux.

Tracks per-key and per-hour keystroke counts from `/dev/input/event*`, stores
them as JSON, and optionally syncs to a GitHub repository.

## Supported OS

Linux (Wayland and X11). Reads input devices directly — no display server
dependencies.

## Prerequisites

- Rust (https://rustup.rs)
- Git installed and configured to be able to clone, pull, push.
- evtest (`sudo pacman -S evtest` or `sudo apt install evtest`)
- systemd (user services)
- User must be in the `input` group

## Install

```bash
git clone https://github.com/your-username/tracker.git
cd tracker
./install.sh
```

The install script will:
1. Add you to the `input` group (reboot required)
2. Guide you through keyboard device selection
3. Build the binary
4. Create a systemd user service
5. Initialize the git repository

## Usage

```
tracker status   — view current keystroke counts
tracker init     — initialize git repository (done by install.sh)
tracker push     — push data to GitHub
tracker pull     — pull from GitHub
```

The daemon runs in the background as a systemd user service and auto-starts
on login.

## Data

Stored in `~/.local/state/tracker_data/data/{date}/{hostname}/keystrokes.json`.

## License

MIT
