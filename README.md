# tracker

A lightweight input-activity tracker daemon for Linux.

Reads directly from `/dev/input/event*` and aggregates, per hour:

- **Keystrokes** — per-key press counts
- **Mouse** — left / right / middle click counts, scrolls, and pointer travel (inches)
- **Active screen time** — seconds the session was awake and unlocked

Data is stored locally as versioned JSON and can optionally be synced to a
GitHub repository.

### Privacy

Only *counts* are stored — no raw key sequences, no timing between keys, no
typed text. You cannot reconstruct what was typed from the data.

## Supported OS

Linux (Wayland and X11). Reads input devices directly — no display server
dependencies. Active-session (lock) detection is currently Hyprland-specific;
sleep detection uses the standard `org.freedesktop.login1` D-Bus signal.

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
1. Add you to the `input` group (reboot required the first time)
2. Guide you through keyboard/mouse device selection
3. Configure `.env` (device paths, `GIT_DIR`, `PROJECT_DIR`, GitHub `URL`)
4. Build the binary and install it to `~/.local/bin/tracker`
5. Initialize the git data repository
6. Create and start a systemd user service

## `install.sh` flags

The install script is also the update/maintenance entry point. Run it with one
of these flags — with no flag it performs a full first-time install.

| Flag | What it does | When to use |
|------|--------------|-------------|
| *(none)* | Full first-time install: input group, device setup, `.env`, build, **`tracker init`**, systemd service. | The very first setup on a machine. |
| `--update` | Rebuild the binary, reinstall it to `~/.local/bin/tracker`, sync `.env` to `~/.config/tracker/.env`, and restart the service. **Does not touch git, devices, or the systemd unit.** | After changing the source code. Safe to re-run — never wipes tracked data. |
| `--reconfigure` | Re-run keyboard/mouse device setup, sync `.env`, and restart the service. Does **not** rebuild. | A device path changed (e.g. you swapped keyboards). |
| `--force` | Stop/disable an existing service, then run the full first-time install again — **including `tracker init`, which deletes and re-creates the data repo**. | A clean reinstall from scratch. ⚠️ Destroys existing tracked data. |

> **Updating an already-installed machine:** use `./install.sh --update`.
> Do **not** re-run `install.sh` with no flag or `--force` to deploy new code —
> both run `tracker init`, which wipes `GIT_DIR`.

## Usage

```
tracker status        — view current keystroke / mouse / active-time counts
tracker push          — aggregate the current session into JSON and push to GitHub
tracker pull          — pull the latest data from GitHub
tracker reconfigure   — re-run device setup, then restart the service
```

`tracker init` (initialize the git repo) and `tracker daemon` (the background
process) are run for you by `install.sh` and the systemd service — you should
not need to invoke them by hand.

The daemon runs in the background as a systemd user service and auto-starts on
login. Manage it with:

```
systemctl --user status  tracker.service
systemctl --user restart tracker.service
systemctl --user stop    tracker.service
journalctl  --user -u    tracker.service -n 50
```

## Data

Stored as versioned JSON, keyed by date and machine model:

```
~/.local/state/tracker_data/data/{YYYY-MM-DD}/{machine-model}/keystrokes.json
```

Each `push` pulls first, merges the current in-memory session into the existing
file for that date/machine, pushes, and then resets the live counters.

## License

MIT
