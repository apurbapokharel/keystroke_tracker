# Keystroke Tracker — Design Document

## Overview
A Rust-based background daemon that tracks keyboard keystrokes on Linux machines, stores the data locally as versioned CSV files, and optionally syncs to a GitHub repository for centralized access across multiple machines.

## Supported Platforms
- **Linux only** (reads `/dev/input/event*` — kernel input subsystem)
- Works on **both Wayland and X11** (display-server agnostic)
- Currently Linux-only; no Windows/macOS support planned

## Machines
Each machine is identified by its hostname (output of `hostname`). Examples:
- `apu` (personal machine)
- `dell-work` (work machine)

Data directories are separated by machine name under the data tree.

## Data Storage Format (v1)

### Directory layout
```
data/
└── v1/
    ├── apu/
    │   ├── 2026-06-18.csv
    │   └── 2026-06-18_keys.csv
    ├── dell-work/
    │   ├── 2026-06-18.csv
    │   └── 2026-06-18_keys.csv
    └── combined/
        └── 2026-06-18.csv
```

### Hourly summary: `{date}.csv`
```csv
hour,total_keystrokes,active_minutes,unique_keys
9,1250,45,28
10,800,30,22
11,0,0,0
```
- One row per hour (0–23)
- `active_minutes` = number of minutes in that hour where at least one key was pressed
- `total_keystrokes` = raw count of all key-down events in that hour

### Per-key breakdown: `{date}_keys.csv`
```csv
key,total_presses
a,1520
Enter,890
Space,2100
Backspace,340
```
- One row per unique key pressed that day
- Keys are recorded by their Linux keycode name (e.g., `KEY_A`, `KEY_ENTER`)

### Combined output: `v1/combined/{date}.csv`
```csv
hour,personal,work,grand_total
9,1250,980,2230
10,800,1100,1900
```
- Merged from all machines for a given date
- The `merge` CLI command produces this

### Versioning
- Future schema changes use a new directory: `v2/`, `v3/`, etc.
- Each version is self-describing; old data is never migrated automatically
- The current active version is always `v1`

### Storage overhead estimate
- Per-key file: ~100 rows/day × ~30 bytes ≈ 3 KB/day/machine
- Hourly file: 24 rows/day × ~30 bytes ≈ 720 bytes/day/machine
- **Total: ~4 KB/day/machine → ~1.5 MB/year/machine**

## How Tracking Works

1. **Input source**: Reads directly from `/dev/input/event*` using the Linux input subsystem (`libevdev` or raw `read()` syscalls)
2. **Filtering**: Only captures `EV_KEY` events with `value == 1` (key press, not release or repeat)
3. **Aggregation**: Counts keystrokes per (hour, key) in memory
4. **Periodic flush**: Writes to CSV every hour (and on SIGTERM/SIGINT)
5. **Startup**: On daemon start, reads existing CSV for today (if any) and resumes counting

## Permissions
- User must be in the `input` group to read `/dev/input/event*`
- Run once: `sudo usermod -aG input $USER && logout & log back in`
- Alternative: run the daemon as root (not recommended)

## Privacy
- Only counts of key presses are stored; **no raw key sequences, no timing between keys, no typed text**
- Per-key breakdown tracks *how many times* each key was pressed, not *in what order*
- This is privacy-safe by design — you cannot reconstruct what was typed

## CLI Commands

```
keystroke-tracker daemon            # Start the tracking daemon (foreground)
keystroke-tracker daemon --detach   # Start as a background process
keystroke-tracker status            # Show live stats since start of day
keystroke-tracker summary --date 2026-06-18  # Print daily summary
keystroke-tracker merge --date 2026-06-18 [--output ./combined.csv]
keystroke-tracker version           # Print version info
```

## Auto-start

### systemd user service (`~/.config/systemd/user/keystroke-tracker.service`)
- Runs on user login
- Can be controlled with:
  - `systemctl --user start keystroke-tracker`
  - `systemctl --user stop keystroke-tracker`
  - `systemctl --user status keystroke-tracker`
  - `systemctl --user disable keystroke-tracker` (permanently stop on boot)

### Desktop autostart (`~/.config/autostart/keystroke-tracker.desktop`)
- Fallback if systemd user services are unavailable

## GitHub Sync

### What "sync" means
The tracking data lives on each machine's local filesystem. Optionally, the user can configure a GitHub repository URL. An end-of-day script (or the daemon itself) will:
1. `git add` the day's data files
2. `git commit -m "daily data YYYY-MM-DD <machine>"`
3. `git push` to the configured remote

This provides:
- **Backup** of all keystroke data
- **Centralized merge** — pull both machines' data to any machine
- **History** — every day's data is a commit you can diff

### Configuration
The GitHub repo URL is stored in a config file (`~/.config/keystroke-tracker/config.toml`):
```toml
git_remote = "git@github.com:username/keystroke-data.git"
git_branch = "main"
machine_name = "apu"        # optional, defaults to hostname
data_dir = "data"           # optional, defaults to ./data
```

### Git repo setup (one-time per machine)
```bash
# Create a private repo on GitHub, then:
git init ~/keystroke-data
cd ~/keystroke-data
git remote add origin git@github.com:username/keystroke-data.git
git push -u origin main
```

## Architecture
```
┌─────────────────────┐     ┌──────────────────┐     ┌───────────────┐
│  /dev/input/event*  │ ──> │  tracker.rs      │ ──> │  storage.rs   │
│  (kernel input)     │     │  (read + count)   │     │  (CSV write)  │
└─────────────────────┘     └──────────────────┘     └───────┬───────┘
                                                              │
                                                              ▼
┌─────────────────────┐     ┌──────────────────┐     ┌───────────────┐
│  GitHub (optional)  │ <── │  sync.rs         │ <── │  data/v1/...  │
│  git push           │     │  (git commit+push)│     │  (CSV files)  │
└─────────────────────┘     └──────────────────┘     └───────────────┘
```

## Open Questions / Future Considerations
- **WPM calculation**: Can be derived from per-minute data (would need a v2 with minute-level buckets)
- **Per-application tracking**: Could track which X11/Wayland window was focused (complex)
- **Idle detection**: Don't count keystrokes after N minutes of inactivity? Or track total active time?
- **Encryption**: Data is plaintext; could encrypt CSVs before git push
- **GUI dashboard**: Future web or TUI app to view stats across machines
