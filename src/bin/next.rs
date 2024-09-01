mod mem_probe;

use clap::{Parser, Subcommand, ValueEnum};
use mem_probe::MemProbe;
use pushover_rs::{send_pushover_request, PushoverSound};
use serde::Deserialize;
use std::{
    env::{self, current_exe},
    ffi::OsString,
    fmt::Display,
    io::Read,
    path::PathBuf,
};
use tokio::{fs, process::Command};
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

    let config = cli
        .resolve_config()
        .await
        .hook_err_if(op.is_deploy(), &pushover)
        .await?;

    match op {
        Op::Upgrade(Target::Hugo) => Hugo::upgrade(&config).await.map(|_| ()),
        Op::Deploy(target) => {
            let mp = MemProbe::new();

            match target {
                Target::Hugo => {
                    let _hugo = Hugo::upgrade(&config).await.hook_err(&pushover).await?;
                }
                _ => (),
            }

            let (mb, _) = mp.join_and_get_mb_sample();
            pushover
                .send_if_some(
                    &format!("GitOps执行成功！\r\n峰值内存：{} MB", mb),
                    PushoverSound::MAGIC,
                )
                .await
        }
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
struct Config {
    hugo: Option<HugoConfig>,
}

#[derive(Deserialize)]
struct HugoConfig {
    version: String,
}

trait HookErrIf<T>: Sized {
    async fn hook_err_if(self, predicate: bool, args: &T) -> Self {
        if predicate {
            self.run_hook(args).await;
        }
        self
    }

    async fn hook_err(self, args: &T) -> Self {
        self.run_hook(args).await;
        self
    }

    async fn run_hook(&self, args: &T);
}

impl<T, E: Display> HookErrIf<Pushover> for Result<T, E> {
    async fn run_hook(&self, args: &Pushover) {
        if let Err(err) = self {
            args.send_if_some(
                &format!("GitOps执行失败！原因：\r\n{}", err),
                PushoverSound::FALLING,
            )
            .await
            .ok();
        }
    }
}

#[allow(dead_code)]
struct Hugo(PathBuf);

impl Hugo {
    async fn upgrade(config: &Config) -> Result<Self, anyhow::Error> {
        let version = &config
            .hugo
            .as_ref()
            .ok_or(anyhow::anyhow!("找不到[hugo]字段！"))?
            .version;

        tracing::info!("请求的hugo版本是：{}", version);
        tracing::info!("正在校验现有hugo版本……");

        let exe = current_exe()?;
        let hugo = exe.with_file_name("hugo");
        let mut need_fetch = true;

        if let Ok(output) = Command::new(&hugo).arg("version").output().await {
            let status = output.status;

            if status.success() {
                if output
                    .stdout
                    .starts_with(format!("hugo v{}", version).as_bytes())
                {
                    need_fetch = false;
                    tracing::info!("现有hugo版本匹配！将跳过下载");
                } else {
                    tracing::info!("现有hug版本不匹配，准备更新hugo");
                }
            } else {
                return Err(anyhow::anyhow!(
                    "hugo version执行失败！退出码：{}",
                    if let Some(code) = status.code() {
                        code.to_string()
                    } else {
                        "None".into()
                    }
                ));
            }
        } else {
            tracing::info!("hugo不存在，准备下载hugo");
        }

        if need_fetch {
            #[cfg(target_os = "macos")]
            const SUFFIX: &str = "darwin-universal.tar.gz";
            #[cfg(target_os = "linux")]
            const SUFFIX: &str = "Linux-64bit.tar.gz";
            #[cfg(target_os = "windows")]
            const SUFFIX: &str = "windows-amd64.zip";

            let url = format!(
                "https://github.com/gohugoio/hugo/releases/download/v{}/hugo_extended_{}_{}",
                version, version, SUFFIX
            );
            tracing::info!("正在GET：{}", url);

            let bytes = reqwest::get(url).await?.error_for_status()?.bytes().await?;

            if bytes.is_empty() {
                return Err(anyhow::anyhow!("未下载任何内容！"));
            } else {
                tracing::info!(
                    "已下载：{} MB",
                    retain_decimal_places(bytes.len() as f64 / 1024.0 / 1024.0, 3)
                );
                tracing::info!("正在解压……");

                let (name, contents) = unzip(&bytes)?;
                tracing::info!(
                    "正在保存：{:?}（{} MB）",
                    name,
                    retain_decimal_places(contents.len() as f64 / 1024.0 / 1024.0, 3)
                );

                let path = exe.with_file_name(name);
                fs::write(&path, contents).await?;

                #[cfg(not(windows))]
                chmod_exec(path).await?;
            }
        }

        Ok(Self(hugo))
    }
}

fn retain_decimal_places(f: f64, n: i32) -> f64 {
    let power = 10.0f64.powi(n);
    (f * power).round() / power
}

#[cfg(windows)]
fn unzip(z: &[u8]) -> Result<(OsString, Vec<u8>), anyhow::Error> {
    use std::io::Cursor;
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(Cursor::new(z))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let path = file
            .enclosed_name()
            .ok_or(anyhow::anyhow!("压缩文件路径异常！"))?;
        let name = path
            .file_name()
            .ok_or(anyhow::anyhow!("压缩文件名异常！"))?;

        if name
            .to_str()
            .ok_or(anyhow::anyhow!("压缩文件名编码异常！"))?
            .starts_with("hugo")
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到hugo执行文件！"))
}

#[cfg(not(windows))]
fn unzip(z: &[u8]) -> Result<(OsString, Vec<u8>), anyhow::Error> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    for entry in Archive::new(GzDecoder::new(z)).entries()? {
        let mut file = entry?;
        let path = file.path()?.into_owned();
        let name = path
            .file_name()
            .ok_or(anyhow::anyhow!("压缩文件名异常！"))?;

        if name
            .to_str()
            .ok_or(anyhow::anyhow!("压缩文件名编码异常！"))?
            .starts_with("hugo")
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到hugo执行文件！"))
}

#[cfg(not(windows))]
async fn chmod_exec(path: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    tracing::info!("正在设置执行权限……");
    use std::{fs::Permissions, os::unix::prelude::PermissionsExt};
    Ok(fs::set_permissions(path, Permissions::from_mode(0o755)).await?)
}
