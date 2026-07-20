pub mod tracker;
pub mod zzbus;

use anyhow::Context;
use anyhow::bail;
use chrono::Timelike;
use chrono::prelude::*;
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode, SynchronizationCode};
use futures_util::stream::StreamExt;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use zbus::Connection;

use crate::cli::ReconfigureTarget;
use crate::daemon::tracker::Tracker;
use crate::daemon::zzbus::*;
use crate::ipc::IPCCommand;

// TODO: change this back to tracker.sock
const SOCKET_NAME: &str = "temptracker.sock";
const KEYBOARD_DEVICE: &str = "KEYBOARD_DEVICE=";
const MOUSE_DEVICE: &str = "MOUSE_DEVICE=";
const MOUSE_DPI: &str = "MOUSE_DPI=";
const HYPR_SIG: &str = "HYPRLAND_INSTANCE_SIGNATURE";
const XDG_RUNTIME: &str = "XDG_RUNTIME_DIR";
const XDG_SESSION_ID: &str = "XDG_SESSION_ID";

fn get_env_path() -> PathBuf {
    let config_path = dirs::config_dir()
        .map(|p| p.join("tracker/.env"))
        .filter(|p| p.exists());
    config_path.unwrap_or_else(|| PathBuf::from(".env"))
}

pub fn read_env_key(key: &str) -> anyhow::Result<String> {
    let env_path = get_env_path();
    let content = std::fs::read_to_string(&env_path)
        .with_context(|| format!(".env not found at {}", env_path.display()))?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(key) {
            let path = value.trim();
            if !path.is_empty() {
                return Ok(path.to_string());
            }
        }
    }
    Err(anyhow::anyhow!(format!("{} not found", key)))
}

pub fn get_socket() -> anyhow::Result<PathBuf> {
    let run_path = dirs::runtime_dir().context("could not determine XDG runtime dir")?;
    Ok(run_path.join(SOCKET_NAME))
}

fn connect_to_socket() -> anyhow::Result<UnixListener> {
    let socket_path = get_socket()?;
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!("failed to remove stale socket at {}", socket_path.display())
        })?;
    }

    let unix_listener = UnixListener::bind(socket_path.as_path())
        .with_context(|| format!("failed to bind unix socket at {}", socket_path.display()))?;
    Ok(unix_listener)
}

pub async fn run() -> anyhow::Result<()> {
    // 1. establish a universal socket for writing and reading.
    let listener = connect_to_socket().context("failed to set up ipc socket")?;

    // 2. run an endless loop that processes the keys pressed.
    let keyboard_path = read_env_key(KEYBOARD_DEVICE).context("reading KEYBOARD_DEVICE")?;
    println!("Using device: {}", keyboard_path);
    let mut keyboard_device = Device::open(keyboard_path)?;
    let tracker: Arc<Tracker> = Arc::new(Tracker::new());

    // Background task that surfaces daemon failures as desktop notifications.
    // The blocking input threads below cannot `.await`, so they push error
    // strings through this channel instead of notifying directly.
    let notifier = spawn_notifier();

    let tracker_write = Arc::clone(&tracker);
    let notifier_kbd = notifier.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            let events = match keyboard_device.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    // Device read failed (typically unplugged). Log, notify the
                    // user, and stop this tracker instead of dying silently.
                    let msg = format!("keyboard tracking stopped: {e}");
                    eprintln!("{msg}");
                    let _ = notifier_kbd.send(msg);
                    break;
                }
            };
            for event in events {
                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    let hour_indicator = Local::now().hour() as u8;
                    let key_code = format!("{:?}", KeyCode::new(key_type.code()));

                    let mut tracker_state = tracker_write.state();

                    *tracker_state
                        .keyboard_state
                        .entry(hour_indicator)
                        .or_default()
                        .entry(key_code)
                        .or_insert(0) += 1;
                }
            }
        }
    });

    // 3. run an endless loop that process the mouse events.
    let mouse_path = read_env_key(MOUSE_DEVICE).context("reading MOUSE_DEVICE")?;
    let mut mouse_device = Device::open(mouse_path)?;
    let tracker_write_2 = Arc::clone(&tracker);
    let notifier_mouse = notifier.clone();
    let mouse_dpi: f64 = read_env_key(MOUSE_DPI)
        .context("reading MOUSE_DPI")?
        .parse()
        .context("MOUSE_DPI is not a valid number")?;
    tokio::task::spawn_blocking(move || {
        // AI: evdev delivers one physical mouse report as a burst of REL_X / REL_Y
        // events terminated by a SYN_REPORT. We accumulate the axes of the
        // current report and only commit distance on that boundary, so we can
        // add the true Euclidean segment length sqrt(dx^2 + dy^2) rather than
        // the Manhattan sum |dx| + |dy| that per-axis handling would give.
        let mut dx: i32 = 0;
        let mut dy: i32 = 0;
        loop {
            let events = match mouse_device.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    let msg = format!("mouse tracking stopped: {e}");
                    eprintln!("{msg}");
                    let _ = notifier_mouse.send(msg);
                    break;
                }
            };
            for event in events {
                match event.destructure() {
                    EventSummary::Key(_ev, key_type, 1) => {
                        let mut tracker_state = tracker_write_2.state();
                        match key_type {
                            KeyCode::BTN_RIGHT => tracker_state.mouse_state.right_click += 1,
                            KeyCode::BTN_LEFT => tracker_state.mouse_state.left_click += 1,
                            KeyCode::BTN_MIDDLE => tracker_state.mouse_state.middle_click += 1,
                            _ => {}
                        }
                    }
                    // Buffer the report's axes; don't touch shared state yet.
                    EventSummary::RelativeAxis(_ev, RelativeAxisCode::REL_X, value) => dx += value,
                    EventSummary::RelativeAxis(_ev, RelativeAxisCode::REL_Y, value) => dy += value,
                    EventSummary::RelativeAxis(_ev, RelativeAxisCode::REL_WHEEL_HI_RES, _) => {
                        tracker_write_2.state().mouse_state.mouse_scrolls += 1;
                    }
                    // Report boundary: commit one Euclidean segment (in inches
                    // of physical desk travel = raw counts / DPI).
                    EventSummary::Synchronization(_ev, SynchronizationCode::SYN_REPORT, _) => {
                        if dx != 0 || dy != 0 {
                            let inches = (dx as f64).hypot(dy as f64) / mouse_dpi;
                            tracker_write_2.state().mouse_state.mouse_inches += inches;
                            dx = 0;
                            dy = 0;
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    // 4. run a seperate task that tracks the active scesion time.
    //NOTE: this is very very complicated than i thought it would be.
    //1. i use hyperland. and in order to detect sleep i can subscribe to dbus signal PrepareToSleep
    //2. figuring this out was no easy task, even with constant mentoring for ai it took me long to
    //   undertand all of this.
    //3. no 2) just solves sleep problem but what if we lock without sleeping that should also not
    //   be counted as a active time.
    //4. this is where hyperland bites back, hyperland uses hyprlock which does not throw the
    //   underlying dbus's LockedHint signal so that does not change on lock and hence it does not help.
    //5. the work around has two options and this is all mentoring of AI i would not be able to figure this out in
    //   this little time,
    //5.1 use pgrep -x hyprlock to see how it works if needed: while true; do pgrep -x hyprlock; echo "---"; sleep 1; done
    //5.2 monitor hyperland socket
    //So, this is my implementation idea so far. Will run this through claude and see if i get
    //better recommendation.

    // spwan a task that subscribes to the dbus PrepareToSleep signal using zbus.
    // These are plain status flags shared across tasks — an AtomicBool needs no
    // lock and no `.await` to read or write, so it's both simpler and cheaper
    // than a mutex. Relaxed ordering is correct: nothing depends on these
    // becoming visible in step with any other memory.
    let is_asleep = Arc::new(AtomicBool::new(false));
    let is_locked = Arc::new(AtomicBool::new(false));
    // refer to https://z-galaxy.github.io/zbus/client.html#signals for this code
    let connection = Connection::system().await?;
    let login_proxy = Login1ManagerProxy::new(&connection).await?;
    let mut new_stream = login_proxy.receive_prepare_for_sleep().await?;

    let is_asleep_clone = Arc::clone(&is_asleep);
    tokio::task::spawn(async move {
        while let Some(msg) = new_stream.next().await {
            let parsed: zbus::Result<PrepareForSleepArgs> = msg.args();
            match parsed {
                Ok(args) => is_asleep_clone.store(args.status, Ordering::Relaxed),
                // One malformed signal shouldn't tear down sleep tracking.
                Err(e) => eprintln!("sleep signal: failed to parse args: {e}"),
            }
        }
    });

    // Track lock state with a backend chosen at runtime. hyprlock doesn't report
    // to logind, so on Hyprland we sniff the compositor's event socket; 
    // For other desktop (GNOME/KDE/…) CODE ASSUMES that lock events reports `LockedHint` to logind, 
    // which we watch instead. 
    // If neither can be set up we log and carry on — active time then
    // counts sleep-only rather than taking the whole daemon down. 
    if std::env::var(HYPR_SIG).is_ok() {
        if let Err(e) = spawn_hypr_lock_task(Arc::clone(&is_locked)).await {
            eprintln!("lock: hyprland backend failed: {e:#}; active time = sleep-only");
        }
    } else if let Err(e) = spawn_logind_lock_task(&connection, Arc::clone(&is_locked)).await {
        eprintln!("lock: logind backend failed: {e:#}; active time = sleep-only");
    }

    // timer to increment counter iff  both true
    let is_locked_clone_2 = Arc::clone(&is_locked);
    let is_asleep_clone_2 = Arc::clone(&is_asleep);
    let tracker_write_clone = Arc::clone(&tracker);
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let locked = is_locked_clone_2.load(Ordering::Relaxed);
            let asleep = is_asleep_clone_2.load(Ordering::Relaxed);
            println!("lock status is {locked} and sleep status is {asleep}");
            if !locked && !asleep {
                let hour_indicator = Local::now().hour() as u8;
                let mut tracker_state = tracker_write_clone.state();

                *tracker_state
                    .display_state
                    .entry(hour_indicator)
                    .or_insert(0) += 3;
            }
        }
    });

    // 5. handle new connections to this socket.
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            // A single failed accept shouldn't bring the whole daemon down.
            Err(e) => {
                eprintln!("ipc: failed to accept connection: {e}");
                continue;
            }
        };
        let tracker_read = Arc::clone(&tracker);
        tokio::spawn(async move {
            if let Err(e) = handle_request(tracker_read, stream).await {
                eprintln!("ipc: failed to handle request: {e:#}");
            }
        });
    }
    // Ok(())
}

/// Hyprland lock backend: watch the compositor's event socket (`.socket2.sock`).
/// hyprlock doesn't flip logind's `LockedHint`, so lock state has to come from
/// Hyprland's own event stream. Errors are returned to the caller (which logs
/// and degrades to sleep-only) rather than aborting the daemon.
async fn spawn_hypr_lock_task(is_locked: Arc<AtomicBool>) -> anyhow::Result<()> {
    let xdg_runtime_dir = std::env::var(XDG_RUNTIME).context("reading $XDG_RUNTIME_DIR")?;
    let hypr_instance_signature =
        std::env::var(HYPR_SIG).context("reading $HYPRLAND_INSTANCE_SIGNATURE")?;
    let socket_path = PathBuf::new()
        .join(&xdg_runtime_dir)
        .join("hypr")
        .join(&hypr_instance_signature)
        .join(".socket2.sock");
    let hypr_stream = UnixStream::connect(socket_path.as_path())
        .await
        .with_context(|| format!("connecting to hypr socket at {}", socket_path.display()))?;
    let reader = BufReader::new(hypr_stream);
    let mut lines = reader.lines();
    let mut prev_line: Option<String> = None;

    // NOTE: the alternative was polling `pgrep -x hyprlock` on an interval, but
    // sniffing the socket is event-driven and cheaper.
    // NOTE: this does not work if hyprlock is actived on an empty workspace with no tabs open
    tokio::task::spawn(async move {
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break, // socket closed
                Err(e) => {
                    eprintln!("hypr socket: read failed, stopping lock tracking: {e}");
                    break;
                }
            };
            let locked = if let Some(prev) = &prev_line {
                prev == "activewindow>>," && line == "activewindowv2>>"
            } else {
                false
            };
            is_locked.store(locked, Ordering::Relaxed);
            prev_line = Some(line);
        }
    });
    Ok(())
}

/// logind lock backend for non-Hyprland desktops: watch the per-session
/// `LockedHint` property. A conforming locker flips it on lock/unlock (and it
/// also flips on sleep), which logind surfaces as `PropertiesChanged`. Reuses
/// the daemon's existing system-bus connection.
async fn spawn_logind_lock_task(
    connection: &Connection,
    is_locked: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let manager = Login1ManagerProxy::new(connection).await?;
    // Prefer the graphical session we were launched under (its id is inherited
    // via $XDG_SESSION_ID); otherwise fall back to whichever session owns this
    // process.
    let session_path = match std::env::var(XDG_SESSION_ID) {
        Ok(id) if !id.is_empty() => manager
            .get_session(&id)
            .await
            .with_context(|| format!("resolving session id {id}"))?,
        _ => manager
            .get_session_by_pid(std::process::id())
            .await
            .context("resolving session by pid")?,
    };

    let session = Login1SessionProxy::builder(connection)
        .path(session_path.clone())?
        .build()
        .await
        .with_context(|| format!("building session proxy at {}", session_path.as_str()))?;

    // Seed the current state before streaming changes, so a daemon started while
    // already locked doesn't count that time as active.
    is_locked.store(session.locked_hint().await?, Ordering::Relaxed);

    tokio::task::spawn(async move {
        let mut changes = session.receive_locked_hint_changed().await;
        while let Some(change) = changes.next().await {
            match change.get().await {
                Ok(locked) => is_locked.store(locked, Ordering::Relaxed),
                Err(e) => eprintln!("lock: failed to read LockedHint change: {e}"),
            }
        }
    });
    Ok(())
}

fn run_setup_script(project_dir: &str, script_name: &str, label: &str) -> anyhow::Result<()> {
    let script = PathBuf::from(project_dir).join("scripts").join(script_name);
    let status = Command::new("bash").arg(&script).status()?;
    if !status.success() {
        bail!("{label} setup failed");
    }
    Ok(())
}

/// Re-run device setup. `None` and `Some(All)` reconfigure both devices;
/// `Some(Keyboard)`/`Some(Mouse)` reconfigure just that one.
pub fn reconfigure(target: Option<ReconfigureTarget>) -> anyhow::Result<()> {
    let project_dir = read_env_key("PROJECT_DIR=")?;

    let (keyboard, mouse) = match target.unwrap_or(ReconfigureTarget::All) {
        ReconfigureTarget::Keyboard => (true, false),
        ReconfigureTarget::Mouse => (false, true),
        ReconfigureTarget::All => (true, true),
    };

    if keyboard {
        run_setup_script(&project_dir, "setup-keyboard.sh", "keyboard")?;
    }
    if mouse {
        run_setup_script(&project_dir, "setup-mouse.sh", "mouse")?;
    }

    // Copy updated .env to config dir so systemd daemon picks it up
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("no config dir"))?
        .join("tracker");
    fs::create_dir_all(&config_dir)?;
    fs::copy(
        PathBuf::from(&project_dir).join(".env"),
        config_dir.join(".env"),
    )?;
    println!("Updated {}", config_dir.join(".env").display());

    Ok(())
}

async fn handle_request(tracker: Arc<Tracker>, stream: UnixStream) -> anyhow::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    // Read the u32 little-endian length prefix. EOF here just means the client
    // connected and hung up without sending anything — that is not an error.
    let mut len_buf = [0u8; 4];
    if let Err(e) = reader.read_exact(&mut len_buf).await {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(());
        }
        return Err(e).context("reading request length prefix");
    }
    let len = u32::from_le_bytes(len_buf) as usize;

    // Guard against a bogus/hostile prefix before allocating.
    const MAX_REQUEST: usize = 64 * 1024;
    if len > MAX_REQUEST {
        bail!("request too large: {len} bytes (max {MAX_REQUEST})");
    }

    // Read exactly the advertised body — a single read() may return less.
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .context("reading request body")?;
    let command: IPCCommand =
        serde_json::from_slice(&buf).context("parsing IPCCommand from request")?;

    match command.action.as_str() {
        "Read" => {
            // Scope the lock so the guard is dropped before the .await writes.
            let serialized = {
                let state = tracker.state().clone();
                serde_json::to_string(&state).context("serializing tracker state")?
            };
            writer
                .write_all(&(serialized.len() as u32).to_le_bytes())
                .await
                .context("writing response length prefix")?;
            writer
                .write_all(serialized.as_bytes())
                .await
                .context("writing response body")?;
            writer.flush().await.context("flushing response")?;
        }
        "Reset" => {
            tracker.state().reset();
        }
        other => bail!("unknown command {other:?}"),
    }

    Ok(())
}
