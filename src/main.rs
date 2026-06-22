mod cli;
mod daemon;

use clap::Parser;
use std::{any, fs::exists};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

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
            let mut buf = vec![0u8; 1024];
            let socket_path = get_socket().await.expect("failed to get socket");
            let mut stream = UnixStream::connect(socket_path.as_path())
                .await
                .expect("failed to connect to socket");
            let count = stream
                .read(&mut buf)
                .await
                .expect("failed to read from unix stream");
            println!("read length {}", count);
        }
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
