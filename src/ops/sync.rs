use super::super::{utils::env_var, Config};
use opendal::{services::Oss, Operator};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize, Serialize, Clone)]
pub struct SyncConfig {
    bucket: String,
    endpoint: String,
    root: String,
    files: Vec<String>,
    access_key_id: Option<String>,
    access_key_secret: Option<String>,
}

pub async fn sync(config: &Config) -> Result<(), anyhow::Error> {
    let mut config = config
        .sync
        .clone()
        .ok_or(anyhow::anyhow!("找不到[sync]字段！"))?;

    config.access_key_id.replace(env_var("OSS_ACCESS_KEY_ID")?);
    config
        .access_key_secret
        .replace(env_var("OSS_ACCESS_KEY_SECRET")?);

    tracing::info!("正在初始化OSS Operator……");
    let op = Operator::new(
        Oss::default()
            .root(&config.root)
            .access_key_id(config.access_key_id.as_ref().unwrap())
            .access_key_secret(config.access_key_secret.as_ref().unwrap())
            .bucket(&config.bucket)
            .endpoint(&config.endpoint),
    )?
    .finish();

    for f in &config.files {
        tracing::info!("正在下载：{}", f);
        let contents = op.read(f).await?.to_bytes();

        tracing::info!("正在保存：{}", f);
        fs::write(f, contents).await?;
    }

    Ok(())
}
