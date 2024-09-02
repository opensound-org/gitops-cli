use super::super::{retain_decimal_places, unzip, Config};
use serde::Deserialize;
use std::{env::current_exe, path::PathBuf};
use tokio::{fs, process::Command};

#[derive(Deserialize)]
pub struct HugoConfig {
    version: String,
}

#[allow(dead_code)]
pub struct Hugo(PathBuf);

impl Hugo {
    pub async fn upgrade(config: &Config) -> Result<Self, anyhow::Error> {
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
                    tracing::info!("现有hugo版本不匹配，准备更新hugo");
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

                let (name, contents) = unzip(&bytes, "hugo")?;
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
