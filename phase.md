Here's the markdown:

```markdown
## Phased Implementation Plan

### Phase 1 — Core Daemon (the hard part)
- Set up a Rust project with a `Cargo.toml`
- Read raw events from `/dev/input/event*` — open each device file, loop on `read()`, parse the `input_event` struct (`type == EV_KEY`, `value == 1`)
- Handle multiple device files concurrently (you'll have keyboard + maybe others) — `epoll` or threads per device
- In-memory aggregation: a `HashMap<(u8 hour, u16 keycode), u32 count>`
- Map Linux keycode numbers to human-readable names (there's a known table for this)
- Graceful shutdown on SIGTERM/SIGINT — flush before exit

### Phase 2 — Storage
- Write hourly CSV (`{date}.csv`) and per-key CSV (`{date}_keys.csv`) on flush
- Flush triggers: every hour on the hour, and on shutdown signal
- On startup, read today's existing CSVs and seed the in-memory map (so a restart doesn't lose the day's data)
- Determine the data directory: `~/.local/share/keystroke-tracker/data/v1/{hostname}/`

### Phase 3 — CLI
- Subcommand parsing (use `clap`)
- `daemon` — starts the loop (foreground); `--detach` forks to background
- `status` — reads today's CSV and prints a live summary
- `summary --date` — pretty-print a past day
- `version`

### Phase 4 — Auto-start
- Write a systemd user service file (`keystroke-tracker.service`) and document how to install it
- Optionally a `.desktop` autostart file as fallback

### Phase 5 — GitHub Sync
- Config file parsing (`config.toml` with `serde` + `toml`)
- A `sync` CLI subcommand that shells out to `git add`, `git commit`, `git push`
- An end-of-day trigger (either a systemd timer, or the daemon notices midnight rollover)

### Phase 6 — Merge / Combined View
- `merge --date` command: reads `data/v1/*/YYYY-MM-DD.csv` across all machine directories, sums by hour, writes `combined/{date}.csv`
- Useful after pulling from GitHub on one machine to see cross-machine totals
```
