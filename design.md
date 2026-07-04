# Keystroke Tracker — Design Document

## Overview
A Rust-based background daemon that tracks keyboard keystrokes on Linux machines, stores the data locally as versioned JSON files, and optionally syncs to a GitHub repository for centralized access across multiple machines.

## Supported Platforms
- **Linux only** (reads `/dev/input/event*` — kernel input subsystem)
- Works on **both Wayland and X11** (display-server agnostic)
- Currently Linux-only; no Windows/macOS support planned

## Machines
Each machine is identified by its device-name 

Data directories are separated by machine name under the data tree.

## Data Storage Format (v1)

### Directory layout
```
data/
└── date in YYYY-MM-DD/
    ├── device-name-1/
    │   ├── keystrokes.json
    ├── device-name-2/
    │   └── keystrokes.json 
    └── total.json
```

### Versioning
- Future schema changes use a new version in the json: `v2/`, `v3/`, etc.
- The current active version is always `v1`

### Storage overhead estimate
- Per-key file: ~100 rows/day × ~30 bytes ≈ 3 KB/day/machine
- Hourly file: 24 rows/day × ~30 bytes ≈ 720 bytes/day/machine
- **Total: ~4 KB/day/machine → ~1.5 MB/year/machine**

## How Tracking Works

1. **Input source**: Reads directly from `/dev/input/event*` using the Linux input subsystem (`libevdev` or raw `read()` syscalls)
2. **Filtering**: Only captures `EV_KEY` events with `value == 1` (key press, not release or repeat)
3. **Aggregation**: Counts keystrokes per (hour, key) in memory
4. **Periodic flush**: Writes to JSON on command

## Permissions
- User must be in the `input` group to read `/dev/input/event*`
- Run once: `sudo usermod -aG input $USER && logout & log back in`

## Privacy
- Only counts of key presses are stored; **no raw key sequences, no timing between keys, no typed text**
- Per-key breakdown tracks *how many times* each key was pressed, not *in what order*
- This is privacy-safe by design — you cannot reconstruct what was typed

## CLI Commands

```
keystroke-tracker daemon                      # Start the tracking daemon (this is done by systemctl at system boot)
keystroke-tracker status                      # Show live stats since start of day
keystroke-tracker summary --date 2026-06-18   # Print daily summary
keystroke-tracker version                     # Print version info
```

## Auto-start

### systemd user service (`~/.config/systemd/user/keystroke-tracker.service`)
- Runs on user login
- Can be controlled with:
  - `systemctl --user start tracker.service`
  - `systemctl --user status tracker.service`

## GitHub Sync
Use github for saving the strokes.
