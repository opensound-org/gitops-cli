use super::super::{
    utils::{retain_decimal_places, unzip},
    Config,
};
use serde::Deserialize;
use std::env::current_exe;
use tokio::{fs, process::Command};

#[derive(Deserialize)]
pub struct CaddyConfig {
    version: String,
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
            chmod_exec(path).await?;

            #[cfg(not(windows))]
            use super::super::spawn_command;

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
