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

