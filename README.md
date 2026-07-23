# tracker

A lightweight input-activity tracker daemon for Linux.

Reads directly from `/dev/input/event*` and aggregates, per hour:

- **Keystrokes** — per-key press counts
- **Mouse** — left / right / middle click counts, scrolls, and pointer travel (inches)
- **Active screen time** — seconds the session was awake and unlocked

Data is stored locally as versioned JSON and can optionally be synced to a
GitHub repository.

![CLI Demo](assets/demo.gif)

### Web UI

An interactive heatmap dashboard is available at
[keystroke_ui](https://github.com/apurbapokharel/keystroke_ui):

<video src="assets/ui.mp4" controls width="720"></video>

### Privacy

Only *counts* are stored — no raw key sequences, no timing between keys, no
typed text. You cannot reconstruct what was typed from the data.

## Supported OS

Linux (Wayland and X11). Reads input devices directly — no display server
dependencies.

**Active-session (lock) detection** picks a backend at runtime:

- **Hyprland** — watches the Hyprland event socket (hyprlock doesn't report to
  logind, so lock state has to come from the compositor).
- **Ubuntu / GNOME and other logind desktops** — watches the per-session
  `LockedHint` property on `org.freedesktop.login1`. On GNOME, make sure the
  screensaver lock is enabled so `LockedHint` actually flips:
  ```bash
  gsettings set org.gnome.desktop.screensaver lock-enabled true
  ```
- **Anything else** — if no lock backend can be set up, the daemon logs and keeps
  running; active time then falls back to sleep-only rather than crashing.

**Sleep detection** uses the standard `org.freedesktop.login1` `PrepareForSleep`
D-Bus signal on every desktop.

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
tracker status             — summary table of everything not yet pushed
tracker status --detailed  — per-hour and per-key breakdown (-d)
tracker push               — aggregate the current session into JSON and push to GitHub
tracker pull               — pull the latest data from GitHub
tracker reconfigure        — re-run device setup, then restart the service
```

`status` reports **live daemon state only** — one row per date that has not been
pushed yet, oldest first. Anything above today is a day that was tracked but
never pushed. History that has already been pushed lives in the data repo and is
what the dashboard reads.

```
DATE          KEYS  CLICKS   INCHES  SCROLLS   ACTIVE
2026-07-20  12,431   2,104  1,204.5      883   6h 12m
2026-07-21   8,002   1,551    893.1      402    4h 3m
-----------------------------------------------------
TOTAL       20,433   3,655  2,097.6    1,285  10h 15m
```

Columns size themselves to the widest value, so the table stays aligned however
large the numbers get. The TOTAL row is omitted when there is only one date —
it would just repeat it.

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

The daemon keeps **one set of counters per calendar date**, not one running
total. Every event is filed under the date it happened on, so forgetting to push
before midnight no longer folds yesterday's activity into today — each day still
lands in its own file whenever you get around to pushing.

Each `push` pulls first, then merges every unpushed date into that date's file
(creating it if absent), commits them all as one commit, pushes, and resets the
live counters.

If a push fails part-way — a file it cannot write, a rejected commit — the data
repo is rolled back to the last commit before anything is reset. Without that,
the counters would still be in memory *and* half-written to disk, and the next
push would count them twice. A failure at the network step is different: the
commit already exists locally, so the counters are cleared and the next push
carries that commit along.

Because the counters live in memory, a reboot or a `systemctl --user restart`
drops whatever has not been pushed. **Run `tracker push` before updating or
rebooting** if you care about the current session.

## Failure notifications

Because the daemon runs in the background, failures are surfaced as desktop
notifications (via the `org.freedesktop.Notifications` D-Bus service) so a broken
tracker doesn't go unnoticed:

- **A single input device stops** (e.g. keyboard unplugged) — the daemon keeps
  running but that tracker thread has stopped. It logs to the journal and raises
  a notification immediately.
- **The whole daemon crashes repeatedly** — systemd's `Restart=always` self-heals
  brief blips silently; only if it crash-loops past the start limit (5 times in
  5 minutes) does the `tracker-failure-notify.service` `OnFailure=` unit fire a
  critical notification.

Notifications are best-effort: if no session bus or notification daemon is
available, the failure is still recorded in the journal
(`journalctl --user -u tracker.service`).

## License

MIT
