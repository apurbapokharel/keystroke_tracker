mod cli;
mod daemon;

use clap::Parser;
use std::{path::PathBuf, process::Command};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tracker::daemon::tracker::TrackerState;

use cli::{Args, KeyPressStatus};
use daemon::{get_socket, read_env_key};

const URL: &str = "URL=";
const _REPO_NAME: &str = "REPO_NAME=";
const GIT_DIR: &str = "GIT_DIR=";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let parsed_command = Args::parse();
    match parsed_command.get {
        KeyPressStatus::Daemon => {
            daemon::run().await.expect("daemon failed to run");
        }
        KeyPressStatus::Status => {
            ensure_daemon_running().await.expect("daemon failed to run");
            let socket_path = get_socket().expect("failed to get socket");
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

            let tracker_state: TrackerState =
                serde_json::from_slice(&data_buf).expect("decoding to tracker_state failed");
            tracker_state.display();
        }
        KeyPressStatus::Init => {
            println!("Init");
            //1. read contents from .env.
            let url = read_env_key(URL).expect("unable to read URL=");
            let git_dir_str = read_env_key(GIT_DIR).expect("unable to read GIT_DIR=");
            let home_dir = dirs::home_dir().expect("failed to get home dir");
            let git_dir = home_dir.join(&git_dir_str);

            //2. check if .git exists if then throw error
            println!("home_dir = {:?}", home_dir);
            println!("git_dir_str = {:?}", git_dir_str);
            println!("git_dir = {:?}", git_dir);
            if git_dir.is_dir() {
                println!("tracker/ folder already exists removing it now");
                std::fs::remove_dir_all(&git_dir).expect("failed to delete /tracker");
            }
            std::fs::create_dir(&git_dir).expect("failed to create new /tracker dir");
            // 4. git init
            Command::new("git")
                .arg("init")
                .current_dir(&git_dir)
                .status()?;

            // 4. git remote add origin <url>
            Command::new("git")
                .args(["remote", "add", "origin", &url])
                .current_dir(&git_dir)
                .status()?;

            // create initial empty commit
            Command::new("git")
                .args(["commit", "--allow-empty", "-m", "initial commit"])
                .current_dir(&git_dir)
                .status()?;

            // optional: push
            Command::new("git")
                .args(["push", "-u", "origin", "master"])
                .current_dir(&git_dir)
                .status()?;
        }
        KeyPressStatus::Pull => {
            println!("Pull");
        }
        KeyPressStatus::Push => {
            println!("Push");
            //1. read contents from .env.
            //2. always pull before pushing
            //3. get Status
            //4. create a json
            //5. add to appropriate folder structure
            //6. push
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
    let socket_path = get_socket().expect("failed to get socket");
    if !socket_path.exists() {
        anyhow::bail!("Socket does not exist. Ensure program is run correctly")
    }
    Ok(())
}
