pub mod tracker;

use anyhow::Context;
use anyhow::bail;
use chrono::Timelike;
use chrono::prelude::*;
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode};
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
    tokio::task::spawn_blocking(move || {
        loop {
            for event in mouse_device.fetch_events().unwrap() {
                // println!("mouse event destructured {:?}", event.destructure());
                // let hour_indicator = Local::now().hour() as u8;
                let mut tracker_state = tracker_write_2
                    .data
                    .lock()
                    .expect("unable to get tracker_state mutex lock");
                let mouse_dpi = read_env_key(MOUSE_DPI).expect("error reading .env");

                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    match key_type {
                        KeyCode::BTN_RIGHT => tracker_state.mouse_state.right_click += 1,
                        KeyCode::BTN_LEFT => tracker_state.mouse_state.left_click += 1,
                        KeyCode::BTN_MIDDLE => tracker_state.mouse_state.middle_click += 1,
                        _ => {}
                    }
                } else if let EventSummary::RelativeAxis(_ev, event_code, value) =
                    event.destructure()
                {
                    match event_code {
                        RelativeAxisCode::REL_X | RelativeAxisCode::REL_Y => {
                            let euc_distance = ((value * value) as f32).sqrt();
                            tracker_state.mouse_state.mouse_inches += euc_distance
                                / mouse_dpi
                                    .parse::<f32>()
                                    .expect("failed to convert dpi to float");
                        }
                        RelativeAxisCode::REL_WHEEL_HI_RES => {
                            tracker_state.mouse_state.mouse_scrolls += 1;
                        }
                        _ => {}
                    }
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
    let mut buf: Vec<u8> = vec![0u8; 1024];
    let n = reader
        .read(&mut buf)
        .await
        .expect("cannot read from stream");
    let buf_to_string = String::from_utf8(buf[..n].to_vec()).expect("failed to convert");
    let command: IPCCommand =
        serde_json::from_str(&buf_to_string).expect("failed to convert to struct IPCCommand");
    if command.action.as_str() == "Read" {
        let state = tracker
            .data
            .lock()
            .expect("unable to get mutex lock")
            .clone();
        let serialized = serde_json::to_string(&state).expect("unable to serialize tracker_state");
        let len = (serialized.len() as u32).to_le_bytes();
        writer
            .write_all(&len)
            .await
            .expect("failed to write length prefix");
        writer
            .write_all(serialized.as_bytes())
            .await
            .expect("failed to write into stream");
    } else if command.action.as_str() == "Reset" {
        tracker
            .data
            .lock()
            .expect("unable to get mutex lock")
            .reset();
    } else {
        bail!("Unknown command {:?}", command.action.as_str())
    }

    Ok(())
}
