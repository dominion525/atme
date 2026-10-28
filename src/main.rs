use clap::{Parser, Subcommand};

mod config;
mod credentials;
mod fetch;
mod slack;
mod state;

#[derive(Parser)]
#[command(
    version,
    about = "Slack の自分宛メンションと DM を見落とさないためのツール"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// メンションと DM を取得し、差分を取ってキャッシュに保存する
    Fetch,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Fetch => fetch::run(),
    }
}
