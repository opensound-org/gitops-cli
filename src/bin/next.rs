use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    init_tracing_with_ansi();

    match Cli::init() {
        _ => Err(anyhow::anyhow!("暂时todo！")),
    }
}

// 参见：https://github.com/tokio-rs/tracing/issues/3068
fn init_tracing_with_ansi() {
    #[cfg(windows)]
    nu_ansi_term::enable_ansi_support().ok();
    init_tracing();
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

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    op: Op,
    #[arg(default_value = "gitops.toml")]
    config: String,
}

impl Cli {
    fn init() -> Self {
        let s = Self::parse();
        tracing::info!("config: {:?}", s.config);
        tracing::info!("op: {:?}", s.op);
        s
    }
}

#[derive(Subcommand, Debug)]
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
