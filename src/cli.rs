use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[command(subcommand)]
    pub get: TrackerCLI,
}

#[derive(Subcommand, Debug)]
pub enum TrackerCLI {
    /// start the daemon in the background (run by systemd service tracker)
    #[command(hide = true)]
    Daemon,

    /// Init git repo that stores the trackerdata (run by ./install.sh, do not run again).
    #[command(hide = true)]
    Init,

    /// Get current tracker status
    Status,

    /// Push the current session
    Push,

    /// Pull into current session
    Pull,

    /// Re-run setup and update device path
    Reconfigure {
        #[command(subcommand)]
        target: Option<ReconfigureTarget>,
    },

    /// just for test will need to remove later
    Test,
}

#[derive(Subcommand, Debug)]
pub enum ReconfigureTarget {
    /// Re-run setup for keyboard only
    Keyboard,
    /// Re-run setup for mouse only
    Mouse,
    /// Re-run setup for both and will be activated by default
    All,
}
