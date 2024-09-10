use std::{
    env,
    ffi::{OsStr, OsString},
    io::Read,
};
use tokio::process::Command;

pub fn env_var(key: impl AsRef<OsStr>) -> Result<String, anyhow::Error> {
    env::var(key.as_ref()).map_err(|_| anyhow::anyhow!("找不到环境变量：{:?}", key.as_ref()))
}

pub fn retain_decimal_places(f: f64, n: i32) -> f64 {
    let power = 10.0f64.powi(n);
    (f * power).round() / power
}

#[cfg(windows)]
pub fn unzip(z: &[u8], e_name: &str) -> Result<(OsString, Vec<u8>), anyhow::Error> {
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
            .starts_with(e_name)
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到{}执行文件！", e_name))
}

#[cfg(not(windows))]
pub fn unzip(z: &[u8], e_name: &str) -> Result<(OsString, Vec<u8>), anyhow::Error> {
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
            .starts_with(e_name)
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到{}执行文件！", e_name))
}

#[cfg(not(windows))]
pub async fn chmod_exec(path: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    tracing::info!("正在设置执行权限……");
    use std::{fs::Permissions, os::unix::prelude::PermissionsExt};
    Ok(fs::set_permissions(path, Permissions::from_mode(0o755)).await?)
}

pub async fn spawn_command(cmd: &mut Command, hint: &str) -> Result<(), anyhow::Error> {
    let status = cmd.spawn()?.wait().await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "{}命令执行失败！退出码：{}",
            hint,
            if let Some(code) = status.code() {
                code.to_string()
            } else {
                "None".into()
            }
        ))
    }
}
