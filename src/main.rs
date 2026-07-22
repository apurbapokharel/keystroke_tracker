mod cli;
mod daemon;
mod ipc;
mod render;

use crate::daemon::tracker::TrackerState;
use anyhow::Context;
use anyhow::Ok;
use chrono::Local;
use clap::Parser;
use ipc::IPCCommand;
use std::collections::BTreeMap;
use std::process::Command;
use std::{fs, path::Path};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;

use cli::{Args, TrackerCLI};
use daemon::{get_socket, read_env_key};

const URL: &str = "URL=";
const GIT_DIR: &str = "GIT_DIR=";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let parsed_command = Args::parse();
    match parsed_command.get {
        TrackerCLI::Daemon => {
            daemon::run().await.context("daemon exited with error")?;
        }
        TrackerCLI::Status { detailed } => {
            let tracker_states = get_status().await?;
            if tracker_states.is_empty() {
                println!("Nothing tracked since the last push.");
            } else if detailed {
                render::detailed(&tracker_states);
            } else {
                render::brief(&tracker_states);
            }
        }
        TrackerCLI::Init => {
            println!("Init");
            let url = read_env_key(URL)?;
            if !url.starts_with("git@") && !url.starts_with("https://") {
                anyhow::bail!("URL must start with 'git@' or 'https://'")
            }
            let git_dir_str = read_env_key(GIT_DIR)?;
            let home_dir = dirs::home_dir().context("failed to get home dir")?;
            let git_dir = home_dir.join(&git_dir_str);

            if git_dir.is_dir() {
                println!("tracker/ folder already exists removing it now");
                std::fs::remove_dir_all(&git_dir)
                    .with_context(|| format!("failed to delete {}", git_dir.display()))?;
            }
            std::fs::create_dir_all(&git_dir)
                .with_context(|| format!("failed to create {}", git_dir.display()))?;

            std::fs::write(git_dir.join(".gitignore"), ".env\n*.log\n")
                .context("failed to write .gitignore")?;

            let data_dir = git_dir.join("data");
            std::fs::create_dir_all(&data_dir).context("failed to create data/ directory")?;

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
        TrackerCLI::Pull => {
            let home_dir = dirs::home_dir().context("failed to get home dir")?;
            let git_dir_str = read_env_key(GIT_DIR)?;
            let git_dir = home_dir.join(&git_dir_str);
            pull_repo(&git_dir)?;
        }
        TrackerCLI::Reconfigure { target } => {
            daemon::reconfigure(target)?;

            println!("Restarting tracker.service to pick up the new config...");
            let status = Command::new("systemctl")
                .args(["--user", "restart", "tracker.service"])
                .status()?;
            if status.success() {
                println!("tracker.service restarted.");
            } else {
                eprintln!(
                    "Automatic restart failed. Run it manually:\n  systemctl --user restart tracker.service"
                );
            }
        }
        TrackerCLI::Push => {
            println!("Push");
            //1. read contents from .env.
            let home_dir = dirs::home_dir().context("failed to get home dir")?;
            let git_dir_str = read_env_key(GIT_DIR)?;
            let git_dir = home_dir.join(&git_dir_str);

            // let user = env::var("USER").unwrap();
            let model = fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
                .unwrap_or_else(|_| "Unknown".to_string())
                .trim()
                .replace(' ', "_");

            //2. always pull before pushing
            pull_repo(&git_dir).context("pull before push failed")?;

            //3. get Status — one state per date the daemon has not pushed yet,
            let tracker_states = get_status().await?;
            if tracker_states.is_empty() {
                println!("Nothing to push.");
                return Ok(());
            }

            //4. write one keystrokes.json per date, then commit
            if let Err(e) = write_and_commit(&git_dir, &model, &tracker_states) {
                restore_repo(&git_dir);
                return Err(e);
            }

            //5. push. The counts are already committed locally, so a network
            // failure here is not data loss — the next push carries the commit
            // along. Reset either way, otherwise the same counts get merged in
            // a second time.
            if let Err(e) = run_git(&git_dir, &["push", "-u", "origin", "main"]) {
                eprintln!(
                    "git push failed: {e:#}\ncommit is saved locally and will go out on the next push"
                );
            }

            //6. reset TrackerState after successfull push
            reset_tracker().await.context("unable to reset tracker")?;
        }
        TrackerCLI::Test => {
            println!("nothing dummy called");
        }
    }
    Ok(())
}

/// AI: Write a command to the daemon using the same u32-length-prefixed framing
/// the daemon uses for its responses.
///
/// AI: `writer` is typed as the concrete `&mut OwnedWriteHalf` because that is the
/// only thing we ever pass in. If this function later needs to write to more than
/// one kind of destination (a plain file, an in-memory `Vec<u8>` in a unit test, a
/// TCP stream, a TLS stream, ...), swap the parameter for a generic bound:
///
///     async fn send_command(writer: &mut (impl AsyncWriteExt + Unpin), action: &str)
///
/// `impl AsyncWriteExt + Unpin` means "any type that can be written to asynchronously"
/// — `AsyncWriteExt` supplies `.write_all()`/`.flush()`, and `Unpin` lets those be
/// `.await`ed by mutable reference. The body here does not change at all; only the
/// signature widens. The usual trade-off: name the concrete type while there is one
/// caller (easier to read), reach for the generic once you genuinely need several.
async fn send_command(writer: &mut OwnedWriteHalf, action: &str) -> anyhow::Result<()> {
    let command = IPCCommand {
        action: action.to_string(),
    };
    let payload = serde_json::to_vec(&command).context("failed to serialize IPCCommand")?;
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await
        .context("failed to write length prefix")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write command body")?;
    writer.flush().await.context("failed to flush command")?;
    Ok(())
}

async fn reset_tracker() -> anyhow::Result<()> {
    ensure_daemon_running().await?;
    let socket_path = get_socket()?;
    let stream = UnixStream::connect(socket_path.as_path())
        .await
        .with_context(|| format!("failed to connect to socket at {}", socket_path.display()))?;
    let (_reader, mut writer) = stream.into_split();
    send_command(&mut writer, "Reset").await?;
    Ok(())
}

async fn get_status() -> anyhow::Result<BTreeMap<String, TrackerState>> {
    ensure_daemon_running().await?;
    let socket_path = get_socket()?;
    let stream = UnixStream::connect(socket_path.as_path())
        .await
        .with_context(|| format!("failed to connect to socket at {}", socket_path.display()))?;
    let (mut reader, mut writer) = stream.into_split();
    // request the status
    send_command(&mut writer, "Read").await?;
    // get the status
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .context("failed to read response length prefix")?;

    let len = u32::from_le_bytes(len_buf) as usize;
    let mut data_buf = vec![0u8; len];
    reader
        .read_exact(&mut data_buf)
        .await
        .context("failed to read response body")?;

    let tracker_states: BTreeMap<String, TrackerState> =
        serde_json::from_slice(&data_buf).context("decoding tracker state from response")?;
    Ok(tracker_states)
}

/// Merge each date's counts into its own `data/{date}/{model}/keystrokes.json`
/// and commit them as a single commit.
fn write_and_commit(
    git_dir: &Path,
    model: &str,
    tracker_states: &BTreeMap<String, TrackerState>,
) -> anyhow::Result<()> {
    for (date, tracker_state) in tracker_states {
        let git_dir_model = git_dir.join("data").join(date).join(model);

        //4. read the current json inside git_dir
        //4.1 if no json then export current state (tracker_state) to json
        if !git_dir_model.join("keystrokes.json").exists() {
            tracker_state.export_to_json(&git_dir_model, true)?;
        }
        // 4.2 else add the stored state with current state (tracker_state) and export new added json (which is also export_to_json with same file name)
        else {
            let stored_state_string = fs::read_to_string(git_dir_model.join("keystrokes.json"))
                .with_context(|| format!("failed to read keystrokes.json for {date}"))?;
            let mut stored_tracker_state: TrackerState = serde_json::from_str(&stored_state_string)
                .with_context(|| format!("failed to parse stored keystrokes.json for {date}"))?;
            stored_tracker_state.add_jsons(tracker_state);
            stored_tracker_state.export_to_json(&git_dir_model, false)?;
        }
    }

    let commit_name = Local::now().to_string();
    let msg = format!("push keystrokes at {}", commit_name);

    run_git(git_dir, &["add", "-A"])?;

    // `git commit` exits non-zero when there is nothing staged,
    // `git diff --cached --quiet` exits 0 when the index is clean.
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(git_dir)
        .status()
        .context("failed to run git diff --cached")?;
    if !staged.success() {
        run_git(git_dir, &["commit", "-m", &msg])?;
    }

    Ok(())
}

/// A failed commit has to abort before the daemon's counters are cleared.
/// hence, returning Result
fn run_git(git_dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(git_dir)
        .status()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !status.success() {
        anyhow::bail!("git {} failed with {}", args.join(" "), status);
    }
    Ok(())
}

/// Roll the data repo back to the last commit after a failed push.
///
/// `restore --staged --worktree` discards edits to tracked files (whether or
/// not `git add` already staged them) and `clean -fd` drops the folders a brand
/// new date created. HEAD is left alone, so an earlier successful commit is
/// never undone. Best-effort: a failure here is reported but must not mask the
/// original push error.
fn restore_repo(git_dir: &Path) {
    for args in [
        &["restore", "--staged", "--worktree", "--", "."][..],
        &["clean", "-fd"][..],
    ] {
        if let Err(e) = run_git(git_dir, args) {
            eprintln!("push rollback failed: {e:#}");
        }
    }
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

    let branch = String::from_utf8(output.stdout).context("invalid utf8 from git")?;
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
    //TODO: can ping to check the connection. I am not doing that, do not feel it's needed.
    let socket_path = get_socket()?;
    if !socket_path.exists() {
        anyhow::bail!("Socket does not exist. Ensure program is run correctly")
    }
    Ok(())
}
