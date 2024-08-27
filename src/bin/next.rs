use clap::{Parser, Subcommand, ValueEnum};

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

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    println!("{:?}", Op::parse());
    Ok(())
}
