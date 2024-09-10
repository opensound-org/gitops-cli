use super::super::{
    opendal_fs::{sync_dir, ConcurrentUploadTasks},
    utils::{env_var, retain_decimal_places, spawn_command, unzip},
    Config,
};
use fs_extra::dir;
use opendal::{layers::MimeGuessLayer, services::Oss, Operator};
use serde::Deserialize;
use std::{
    env::{current_exe, set_current_dir},
    ffi::OsStr,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, remove_dir_all},
    process::Command,
    task::spawn_blocking,
};

#[derive(Deserialize)]
pub struct HugoConfig {
    version: String,
    deploy: Option<DeployConfig>,
}

#[derive(Deserialize, Clone)]
struct DeployConfig {
    github: GithubConfig,
    oss: OssConfig,
}

#[derive(Deserialize, Clone)]
struct GithubConfig {
    username: String,
    org: String,
    repo: String,
    access_token: Option<String>,
    user_email: Option<String>,
    user_name: Option<String>,
}

#[derive(Deserialize, Clone)]
struct OssConfig {
    sync: OssSyncConfig,
    access_key_id: Option<String>,
    access_key_secret: Option<String>,
}

#[derive(Deserialize, Clone)]
struct OssSyncConfig {
    root: String,
    files: Vec<String>,
    dirs: Vec<String>,
}

pub struct Hugo(PathBuf);

impl Hugo {
    pub async fn upgrade(config: &Config) -> Result<(Self, &HugoConfig), anyhow::Error> {
        let config = config
            .hugo
            .as_ref()
            .ok_or(anyhow::anyhow!("找不到[hugo]字段！"))?;
        let version = &config.version;

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

        Ok((Self(hugo), config))
    }

    async fn deploy_step(
        &self,
        config: &DeployConfig,
        for_draft: bool,
    ) -> Result<(), anyhow::Error> {
        tracing::info!(
            "正在hugo deploy {}版本……",
            if for_draft { "draft" } else { "production" }
        );

        remove_public().await?;

        let mut hugo = Command::new(&self.0);
        let (hugo, base_url) = if for_draft {
            let base_url = env_var("HUGO_DRAFT_BASE_URL")?;
            (
                hugo.arg("-b").arg(&base_url).arg("-D").arg("-F"),
                Some(base_url),
            )
        } else {
            (&mut hugo, None)
        };

        if let Some(base_url) = base_url {
            tracing::info!(
                "正在执行：hugo {}",
                hugo.as_std()
                    .get_args()
                    .collect::<Vec<&OsStr>>()
                    .join(" ".as_ref())
                    .to_string_lossy()
                    .replace(&base_url, "****")
            );
        } else {
            tracing::info!("正在执行：hugo");
        }
        spawn_command(hugo, "hugo").await?;

        deploy_github(&config.github, for_draft).await?;
        deploy_oss(&config.oss, for_draft).await
    }
}

pub async fn deploy(config: &Config) -> Result<(), anyhow::Error> {
    let (hugo, config) = Hugo::upgrade(&config).await?;
    let mut config = config
        .deploy
        .clone()
        .ok_or(anyhow::anyhow!("找不到[hugo.deploy]字段！"))?;

    config
        .github
        .access_token
        .replace(env_var("DEPLOY_GITHUB_ACCESS_TOKEN")?);
    config
        .github
        .user_email
        .replace(env_var("DEPLOY_GITHUB_USER_EMAIL")?);
    config
        .github
        .user_name
        .replace(env_var("DEPLOY_GITHUB_USER_NAME")?);
    config
        .oss
        .access_key_id
        .replace(env_var("OSS_ACCESS_KEY_ID")?);
    config
        .oss
        .access_key_secret
        .replace(env_var("OSS_ACCESS_KEY_SECRET")?);

    tracing::info!("================");
    hugo.deploy_step(&config, true).await?;

    tracing::info!("================");
    hugo.deploy_step(&config, false).await?;

    Ok(())
}

async fn deploy_github(config: &GithubConfig, for_draft: bool) -> Result<(), anyhow::Error> {
    tracing::info!(
        "正在deploy github {}",
        if for_draft { "draft" } else { "main" }
    );

    let repo = &config.repo;
    let access_token = config.access_token.as_ref().unwrap();
    let url = format!(
        "https://{}:{}@github.com/{}/{}.git",
        config.username, access_token, config.org, repo
    );

    tracing::info!("正在执行：git clone {}", url.replace(access_token, "****"));
    spawn_command(Command::new("git").arg("clone").arg(url), "git").await?;
    set_current_dir(repo)?;

    tracing::info!("正在配置git环境……");
    spawn_command(
        Command::new("git")
            .arg("config")
            .arg("user.email")
            .arg(config.user_email.as_ref().unwrap()),
        "git",
    )
    .await?;
    spawn_command(
        Command::new("git")
            .arg("config")
            .arg("user.name")
            .arg(config.user_name.as_ref().unwrap()),
        "git",
    )
    .await?;

    if for_draft {
        tracing::info!("正在执行：git checkout draft");
        spawn_command(Command::new("git").arg("checkout").arg("draft"), "git").await?;
    }

    remove_public().await?;

    tracing::info!("正在拷贝public目录……");
    spawn_blocking(move || dir::copy("../public", "", &Default::default())).await??;

    tracing::info!("正在提交……");
    spawn_command(Command::new("git").arg("add").arg("."), "git").await?;

    if Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Deploy")
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

    tracing::info!("正在清理{}目录……", repo);
    set_current_dir("..")?;
    Ok(remove_dir_all(repo).await?)
}

async fn deploy_oss(config: &OssConfig, for_draft: bool) -> Result<(), anyhow::Error> {
    tracing::info!(
        "正在deploy oss {}",
        if for_draft { "draft" } else { "prod" }
    );
    set_current_dir("public")?;

    tracing::info!("正在初始化Operator……");
    let sync = &config.sync;
    let oss = Oss::default()
        .root(&sync.root)
        .access_key_id(config.access_key_id.as_ref().unwrap())
        .access_key_secret(config.access_key_secret.as_ref().unwrap());
    let oss = if for_draft {
        oss.bucket(&env_var("OSS_DRAFT_BUCKET")?)
            .endpoint(&env_var("OSS_DRAFT_ENDPOINT")?)
    } else {
        oss.bucket(&env_var("OSS_PROD_BUCKET")?)
            .endpoint(&env_var("OSS_PROD_ENDPOINT")?)
    };

    let op = Operator::new(oss)?
        .layer(MimeGuessLayer::default())
        .finish();

    tracing::info!("开始上传文件……");
    let mut files = ConcurrentUploadTasks::new(op.clone());
    files.push_str_seq(&sync.files).await?;
    files.join().await?;

    tracing::info!("开始同步目录……");
    for dir in &sync.dirs {
        tracing::info!("正在同步目录：{}", dir);
        sync_dir(&op, dir).await?;
    }

    Ok(set_current_dir("..")?)
}

async fn remove_public() -> Result<(), anyhow::Error> {
    let public = Path::new("public");
    if public.is_dir() {
        tracing::info!("正在清理public目录……");
        remove_dir_all(public).await?;
    }
    Ok(())
}
