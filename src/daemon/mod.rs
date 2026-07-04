pub mod tracker;

use anyhow::Context;
use anyhow::bail;
use chrono::Timelike;
use chrono::prelude::*;
use evdev::{Device, EventSummary, KeyCode};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::daemon::tracker::Tracker;
use crate::ipc::IPCCommand;

const SOCKET_NAME: &str = "tracker.sock";
const KEYBOARD_DEVICE: &str = "KEYBOARD_DEVICE=";

fn get_env_path() -> PathBuf {
    let config_path = dirs::config_dir()
        .map(|p| p.join("tracker/.env"))
        .filter(|p| p.exists());
    config_path.unwrap_or_else(|| PathBuf::from(".env"))
}

pub fn read_env_key(key: &str) -> anyhow::Result<String> {
    let env_path = get_env_path();
    let content =
        std::fs::read_to_string(&env_path).unwrap_or_else(|_| panic!(".env not found at {}", env_path.display()));
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
    let device_path = read_env_key(KEYBOARD_DEVICE).expect("error reading .env");
    println!("Using device: {}", device_path);
    let mut device = Device::open(device_path)?;
    let tracker: Arc<Tracker> = Arc::new(Tracker::new());
    let tracker_write = Arc::clone(&tracker);
    tokio::task::spawn_blocking(move || {
        loop {
            for event in device.fetch_events().unwrap() {
                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    let hour_indicator = Local::now().hour() as u8;
                    let key_code = format!("{:?}", KeyCode::new(key_type.code()));

                    let mut tracker_state = tracker_write
                        .data
                        .lock()
                        .expect("unable to get tracker_state mutex lock");

                    tracker_state
                        .count_freq
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
                    //         .count_freq
                    //         .get(&hour_indicator)
                    //         .and_then(|inner| inner.get(&key_code))
                    //         .unwrap_or(&0)
                    // );
                }
            }
        }
    });

    // 3. also handle new connections to this socket.
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
