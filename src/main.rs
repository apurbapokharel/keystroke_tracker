mod cli;
mod daemon;

use clap::Parser;
use std::collections::HashMap;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tracker::daemon::tracker::TrackerState;

use cli::{Args, KeyPressStatus};
use daemon::get_socket;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let parsed_command = Args::parse();
    println!("Command {:?}", &parsed_command);
    match parsed_command.get {
        KeyPressStatus::Daemon => {
            daemon::run().await.expect("daemon failed to run");
        }
        KeyPressStatus::Status => {
            println!("status called");
            ensure_daemon_running().await.expect("daemon failed to run");
            let socket_path = get_socket().await.expect("failed to get socket");
            let mut stream = UnixStream::connect(socket_path.as_path())
                .await
                .expect("failed to connect to socket");
            let mut len_buf = [0u8; 4];
            stream
                .read_exact(&mut len_buf)
                .await
                .expect("failed to read length prefix");
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut data_buf = vec![0u8; len];
            stream
                .read_exact(&mut data_buf)
                .await
                .expect("failed to read data");
            let tracker_state_date: HashMap<u8, HashMap<u16, u32>> =
                serde_json::from_slice(&data_buf).expect("decoding to hashmap failed");
            let tracker_state = TrackerState {
                count_freq: tracker_state_date,
            };
            tracker_state.display();
        }
        KeyPressStatus::Push => {}
            println!("Push");
        _ => {
            println!("not implemented yet")
        } // KeyPressStatus::Status => {}
          // KeyPressStatus::Push => {}
          // KeyPressStatus::Pull => {}
          // KeyPressStatus::Sync => {}
    }

    Ok(())
    // let device_path = read_env_key().expect("error reading .env");
    // println!("Using device: {}", device_path);
    // let mut device = Device::open(device_path)?;
    // loop {
    //     for event in device.fetch_events().unwrap() {
    //         if let EventSummary::Key(_ev, key_type, 1) = event.destructure() {
    //             println!("Key {:?} was pressed", key_type);
    //         }
    //     }
    // }
}

async fn ensure_daemon_running() -> anyhow::Result<()> {
    //TODO: need to ping to check the connection. This is just a placeholder
    let socket_path = get_socket().await.expect("failed to get socket");
    if !socket_path.exists() {
        anyhow::bail!("Socket does not exist. Ensure program is run correctly")
    }
    Ok(())
}
