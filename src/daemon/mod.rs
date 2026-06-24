pub mod tracker;

use anyhow::Context;
use chrono::Timelike;
use chrono::prelude::*;
use evdev::{Device, EventSummary};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

use crate::daemon::tracker::Tracker;

const SOCKET_NAME: &str = "tracker.sock";

pub async fn get_socket() -> anyhow::Result<PathBuf> {
    let run_path = dirs::runtime_dir().expect("error getting runtime dir");
    Ok(run_path.join(SOCKET_NAME))
}

async fn connect_to_socket() -> anyhow::Result<UnixListener> {
    let socket_path = get_socket().await.expect("failed to get socket");
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).with_context(|| {
            format!("failed to remove stale socket at {}", socket_path.display())
        })?;
    }

    let unix_listener =
        UnixListener::bind(socket_path.as_path()).expect("Failed to establish unix stream");
    Ok(unix_listener)
}

fn read_env_key() -> anyhow::Result<String> {
    let content = std::fs::read_to_string(".env").expect(".env does not exist");
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("KEYBOARD_DEVICE=") {
            let path = value.trim();
            if !path.is_empty() {
                return Ok(path.to_string());
            }
        }
    }
    Err(anyhow::anyhow!("KEYBOARD_DEVICE not found in .env"))
}

pub async fn run() -> anyhow::Result<()> {
    // 1. establish a universal socket for writing and reading.
    let unix_stream = connect_to_socket()
        .await
        .expect("failed to connect to unix socket");

    // 2. run an endless loop that processes the keys pressed.
    let device_path = read_env_key().expect("error reading .env");
    println!("Using device: {}", device_path);
    let mut device = Device::open(device_path)?;
    let tracker: Arc<Tracker> = Arc::new(Tracker::new());
    let tracker_write = Arc::clone(&tracker);
    tokio::task::spawn_blocking(move || {
        loop {
            for event in device.fetch_events().unwrap() {
                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    let hour_indicator = Local::now().hour() as u8;
                    let key_code = key_type.code();

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
        let (mut stream, _addr) = unix_stream
            .accept()
            .await
            .expect("unable to fetch incoming request");
        let tracker_read = Arc::clone(&tracker);
        let tracker_state = tracker_read
            .data
            .lock()
            .expect("unable to get mutex lock")
            .clone();
        tokio::spawn(async move {
            let serialized =
                serde_json::to_string(&tracker_state).expect("unable to serialize tracker_state");
            let len = (serialized.len() as u32).to_le_bytes();
            stream
                .write_all(&len)
                .await
                .expect("failed to write length prefix");
            stream
                .write_all(serialized.as_bytes())
                .await
                .expect("failed to write into stream");
        });
    }
    // Ok(())
}
