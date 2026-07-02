mod cli;
mod daemon;
mod ipc;
use crate::daemon::tracker::TrackerState;
use anyhow::{Ok, bail};
use chrono::Local;
use clap::Parser;
use ipc::IPCCommand;
use std::process::Command;
use std::{fs, path::Path};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

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
            let tracker_state = get_status().await.expect("get_status failed");
            tracker_state.display();
        }
        KeyPressStatus::Init => {
            println!("Init");
            let url = read_env_key(URL).expect("unable to read URL=");
            if !url.starts_with("git@") && !url.starts_with("https://") {
                anyhow::bail!("URL must start with 'git@' or 'https://'")
            }
            let git_dir_str = read_env_key(GIT_DIR).expect("unable to read GIT_DIR=");
            let home_dir = dirs::home_dir().expect("failed to get home dir");
            let git_dir = home_dir.join(&git_dir_str);

            if git_dir.is_dir() {
                println!("tracker/ folder already exists removing it now");
                std::fs::remove_dir_all(&git_dir).expect("failed to delete /tracker");
            }
            std::fs::create_dir_all(&git_dir).expect("failed to create tracker directory");

            std::fs::write(git_dir.join(".gitignore"), ".env\n*.log\n")
                .expect("failed to write .gitignore");

            let data_dir = git_dir.join("data");
            std::fs::create_dir_all(&data_dir).expect("failed to create data/ directory");

            Command::new("git")
                .arg("init")
                .current_dir(&git_dir)
                .status()?;

            Command::new("git")
                .args(["branch", "-M", "main"])
                .current_dir(&git_dir)
                .status()?;

            Command::new("git")
                .args(["remote", "add", "origin", &url])
                .current_dir(&git_dir)
                .status()?;

            Command::new("git")
                .args(["commit", "--allow-empty", "-m", "initial commit"])
                .current_dir(&git_dir)
                .status()?;

            Command::new("git")
                .args(["push", "-u", "origin", "main"])
                .current_dir(&git_dir)
                .status()?;
        }
        KeyPressStatus::Pull => {
            let home_dir = dirs::home_dir().expect("failed to get home dir");
            let git_dir_str = read_env_key(GIT_DIR).expect("unable to read GIT_DIR=");
            let git_dir = home_dir.join(&git_dir_str);
            pull_repo(&git_dir).expect("pull failed");
        }
        KeyPressStatus::Push => {
            println!("Push");
            //1. read contents from .env.
            let home_dir = dirs::home_dir().expect("failed to get home dir");
            let git_dir_str = read_env_key(GIT_DIR).expect("unable to read GIT_DIR=");
            let git_dir = home_dir.join(&git_dir_str);

            let date = Local::now().format("%Y-%m-%d").to_string();
            // let user = env::var("USER").unwrap();
            let model = fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
                .unwrap_or_else(|_| "Unknown".to_string())
                .trim()
                .replace(' ', "_");
            let git_dir_model = git_dir.join("data").join(date).join(model);
            // let hour_indicator = Local::now().hour() as u8;

            //2. always pull before pushing
            // pull_repo(&git_dir).expect("pull before push failed");

            //3. get Status
            let tracker_state = get_status().await.expect("get_status failed");

            //4. read the current json inside git_dir
            //4.1 if no json then export to json
            if !git_dir_model.join("keystrokes.json").exists() {
                tracker_state
                    .export_to_json(&git_dir_model, true)
                    .expect("export to json failed");
            }
            // 4.2 else add the state to json and export new added json (which is also export_to_json with same file name)
            else {
                let stored_state_string = fs::read_to_string(git_dir_model.join("keystrokes.json"))
                    .expect("failed to read keystrokes.json to string");
                let mut stored_tracker_state: TrackerState =
                    serde_json::from_str(&stored_state_string)
                        .expect("failed to create TrackerState struct from string");
                stored_tracker_state
                    .add_jsons(&tracker_state)
                    .expect("failed to update current TrackerState");
                stored_tracker_state
                    .export_to_json(&git_dir_model, false)
                    .expect("export to json failed");
            }

            //5. push
            // let commit_name = Local::now().to_string();
            // let msg = format!("push keystrokes from {}", commit_name);
            //
            // Command::new("git")
            //     .args(["add", "-A"])
            //     .current_dir(&git_dir)
            //     .status()?;
            //
            // Command::new("git")
            //     .args(["commit", "-m", &msg])
            //     .current_dir(&git_dir)
            //     .status()?;
            //
            // Command::new("git")
            //     .args(["push", "-u", "origin", "main"])
            //     .current_dir(&git_dir)
            //     .status()
            //     .expect("gir push failed");
            //
            //6. reset TrackerState after successfull push
            reset_tracker().await.expect("unable to reset tracker");
        }
        _ => {
            bail!("This command is not supported {:?}", parsed_command.get)
        }
    }
    Ok(())
}

async fn reset_tracker() -> anyhow::Result<()> {
    ensure_daemon_running().await.expect("daemon failed to run");
    let socket_path = get_socket().expect("failed to get socket");
    let stream = UnixStream::connect(socket_path.as_path())
        .await
        .expect("failed to connect to socket");
    let (_reader, mut writer) = stream.into_split();
    let command = IPCCommand {
        action: "Reset".to_string(),
    };
    let serialized_command =
        serde_json::to_string(&command).expect("failed to convert struct IPCCommand into string");
    let _size = writer
        .write(serialized_command.as_bytes())
        .await
        .expect("failed to write");
    Ok(())
}

async fn get_status() -> anyhow::Result<TrackerState> {
    ensure_daemon_running().await.expect("daemon failed to run");
    let socket_path = get_socket().expect("failed to get socket");
    let stream = UnixStream::connect(socket_path.as_path())
        .await
        .expect("failed to connect to socket");
    let (mut reader, mut writer) = stream.into_split();
    // request the status
    let command = IPCCommand {
        action: "Read".to_string(),
    };
    let serialized_command =
        serde_json::to_string(&command).expect("failed to convert struct IPCCommand into string");
    let _size = writer
        .write(serialized_command.as_bytes())
        .await
        .expect("failed to write");
    // get the status
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .expect("failed to read length prefix");

    let len = u32::from_le_bytes(len_buf) as usize;
    let mut data_buf = vec![0u8; len];
    reader
        .read_exact(&mut data_buf)
        .await
        .expect("failed to read data");

    let tracker_state: TrackerState =
        serde_json::from_slice(&data_buf).expect("decoding to tracker_state failed");
    Ok(tracker_state)
}

fn pull_repo(git_dir: &Path) -> anyhow::Result<()> {
    if !git_dir.join(".git").exists() {
        anyhow::bail!(
            "No .git directory found at {}. Run `init` first.",
            git_dir.display()
        );
    }

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(git_dir)
        .output()?;

    let branch = String::from_utf8(output.stdout).expect("invalid utf8 from git");
    let branch = branch.trim();

    let upstream_status = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .current_dir(git_dir)
        .status()?;

    if !upstream_status.success() {
        Command::new("git")
            .args(["branch", "-u", &format!("origin/{}", branch)])
            .current_dir(git_dir)
            .status()?;
    }

    let status = Command::new("git")
        .args(["pull"])
        .current_dir(git_dir)
        .status()?;

    if !status.success() {
        anyhow::bail!("git pull failed");
    }

    Ok(())
}

async fn ensure_daemon_running() -> anyhow::Result<()> {
    //TODO: need to ping to check the connection. This is just a placeholder
    let socket_path = get_socket().expect("failed to get socket");
    if !socket_path.exists() {
        anyhow::bail!("Socket does not exist. Ensure program is run correctly")
    }
    Ok(())
}
