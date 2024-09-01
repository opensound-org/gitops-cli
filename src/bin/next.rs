use clap::{Parser, Subcommand, ValueEnum};
use pushover_rs::{send_pushover_request, PushoverSound};
use serde::Deserialize;
use std::{env, fmt::Display};
use tokio::fs;
use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    init_tracing_with_ansi();

    let cli = Cli::init();
    let pushover = cli.get_pushover()?;
    let op = &cli.op;

    if op.is_deploy() {
        pushover
            .send_if_some(&format!("开始执行GitOps：{:?}", op), PushoverSound::BIKE)
            .await?;
    }

    let _config = cli.resolve_config().await.hook_err(&pushover).await?;

    match op {
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

    fn get_pushover(&self) -> Result<Pushover, anyhow::Error> {
        if self.op.need_pushover() {
            Ok(Pushover::Some {
                user_key: env::var("PUSHOVER_USER_KEY")?,
                app_token: env::var("PUSHOVER_APP_TOKEN")?,
            })
        } else {
            Ok(Pushover::None)
        }
    }

    async fn resolve_config(&self) -> Result<Config, anyhow::Error> {
        let config = &self.config;
        tracing::info!("正在读取{}……", config);
        Ok(toml::from_str(&fs::read_to_string(config).await?)?)
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

impl Op {
    fn need_pushover(&self) -> bool {
        match self {
            Self::Deploy(_) | Self::Alarm { reason: _, host: _ } => true,
            _ => false,
        }
    }

    fn is_deploy(&self) -> bool {
        if let Self::Deploy(_) = self {
            true
        } else {
            false
        }
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

enum Pushover {
    None,
    Some { user_key: String, app_token: String },
}

impl Pushover {
    async fn send_if_some(&self, message: &str, sound: PushoverSound) -> Result<(), anyhow::Error> {
        match self {
            Self::None => Ok(()),
            Self::Some {
                user_key,
                app_token,
            } => {
                tracing::info!("正在发送Pushover消息：{}", message);
                tracing::info!("Pushover音色：{}", sound);

                match send_pushover_request(
                    pushover_rs::MessageBuilder::new(user_key, app_token, message)
                        .set_sound(sound)
                        .build(),
                )
                .await
                {
                    Ok(res) => match res.errors {
                        None => Ok(()),
                        Some(errs) => Err(anyhow::anyhow!("{}", errs.join("\r\n"))),
                    },
                    Err(err) => Err(anyhow::anyhow!("{}", err)),
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Config;

trait HookErr<T> {
    async fn hook_err(self, args: &T) -> Self;
}

impl<T, E: Display> HookErr<Pushover> for Result<T, E> {
    async fn hook_err(self, args: &Pushover) -> Self {
        if let Err(err) = &self {
            args.send_if_some(
                &format!("GitOps执行失败！原因：\r\n{}", err),
                PushoverSound::FALLING,
            )
            .await
            .ok();
        }
        self
    }
}
