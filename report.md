# Keystroke Tracker — Code Review Report

_Reviewed by hand (no Rust toolchain available in the review environment, so this is a
manual read rather than a `clippy` run)._

This is a genuinely nice project — the privacy-by-design aggregation, the versioned
schema, and the systemd/git sync story are all thoughtful. What follows is an honest,
constructive review, starting with the active-session logic that was the main concern.

---

## 1. The active-session / lock logic (main concern)

The instinct — "count a 3s tick as active iff not asleep AND not locked" — is sound.
The **sleep** half (login1 `PrepareForSleep`) is correct and idiomatic. The **lock**
half has a real structural bug.

### The bug: `is_locked` is edge-triggered, not level-triggered

```rust
// mod.rs:228-237
let mut status = is_locked_clone.lock().await;
if let Some(prev) = &prev_line
    && prev == "activewindow>>,"
    && line == "activewindowv2>>"
{
    *status = true;
} else {
    *status = false;   // <-- resets on EVERY other event line
}
prev_line = Some(line);
```

`is_locked` is overwritten on *every single line* the socket emits. It is `true` only
during the one loop iteration immediately after the empty-focus pair; the very next
event — whatever it is — flips it back to `false`. So it behaves like an *edge* ("the
last event was empty-focus"), not a *level* ("we are currently locked").

It happens to *appear* to work only because when you lock and sit idle, Hyprland emits
no further events, so the `true` sticks. But:

- **False negative (undercounts locked as active):** hyprlock opens a layer surface, and
  Hyprland fires `openlayer>>`, `closelayer>>`, `monitoradded`, `screencast>>`, etc. Any
  such event arriving *while locked* flips `is_locked` back to `false`, and the next 3s
  tick counts you as active even though the screen is locked.
- **False positive (undercounts active as locked):** focusing an empty workspace, closing
  your last window, or toggling a special workspace *also* produces `activewindow>>,` +
  `activewindowv2>>`. You get flagged "locked" while actually sitting at an empty desktop,
  and your active time silently stops counting.

So the metric being measured is corrupted in both directions, and the failures are
invisible.

### The fix

Two robust options, in order of preference:

**(a) Poll `pgrep -x hyprlock`** — this is literally comment 5.1 in the source, and it's
more reliable than the socket heuristic that actually shipped. The process exists ⇔
you are locked. One source of truth, zero false positives from empty workspaces:

```rust
let is_locked_clone = Arc::clone(&is_locked);
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let locked = tokio::process::Command::new("pgrep")
            .args(["-x", "hyprlock"])
            .status().await
            .map(|s| s.success())
            .unwrap_or(false);
        *is_locked_clone.lock().await = locked;
    }
});
```

**(b) If staying event-driven on socket2**, key off the layer surface instead of
active-window, and make it a proper level with explicit set/clear:

```rust
if let Some(ns) = line.strip_prefix("openlayer>>") {
    if ns.contains("hyprlock") { *status = true; }
} else if let Some(ns) = line.strip_prefix("closelayer>>") {
    if ns.contains("hyprlock") { *status = false; }
}
// otherwise: leave *status unchanged  <-- the crucial difference
```

The one-line takeaway: **a lock state must persist until an explicit unlock event; never
reset it on unrelated events.**

### Two smaller issues in the same task

- **`tokio::time::interval`'s first tick fires immediately**, so at startup 3s is added
  before any 3s has elapsed. And the default `MissedTickBehavior::Burst` means that after
  a suspend/resume the timer can burst several catch-up ticks, over-counting on wake.
  Consider `interval.set_missed_tick_behavior(MissedTickBehavior::Skip)` and/or tracking
  elapsed wall-clock deltas instead of assuming exactly 3s.
- `is_locked` and `is_asleep` are locked **twice** per tick (once for the `println!`, once
  for the `if`). Harmless, but each could be read once into a `bool`. Also that `println!`
  every 3s will spam the journal forever — fine for debugging, drop it for release.

---

## 3. The `.expect()` / `panic!` culture — the biggest thing between this and "top 99%"

This is the theme that will most move the code from "works" to "excellent." Right now
nearly every fallible call ends in `.expect()`, `.unwrap()`, or `panic!`. Consequences:

- **`read_env_key` panics instead of returning its `Result`** (mod.rs:50-51): it's
  declared `-> anyhow::Result<String>` but `panic!`s on a missing file, defeating the
  point of the return type.
- **`fetch_events().unwrap()`** in both blocking loops (mod.rs:94, 133): unplug the
  keyboard/mouse and that thread panics and dies *silently* — the daemon keeps running but
  stops tracking that device forever, with no log and no recovery. For a background daemon
  this is the failure mode that matters most. Better: a `match`/`?` that logs and, ideally,
  tries to reopen the device.
- The pervasive `.expect()` in async tasks: a panic in a `tokio::spawn` task just vanishes
  into the void (the task aborts, nothing else notices).

For 99%-tier code the rule of thumb is: **`?` for anything that can fail at runtime;
reserve `panic!`/`expect` for genuine invariant violations (bugs).** With `anyhow` already
in the deps, propagating is nearly free, and context is cheap: `.context("reading DPI from
.env")?`. Spawned tasks should log their error on exit rather than `.expect()`.

---

## 4. Idiomatic / polish notes (the last mile to 99%)

- **`Result<()>` that never fails.** `add_jsons` and `export_to_json` return
  `anyhow::Result<()>` but only ever `Ok(())` (all their internal failures are
  `.expect()`ed). Either make them infallible (`-> ()`) or actually propagate with `?`.
  Pick one; mixed signals read as unfinished.
- **`export_to_json` ignores its own errors** — `create_dir_all(...).expect(...)`,
  `fs::write(...).expect(...)` inside a function that returns `Result`. Convert to `?`.
- **Naming:** `let unix_stream = connect_to_socket()...` is actually a `UnixListener`, not
  a stream (mod.rs:84). Small, but names like this are what reviewers notice.
- **Two `Mutex` types in one module** — `std::sync::Mutex` for the tracker,
  `futures_util::lock::Mutex` for the flags. Using the std mutex for the tracker is
  *correct* per the Tokio tutorial (never held across `.await`). Consider using
  `tokio::sync::Mutex` for the flags rather than pulling in `futures_util`'s, for
  consistency — or better, since the flags are just booleans, `Arc<AtomicBool>` removes the
  locking entirely and reads cleaner.
- **`HashMap<u8, ...>` keyed by hour** — fine, but the values are never ordered when
  `display()` runs; `for (hour, ...) in &self.keyboard_state` iterates in random order.
  Sort the keys before printing so status output is stable/readable. A `BTreeMap<u8, _>`
  gives free ordering and is a natural fit for a small dense key space like 0–23.
- **`or_insert_with(HashMap::new)` → `or_default()`** (tracker.rs:107); and the keyboard
  `.and_modify(|c| *c += 1).or_insert(1)` is exactly `*entry.or_insert(0) += 1`.
- **Unused/loose ends:** `Reconfigure { target }` ignores `target` (there's even a
  `//TODO: need to handle the target`), the `Test` command and `_REPO_NAME` const are dead,
  and the temp socket name `temptracker.sock` still has its "change this back" TODO. None
  are bugs, but a reviewer scanning for craft will tally them.
- **The test in `tracker.rs`** asserts `1 == 1` — it's a scratch/print test, not a real
  one. A real round-trip test (`add_jsons` correctness, serialize→deserialize equality)
  would be far more valuable and is exactly the kind of thing that signals senior-level
  Rust.
- **`chrono` hour boundaries:** `Local::now()` is fine, but if a tick lands on an hour
  boundary a few seconds get misattributed. Not worth fixing; just be aware.

---

## Summary — priorities

| Priority | Item |
|---|---|
| **1 — correctness** | Rewrite lock detection as a persistent level (prefer `pgrep -x hyprlock`, or key off `openlayer/closelayer>>hyprlock`). Current logic mis-measures active time in both directions. |
| **2 — perf** | Hoist `read_env_key(MOUSE_DPI)` out of the mouse loop (it's a disk read per event, under a lock). |
| **3 — robustness** | Replace `.expect()`/`unwrap()`/`panic!` on runtime-fallible calls with `?`+context; make `fetch_events()` failures logged/recoverable, not silent thread death. |
| **4 — polish** | Fix the mislabeled distance math, unify IPC framing, make `Result` functions actually propagate, `BTreeMap` for hours, real tests, clear the dead TODOs. |

The bones here are good — the architecture (blocking input threads + async IPC/signal
tasks + a shared mutex-guarded state) is the right shape, and the comments show real
understanding of *why* each piece exists rather than cargo-culting it. Close the
lock-detection gap and stamp out the panic-happy error handling and this genuinely reads
like top-tier hobby Rust.
