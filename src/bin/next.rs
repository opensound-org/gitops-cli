use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

#[derive(Parser, Debug)]
enum Op {
    #[command(subcommand)]
    Upgrade(Target),
    #[command(subcommand)]
    Deploy(Target),
    Sync,
    Alarm {
        reason: Alarm,
        host: String,
    },
}

impl Op {
    fn init() -> Self {
        let s = Self::parse();
        tracing::info!("{:?}", s);
        s
    }
}

#[derive(Subcommand, Debug)]
enum Target {
    Hugo,
    Caddy,
}

#[derive(ValueEnum, Debug, Clone)]
enum Alarm {
    Login,
    ConnectRemote,
    ConnectLocal,
    Unlock,
}

#[cfg(debug_assertions)]
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::new("%m-%d %H:%M:%S".into()))
        .with_max_level(tracing::Level::DEBUG)
        .with_span_events(FmtSpan::FULL)
        .with_thread_names(true)
        .init();
}

#[cfg(not(debug_assertions))]
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::new("%m-%d %H:%M:%S".into()))
        .with_span_events(FmtSpan::FULL)
        .with_thread_names(true)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    #[cfg(windows)]
    nu_ansi_term::enable_ansi_support().ok();
    init_tracing();

    match Op::init() {
        _ => Err(anyhow::anyhow!("暂时todo！")),
    }
}
