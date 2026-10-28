use clap::{Parser, Subcommand};

mod config;
mod credentials;
mod display;
mod fetch;
mod list;
mod notify;
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
    /// キャッシュから一覧を表示する（Slack の API は呼ばない）
    List {
        #[arg(long, value_enum, default_value_t = list::Format::Text)]
        format: list::Format,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Fetch => fetch::run(),
        Command::List { format } => list::run(format),
    }
}
