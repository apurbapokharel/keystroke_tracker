use anyhow::Context;
use evdev::{Device, EventSummary};
use std::sync::{Arc, Mutex};
use std::{os::unix, path::PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};

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
    let count = Arc::new(Mutex::new(0));
    tokio::task::spawn_blocking(move || {
        loop {
            for event in device.fetch_events().unwrap() {
                if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
                    let mut count = count.lock().expect("unable to get mutex lock");
                    *count += 1;
                    println!("Key {:?} was pressed {:?}", key_type, count);
                }
            }
        }
    });

    // 3. also handle new connections to this socket.
    println!("outside");
    loop {
        println!("accept request");
        let (mut stream, _addr) = unix_stream
            .accept()
            .await
            .expect("unable to fetch incoming request");
        // let count = Arc::clone(&count);
        tokio::spawn(async move {
            // process(stream).await;
            stream
                .write_all(b"hello")
                .await
                .expect("failed to write into steam");
        });
    }
    // Ok(())
}

// async fn process(stream: UnixStream) {
//     println!("inside process")
// }
