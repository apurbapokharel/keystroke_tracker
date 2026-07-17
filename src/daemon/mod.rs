pub mod tracker;

use anyhow::Context;
use anyhow::bail;
use chrono::Timelike;
use chrono::prelude::*;
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode, SynchronizationCode};
use futures_util::lock::Mutex;
use futures_util::stream::StreamExt;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use zbus::{Connection, proxy};

use crate::daemon::tracker::Tracker;
use crate::ipc::IPCCommand;

// TODO: change this back to tracker.sock
const SOCKET_NAME: &str = "temptracker.sock";
const KEYBOARD_DEVICE: &str = "KEYBOARD_DEVICE=";
const MOUSE_DEVICE: &str = "MOUSE_DEVICE=";
const MOUSE_DPI: &str = "MOUSE_DPI=";
const HYPR_SIG: &str = "HYPRLAND_INSTANCE_SIGNATURE";
const XDG_RUNTIME: &str = "XDG_RUNTIME_DIR";

#[proxy(
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    interface = "org.freedesktop.login1.Manager"
)]

trait Login1Manager {
    // Defines signature for D-Bus signal named `PrepareForSleep`
    #[zbus(signal)]
    fn prepare_for_sleep(&self, status: bool);
}

fn get_env_path() -> PathBuf {
    let config_path = dirs::config_dir()
        .map(|p| p.join("tracker/.env"))
        .filter(|p| p.exists());
    config_path.unwrap_or_else(|| PathBuf::from(".env"))
}

pub fn read_env_key(key: &str) -> anyhow::Result<String> {
    let env_path = get_env_path();
    let content = std::fs::read_to_string(&env_path)
        .unwrap_or_else(|_| panic!(".env not found at {}", env_path.display()));
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
    let run_path = dirs::runtime_dir().expect("error getting runtime dir");
    Ok(run_path.join(SOCKET_NAME))
}

fn connect_to_socket() -> anyhow::Result<UnixListener> {
    let socket_path = get_socket().expect("failed to get socket");
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!("failed to remove stale socket at {}", socket_path.display())
        })?;
    }

    let unix_listener =
        UnixListener::bind(socket_path.as_path()).expect("Failed to establish unix stream");
    Ok(unix_listener)
}

pub async fn run() -> anyhow::Result<()> {
    // 1. establish a universal socket for writing and reading.
    let unix_stream = connect_to_socket().expect("failed to connect to unix socket");

    // 2. run an endless loop that processes the keys pressed.
    let keyboard_path = read_env_key(KEYBOARD_DEVICE).expect("error reading .env");
    println!("Using device: {}", keyboard_path);
    let mut keyboard_device = Device::open(keyboard_path)?;
    let tracker: Arc<Tracker> = Arc::new(Tracker::new());
    let tracker_write = Arc::clone(&tracker);
    tokio::task::spawn_blocking(move || {
        loop {
            for event in keyboard_device.fetch_events().unwrap() {
                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    let hour_indicator = Local::now().hour() as u8;
                    let key_code = format!("{:?}", KeyCode::new(key_type.code()));

                    let mut tracker_state = tracker_write
                        .data
                        .lock()
                        .expect("unable to get tracker_state mutex lock");

                    tracker_state
                        .keyboard_state
                        .entry(hour_indicator)
                        .or_default()
                        .entry(key_code)
                        .and_modify(|count| *count += 1)
                        .or_insert(1);

                    // println!(
                    //     "Key {:?}, keycode {:?} was pressed {:?}",
                    //     key_type,
                    //     key_type.code(),
                    //     tracker_state
                    //         .keyboard_state
                    //         .get(&hour_indicator)
                    //         .and_then(|inner| inner.get(&key_code))
                    //         .unwrap_or(&0)
                    // );
                }
            }
        }
    });

    // 3. run an endless loop that process the mouse events.
    let mouse_path = read_env_key(MOUSE_DEVICE).expect("error reading .env");
    let mut mouse_device = Device::open(mouse_path)?;
    let tracker_write_2 = Arc::clone(&tracker);
    let mouse_dpi: f64 = read_env_key(MOUSE_DPI)
        .expect("error reading .env")
        .parse()
        .expect("MOUSE_DPI is not a valid number");
    tokio::task::spawn_blocking(move || {
        // AI: evdev delivers one physical mouse report as a burst of REL_X / REL_Y
        // events terminated by a SYN_REPORT. We accumulate the axes of the
        // current report and only commit distance on that boundary, so we can
        // add the true Euclidean segment length sqrt(dx^2 + dy^2) rather than
        // the Manhattan sum |dx| + |dy| that per-axis handling would give.
        let mut dx: i32 = 0;
        let mut dy: i32 = 0;
        loop {
            for event in mouse_device.fetch_events().unwrap() {
                match event.destructure() {
                    EventSummary::Key(_ev, key_type, 1) => {
                        let mut tracker_state = tracker_write_2
                            .data
                            .lock()
                            .expect("unable to get tracker_state mutex lock");
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
                        tracker_write_2
                            .data
                            .lock()
                            .expect("unable to get tracker_state mutex lock")
                            .mouse_state
                            .mouse_scrolls += 1;
                    }
                    // Report boundary: commit one Euclidean segment (in inches
                    // of physical desk travel = raw counts / DPI).
                    EventSummary::Synchronization(_ev, SynchronizationCode::SYN_REPORT, _) => {
                        if dx != 0 || dy != 0 {
                            let inches = (dx as f64).hypot(dy as f64) / mouse_dpi;
                            tracker_write_2
                                .data
                                .lock()
                                .expect("unable to get tracker_state mutex lock")
                                .mouse_state
                                .mouse_inches += inches;
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
    let is_asleep: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let is_locked: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    // refer to https://z-galaxy.github.io/zbus/client.html#signals for this code
    let connection = Connection::system().await?;
    let login_proxy = Login1ManagerProxy::new(&connection).await?;
    let mut new_stream = login_proxy.receive_prepare_for_sleep().await?;

    let is_asleep_clone = Arc::clone(&is_asleep);
    tokio::task::spawn(async move {
        while let Some(msg) = new_stream.next().await {
            let args: PrepareForSleepArgs = msg.args().expect("Error parsing message");
            let mut status = is_asleep_clone.lock().await;
            *status = args.status;
        }
    });

    // spwan a task that monitors hyprlock status.
    let is_locked_clone = Arc::clone(&is_locked);
    let xdg_runtime_dir = std::env::var(XDG_RUNTIME).expect("unable to read env");
    let hypr_instance_signature = std::env::var(HYPR_SIG).expect("unable to read env");
    let socket_path = PathBuf::new()
        .join(&xdg_runtime_dir)
        .join("hypr")
        .join(&hypr_instance_signature)
        .join(".socket2.sock");
    let hypr_stream = UnixStream::connect(socket_path.as_path())
        .await
        .expect("failed to connect to hypr socket");
    // NOTE: this is where reading line by line comes into play,
    // because i don't know the size of the entire payload and i also don't need to.
    // jhola also has this same code but since that was all vibes I could never figure why it was used.
    let reader = BufReader::new(hypr_stream);
    let mut lines = reader.lines();
    let mut prev_line: Option<String> = None;
    tokio::task::spawn(async move {
        while let Some(line) = lines
            .next_line()
            .await
            .expect("reading line by line failed from stream")
        {
            // println!("line {:?}", line);
            let mut status = is_locked_clone.lock().await;
            if let Some(prev) = &prev_line
                && prev == "activewindow>>,"
                && line == "activewindowv2>>"
            {
                *status = true;
            } else {
                *status = false;
            }
            prev_line = Some(line);
        }
    });

    // timer to increment counter iff  both true
    let is_locked_clone_2 = Arc::clone(&is_locked);
    let is_asleep_clone_2 = Arc::clone(&is_asleep);
    let tracker_write_clone = Arc::clone(&tracker);
    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
        loop {
            interval.tick().await;
            println!(
                "lock status is {} and sleep status is {}",
                *is_locked_clone_2.lock().await,
                *is_asleep_clone_2.lock().await
            );
            if !*is_locked_clone_2.lock().await && !*is_asleep_clone_2.lock().await {
                let hour_indicator = Local::now().hour() as u8;
                let mut tracker_state = tracker_write_clone
                    .data
                    .lock()
                    .expect("unable to get tracker_state mutex lock");

                tracker_state
                    .display_state
                    .entry(hour_indicator)
                    .and_modify(|count| *count += 3)
                    .or_insert(3);
            }
        }
    });

    // 5. handle new connections to this socket.
    loop {
        let (stream, _addr) = unix_stream
            .accept()
            .await
            .expect("unable to fetch incoming request");
        let tracker_read = Arc::clone(&tracker);
        tokio::spawn(async move {
            handle_request(tracker_read, stream)
                .await
                .expect("failed to handle incoming requests");
        });
    }
    // Ok(())
}

pub fn reconfigure() -> anyhow::Result<()> {
    let project_dir = read_env_key("PROJECT_DIR=")?;
    let script = PathBuf::from(&project_dir).join("scripts/setup-keyboard.sh");

    let status = Command::new("bash").arg(&script).status()?;
    if !status.success() {
        bail!("keyboard setup failed");
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
                let state = tracker
                    .data
                    .lock()
                    .expect("tracker mutex poisoned")
                    .clone();
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
            tracker
                .data
                .lock()
                .expect("tracker mutex poisoned")
                .reset();
        }
        other => bail!("unknown command {other:?}"),
    }

    Ok(())
}
