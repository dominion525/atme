use clap::{Parser, Subcommand};

mod app;
mod config;
mod credentials;
mod dismiss;
mod display;
mod fetch;
mod list;
mod menu;
mod notify;
mod read;
mod scheduler;
mod slack;
mod state;

#[derive(Parser)]
#[command(
    version,
    about = "Slack の自分宛メンションと DM を見落とさないためのツール"
)]
struct Cli {
    /// 省略するとメニューバーに常駐する（atme.app を open したときの動き）
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// メンションと DM を取得し、差分を取ってキャッシュに保存する
    Fetch,
    /// キャッシュから一覧を表示する（Slack の API は呼ばない）
    List,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => app::run(),
        Some(Command::Fetch) => fetch::run(),
        Some(Command::List) => list::run(),
    }
}
