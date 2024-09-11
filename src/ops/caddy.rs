use super::super::{
    utils::{env_var, retain_decimal_places, spawn_command, unzip},
    Config,
};
use opendal::{layers::MimeGuessLayer, services::Oss, Operator};
use serde::{Deserialize, Serialize};
use std::{
    env::current_exe,
    path::{Path, PathBuf},
};
use tokio::{fs, process::Command};

#[derive(Deserialize, Serialize, Default)]
pub struct CaddyConfig {
    version: String,
    deploy: Option<DeployConfig>,
    routes: Option<Routes>,
}

impl CaddyConfig {
    fn get_caddyfile(&self) -> Result<String, anyhow::Error> {
        Ok(format!(
            "{}\r\n",
            if let Some(r) = &self.routes {
                r.join_site_blocks()?
            } else {
                "".into()
            }
        ))
    }

    fn version(version: &str) -> Self {
        Self {
            version: version.into(),
            ..Default::default()
        }
    }
}

impl From<CaddyConfig> for Config {
    fn from(value: CaddyConfig) -> Self {
        Self {
            caddy: Some(value),
            ..Default::default()
        }
    }
}

#[derive(Deserialize, Serialize, Clone)]
struct DeployConfig {
    oss: OssConfig,
}

#[derive(Deserialize, Serialize, Clone)]
struct OssConfig {
    root: String,
    access_key_id: Option<String>,
    access_key_secret: Option<String>,
    ops_bucket: Option<String>,
    ops_endpoint: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Routes {
    fs: Option<Vec<FileServer>>,
    redirs: Option<Vec<AddrsBackend>>,
    rev_proxies: Option<Vec<AddrsBackend>>,
}

impl Routes {
    fn join_site_blocks(&self) -> Result<String, anyhow::Error> {
        let mut blocks = Vec::new();

        if let Some(fs) = &self.fs {
            push_site_blocks(fs, &mut blocks, |r| r.to_fs_site_block())?;
        }

        if let Some(redirs) = &self.redirs {
            push_site_blocks(redirs, &mut blocks, |r| r.to_redir_site_block())?;
        }

        if let Some(proxies) = &self.rev_proxies {
            push_site_blocks(proxies, &mut blocks, |r| r.to_proxy_site_block())?;
        }

        Ok(blocks.join("\r\n\r\n"))
    }
}

fn push_site_blocks<T>(
    r: &Vec<T>,
    blocks: &mut Vec<String>,
    f: impl Fn(&T) -> Result<String, anyhow::Error>,
) -> Result<(), anyhow::Error> {
    Ok(r.iter()
        .try_for_each(|r| Ok::<(), anyhow::Error>(blocks.push(f(r)?)))?)
}

#[derive(Deserialize, Serialize)]
struct FileServer {
    addrs: Vec<String>,
    dir: PathBuf,
}

impl FileServer {
    fn to_fs_site_block(&self) -> Result<String, anyhow::Error> {
        check_addrs(&self.addrs)?;
        Ok(format!(
            "{} {{\r\n  root {}\r\n  file_server browse\r\n}}",
            self.addrs.join(", "),
            self.dir.display()
        ))
    }
}

#[derive(Deserialize, Serialize)]
struct AddrsBackend {
    addrs: Vec<String>,
    backend: String,
}

impl AddrsBackend {
    fn to_redir_site_block(&self) -> Result<String, anyhow::Error> {
        check_addrs(&self.addrs)?;
        Ok(format!(
            "{} {{\r\n  redir https://{}{{uri}}\r\n}}",
            self.addrs.join(", "),
            self.backend
        ))
    }

    fn to_proxy_site_block(&self) -> Result<String, anyhow::Error> {
        check_addrs(&self.addrs)?;
        Ok(format!(
            "{} {{\r\n  reverse_proxy {}\r\n}}",
            self.addrs.join(", "),
            self.backend
        ))
    }
}

fn check_addrs(addrs: &[String]) -> Result<(), anyhow::Error> {
    match addrs.is_empty() {
        true => Err(anyhow::anyhow!("addrs不能为空！")),
        false => Ok(()),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn upgrade(config: &Config) -> Result<(), anyhow::Error> {
    let version = &get_config(config)?.version;

    tracing::info!("请求的caddy版本是：{}", version);
    tracing::info!("正在校验现有caddy版本……");

    let exe = current_exe()?;
    let caddy = exe.with_file_name("caddy");
    let mut need_fetch = true;

    if let Ok(output) = Command::new(&caddy).arg("version").output().await {
        let status = output.status;

        if status.success() {
            if output
                .stdout
                .starts_with(format!("v{}", version).as_bytes())
            {
                need_fetch = false;
                tracing::info!("现有caddy版本匹配！将跳过下载");
            } else {
                tracing::info!("现有caddy版本不匹配，准备更新caddy");
            }
        } else {
            return Err(anyhow::anyhow!(
                "caddy version执行失败！退出码：{}",
                if let Some(code) = status.code() {
                    code.to_string()
                } else {
                    "None".into()
                }
            ));
        }
    } else {
        return Err(anyhow::anyhow!("需要先手动部署一个初始版本的caddy！"));
    }

    if need_fetch {
        #[cfg(windows)]
        use windows_service::{
            service::ServiceAccess,
            service_manager::{ServiceManager, ServiceManagerAccess},
        };

        #[cfg(windows)]
        tracing::info!("正在连接本地服务……");

        #[cfg(windows)]
        let service = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?
            .open_service("caddy", ServiceAccess::START | ServiceAccess::STOP)?;

        #[cfg(target_os = "linux")]
        const SUFFIX: &str = "linux_amd64.tar.gz";
        #[cfg(target_os = "windows")]
        const SUFFIX: &str = "windows_amd64.zip";

        let url = format!(
            "https://github.com/caddyserver/caddy/releases/download/v{}/caddy_{}_{}",
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

            let (name, contents) = unzip(&bytes, "caddy")?;

            #[cfg(windows)]
            {
                tracing::info!("正在停止服务……");
                service.stop()?;
            }

            tracing::info!(
                "正在保存：{:?}（{} MB）",
                name,
                retain_decimal_places(contents.len() as f64 / 1024.0 / 1024.0, 3)
            );

            let path = exe.with_file_name(name);
            fs::write(&path, contents).await?;

            #[cfg(not(windows))]
            crate::utils::chmod_exec(path).await?;

            #[cfg(not(windows))]
            {
                tracing::info!("正在停止服务……");
                spawn_command(Command::new(&caddy).arg("stop"), "caddy stop").await?;
            }

            tracing::info!("正在启动服务……");

            #[cfg(windows)]
            service.start::<&str>(&[])?;
            #[cfg(not(windows))]
            spawn_command(Command::new(&caddy).arg("start"), "caddy start").await?;
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn upgrade(_config: &Config) -> Result<(), anyhow::Error> {
    Err(anyhow::anyhow!("不支持macOS！"))
}

fn get_config(config: &Config) -> Result<&CaddyConfig, anyhow::Error> {
    config
        .caddy
        .as_ref()
        .ok_or(anyhow::anyhow!("找不到[caddy]字段！"))
}

pub async fn deploy(config: &Config) -> Result<(), anyhow::Error> {
    let config = get_config(config)?;
    let mut oss = config
        .deploy
        .clone()
        .ok_or(anyhow::anyhow!("找不到[caddy.deploy]字段！"))?
        .oss;

    oss.access_key_id.replace(env_var("OSS_ACCESS_KEY_ID")?);
    oss.access_key_secret
        .replace(env_var("OSS_ACCESS_KEY_SECRET")?);
    oss.ops_bucket.replace(env_var("OSS_OPS_BUCKET")?);
    oss.ops_endpoint.replace(env_var("OSS_OPS_ENDPOINT")?);

    tracing::info!("正在生成Caddyfile……");
    let caddyfile = config.get_caddyfile()?;

    tracing::info!("正在保存Caddyfile……");
    fs::write("Caddyfile", &caddyfile).await?;

    tracing::info!("正在提交git……");
    spawn_command(Command::new("git").arg("add").arg("Caddyfile"), "git").await?;

    if Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("版本控制生成的Caddyfile")
        .spawn()?
        .wait()
        .await?
        .success()
    {
        tracing::info!("正在执行：git push");
        spawn_command(Command::new("git").arg("push"), "git").await?;
    } else {
        tracing::warn!("没有可以提交的内容！");
    }

    tracing::info!("正在初始化OSS Operator……");
    let op = Operator::new(
        Oss::default()
            .root(&oss.root)
            .access_key_id(oss.access_key_id.as_ref().unwrap())
            .access_key_secret(oss.access_key_secret.as_ref().unwrap())
            .bucket(oss.ops_bucket.as_ref().unwrap())
            .endpoint(oss.ops_endpoint.as_ref().unwrap()),
    )?
    .layer(MimeGuessLayer::default())
    .finish();

    tracing::info!("正在上传：Caddyfile");
    op.write("Caddyfile", caddyfile).await?;

    tracing::info!("正在上传处理后的：gitops.toml");
    op.write(
        "gitops.toml",
        toml::to_string_pretty(&Config::from(CaddyConfig::version(&config.version)))?,
    )
    .await?;

    let sync_toml = Path::new("sync.toml");
    if sync_toml.is_file() {
        tracing::info!("正在上传：sync.toml");
        op.write("sync.toml", fs::read(sync_toml).await?).await?;
    }

    Ok(())
}
