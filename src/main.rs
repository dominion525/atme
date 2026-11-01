use clap::{Parser, Subcommand};

mod app;
mod config;
mod credentials;
mod dismiss;
mod display;
mod fetch;
mod l10n;
mod list;
mod login;
mod menu;
mod notify;
mod read;
mod scheduler;
mod setup;
mod slack;
mod state;

#[derive(Parser)]
#[command(
    version,
    about = "Never miss Slack mentions and DMs addressed to you"
)]
struct Cli {
    /// Stays in the menu bar when omitted (what happens when atme.app is opened)
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Fetches mentions and DMs, finds the new ones, and saves the cache
    Fetch,
    /// Shows the list from the cache (doesn't call the Slack API)
    List,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => app::run(),
        Some(Command::Fetch) => fetch::run(),
        Some(Command::List) => list::run(),
    }
}
