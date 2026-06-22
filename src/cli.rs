use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[command(subcommand)]
    pub get: KeyPressStatus,
}

#[derive(Subcommand, Debug)]
pub enum KeyPressStatus {
    /// start the daemon in the background
    #[command(hide = true)]
    Daemon,

    /// Get current keypress status
    Status,

    /// Push the current session
    Push,

    /// Pull into current session
    Pull,

    /// Generate daily report
    Sync,
}
