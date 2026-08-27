use std::{
    env, fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use reqwest::{redirect::Policy, Client};
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::{
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::{
    models::{
        FeishuCliProgress, FeishuCliStatus, FeishuDestination, FeishuLoginSession, FeishuSelf,
        FeishuSendProgress, StickerRecord,
    },
    store::StickerStore,
};

const NPM_LATEST_URL: &str = "https://registry.npmjs.org/@larksuite%2fcli/latest";
const GITHUB_RELEASE_BASE: &str = "https://github.com/larksuite/cli/releases/download";
const NPM_MIRROR_BASE: &str = "https://registry.npmmirror.com/-/binary/lark-cli";
const MAX_METADATA_BYTES: usize = 5 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 200 * 1024 * 1024;
const DOWNLOAD_ATTEMPTS_PER_SOURCE: usize = 2;
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

struct CliResult {
    code: i32,
    stdout: String,
    stderr: String,
}

struct PendingLogin {
    device_code: String,
    session: FeishuLoginSession,
}

#[derive(Clone)]
struct CliLocation {
    path: PathBuf,
    source: &'static str,
}

struct ReleaseSpec {
    version: Version,
    archive_name: String,
    checksum: String,
    github_url: String,
    mirror_url: String,
}

struct DownloadSource<'a> {
    label: &'static str,
    url: &'a str,
}

#[derive(Clone, Copy)]
struct DownloadAttempt {
    current: usize,
    total: usize,
    source: &'static str,
}

#[derive(Deserialize)]
struct NpmLatestMetadata {
    version: String,
}

pub struct FeishuCli {
    managed_directory: PathBuf,
    fallback_executables: Vec<PathBuf>,
    pending_login: Option<PendingLogin>,
    sending: bool,
}

pub(crate) fn migrate_managed_component(source: &Path, destination: &Path) -> Result<bool> {
    let source_executable = source.join(cli_binary_name());
    let destination_executable = destination.join(cli_binary_name());
    if is_executable_file(&destination_executable) || !is_executable_file(&source_executable) {
        return Ok(false);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if !destination.exists() && fs::rename(source, destination).is_ok() {
        return Ok(true);
    }

    fs::create_dir_all(destination)?;
    let temporary_executable = destination_executable.with_extension("migrating");
    let _ = fs::remove_file(&temporary_executable);
    fs::copy(&source_executable, &temporary_executable).context("无法迁移已安装的飞书 CLI")?;
    fs::set_permissions(
        &temporary_executable,
        fs::metadata(&source_executable)?.permissions(),
    )?;
    fs::rename(&temporary_executable, &destination_executable)
        .context("无法启用迁移后的飞书 CLI")?;

    let source_metadata = source.join("component.json");
    if source_metadata.is_file() {
        let _ = fs::copy(&source_metadata, destination.join("component.json"));
    }

    let _ = fs::remove_file(source_executable);
    let _ = fs::remove_file(source_metadata);
    let _ = fs::remove_dir(source);
    if let Some(parent) = source.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(true)
}

impl FeishuCli {
    pub fn new(managed_directory: PathBuf, fallback_executables: Vec<PathBuf>) -> Self {
        Self {
            managed_directory,
            fallback_executables,
            pending_login: None,
            sending: false,
        }
    }

    pub async fn status(&self) -> FeishuCliStatus {
        let Some(location) = self.resolve_executable() else {
            return unavailable_status(format!(
                "尚未安装飞书官方连接组件。点击“下载官方组件”后会保存到：{}",
                self.managed_directory.display()
            ));
        };
        let current = env::current_dir().unwrap_or_default();
        let version_result = match self
            .run_with(
                &location.path,
                &["--version"],
                &current,
                Duration::from_secs(15),
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                return FeishuCliStatus {
                    installed: false,
                    version: None,
                    authenticated: false,
                    detail: Some(error.to_string()),
                    source: Some(location.source.to_string()),
                    executable_path: Some(location.path.to_string_lossy().to_string()),
                    latest_version: None,
                    update_available: false,
                };
            }
        };
        if version_result.code != 0 {
            return FeishuCliStatus {
                installed: false,
                version: None,
                authenticated: false,
                detail: Some(error_text(&version_result)),
                source: Some(location.source.to_string()),
                executable_path: Some(location.path.to_string_lossy().to_string()),
                latest_version: None,
                update_available: false,
            };
        }

        let version = normalized_version(&version_result);
        let auth_result = match self
            .run_with(
                &location.path,
                &["auth", "status", "--json"],
                &current,
                Duration::from_secs(20),
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                return FeishuCliStatus {
                    installed: true,
                    version: Some(version),
                    authenticated: false,
                    detail: Some(error.to_string()),
                    source: Some(location.source.to_string()),
                    executable_path: Some(location.path.to_string_lossy().to_string()),
                    latest_version: None,
                    update_available: false,
                };
            }
        };
        let combined = format!("{}\n{}", auth_result.stdout, auth_result.stderr).to_lowercase();
        let parsed = parse_json(&auth_result.stdout);
        let (identity_authenticated, identity_detail) = user_identity_state(parsed.as_ref());
        let authenticated = auth_result.code == 0
            && !combined.contains("not logged")
            && !combined.contains("未登录")
            && !combined.contains("user identity is missing")
            && !combined.contains("expired")
            && parsed.is_some()
            && identity_authenticated != Some(false);
        FeishuCliStatus {
            installed: true,
            version: Some(version),
            authenticated,
            detail: Some(if authenticated {
                "飞书 CLI 用户身份已登录".to_string()
            } else {
                identity_detail.unwrap_or_else(|| error_text(&auth_result))
            }),
            source: Some(location.source.to_string()),
            executable_path: Some(location.path.to_string_lossy().to_string()),
            latest_version: None,
            update_available: false,
        }
    }

    pub async fn check_update(&self) -> FeishuCliStatus {
        let mut status = self.status().await;
        let Ok(release) = self.latest_release_spec().await else {
            return status;
        };
        status.latest_version = Some(release.version.to_string());
        status.update_available = status
            .version
            .as_deref()
            .and_then(|version| Version::parse(version).ok())
            .map(|current| current < release.version)
            .unwrap_or(!status.installed);
        status
    }

    pub async fn install_latest(&mut self, app: &AppHandle) -> Result<FeishuCliStatus> {
        emit_cli_progress(
            app,
            "resolving",
            0,
            None,
            "正在读取飞书官方版本与校验信息…",
            false,
        );
        let release = self.latest_release_spec().await?;
        emit_cli_progress(
            app,
            "downloading",
            0,
            None,
            &format!("正在下载飞书 CLI {}…", release.version),
            false,
        );
        let archive = download_verified_release_archive(&release, app).await?;
        let archive_size = archive.len() as u64;
        emit_cli_progress(
            app,
            "installing",
            archive_size,
            Some(archive_size),
            "校验通过，正在安装组件…",
            false,
        );
        let archive_name = release.archive_name.clone();
        let binary =
            tokio::task::spawn_blocking(move || extract_cli_binary(&archive_name, &archive))
                .await
                .context("飞书 CLI 解压任务异常")??;
        let destination = self.managed_executable();
        let version = release.version.to_string();
        let checksum = release.checksum.clone();
        tokio::task::spawn_blocking(move || {
            persist_managed_binary(&destination, &version, &checksum, &binary)
        })
        .await
        .context("飞书 CLI 安装任务异常")??;
        emit_cli_progress(
            app,
            "done",
            archive_size,
            Some(archive_size),
            &format!("飞书 CLI {} 已安装并通过 SHA-256 校验", release.version),
            true,
        );
        let mut status = self.status().await;
        status.latest_version = Some(release.version.to_string());
        status.update_available = false;
        Ok(status)
    }

    pub async fn get_self(&self) -> Result<FeishuSelf> {
        let current = env::current_dir().unwrap_or_default();
        let result = self
            .run(
                &["contact", "+get-user", "--as", "user", "--format", "json"],
                &current,
                Duration::from_secs(30),
            )
            .await?;
        if result.code != 0 {
            return Err(anyhow!(error_text(&result)));
        }
        let payload = parse_json(&result.stdout);
        let open_id = find_deep_string(payload.as_ref(), &["open_id", "openId"])
            .ok_or_else(|| anyhow!("飞书 CLI 未返回当前用户 open_id，请重新连接飞书"))?;
        let name = find_deep_string(payload.as_ref(), &["name", "display_name", "displayName"]);
        Ok(FeishuSelf { open_id, name })
    }

    pub async fn start_login(&mut self) -> Result<FeishuLoginSession> {
        let current = env::current_dir().unwrap_or_default();
        let result = self
            .run(
                &[
                    "auth",
                    "login",
                    "--domain",
                    "contact,im",
                    "--no-wait",
                    "--json",
                ],
                &current,
                Duration::from_secs(30),
            )
            .await?;
        if result.code != 0 {
            return Err(anyhow!(error_text(&result)));
        }
        let pending = parse_login_payload(parse_json(&result.stdout).as_ref())?;
        let session = pending.session.clone();
        self.pending_login = Some(pending);
        Ok(session)
    }

    pub fn pending_login_url(&self) -> Option<String> {
        self.pending_login
            .as_ref()
            .map(|pending| pending.session.verification_url.clone())
    }

    pub async fn finish_login(&mut self) -> Result<FeishuCliStatus> {
        let device_code = self
            .pending_login
            .as_ref()
            .map(|pending| pending.device_code.clone())
            .ok_or_else(|| anyhow!("当前没有待完成的飞书授权，请重新点击“连接飞书”"))?;
        let current = env::current_dir().unwrap_or_default();
        let result = self
            .run(
                &["auth", "login", "--device-code", &device_code, "--json"],
                &current,
                Duration::from_secs(150),
            )
            .await?;
        if result.code != 0 {
            return Err(anyhow!(error_text(&result)));
        }
        self.pending_login = None;
        let status = self.status().await;
        if !status.authenticated {
            return Err(anyhow!(status
                .detail
                .clone()
                .unwrap_or_else(|| "飞书授权尚未完成".into())));
        }
        Ok(status)
    }

    pub fn cancel_login(&mut self) {
        self.pending_login = None;
    }

    pub async fn send_batch(
        &mut self,
        records: Vec<StickerRecord>,
        destination: FeishuDestination,
        store: Arc<Mutex<StickerStore>>,
        app: AppHandle,
    ) -> Result<()> {
        if self.sending {
            return Err(anyhow!("已有飞书发送任务正在运行"));
        }
        self.sending = true;
        let outcome = self
            .send_batch_inner(records, destination, store, app)
            .await;
        self.sending = false;
        outcome
    }

    async fn send_batch_inner(
        &self,
        records: Vec<StickerRecord>,
        destination: FeishuDestination,
        store: Arc<Mutex<StickerStore>>,
        app: AppHandle,
    ) -> Result<()> {
        let (target_flag, target_id) = self.resolve_destination(destination).await?;
        let total = records.len();
        let mut sent = 0;
        let mut failed = 0;
        emit_progress(
            &app,
            FeishuSendProgress {
                current: 0,
                total,
                sticker_id: None,
                sent,
                failed,
                message: None,
                done: total == 0,
            },
        );

        for (index, record) in records.iter().enumerate() {
            let file_path = { store.lock().await.get_file_path(&record.id) };
            let Some(file_path) = file_path else {
                failed += 1;
                store
                    .lock()
                    .await
                    .mark_failed(&record.id, "本地表情文件不存在".into())?;
                emit_progress(
                    &app,
                    progress(
                        index + 1,
                        total,
                        record,
                        sent,
                        failed,
                        "本地表情文件不存在",
                        index + 1 == total,
                    ),
                );
                continue;
            };

            store.lock().await.mark_sending(&record.id)?;
            let filename = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&record.filename);
            let image_argument = format!("./{filename}");
            let idempotency = format!("wxsticker-{}", record.id);
            let arguments = [
                "im",
                "+messages-send",
                "--as",
                "user",
                target_flag,
                &target_id,
                "--image",
                &image_argument,
                "--idempotency-key",
                &idempotency,
                "--format",
                "json",
            ];
            let working_directory = file_path.parent().unwrap_or_else(|| Path::new("."));
            let result = self
                .run(&arguments, working_directory, Duration::from_secs(90))
                .await?;
            let message = if result.code == 0 {
                let payload = parse_json(&result.stdout);
                let message_id = find_deep_string(payload.as_ref(), &["message_id", "messageId"]);
                sent += 1;
                store.lock().await.mark_sent(&record.id, message_id)?;
                "已发送".to_string()
            } else {
                failed += 1;
                let message = error_text(&result);
                store
                    .lock()
                    .await
                    .mark_failed(&record.id, message.clone())?;
                message
            };
            emit_progress(
                &app,
                progress(
                    index + 1,
                    total,
                    record,
                    sent,
                    failed,
                    &message,
                    index + 1 == total,
                ),
            );
            if index + 1 < total {
                sleep(Duration::from_millis(450)).await;
            }
        }
        Ok(())
    }

    async fn resolve_destination(
        &self,
        destination: FeishuDestination,
    ) -> Result<(&'static str, String)> {
        match destination {
            FeishuDestination::SelfTarget => Ok(("--user-id", self.get_self().await?.open_id)),
            FeishuDestination::User { id } => {
                let id = id.trim().to_string();
                if id.is_empty() {
                    Err(anyhow!("飞书接收用户不能为空"))
                } else {
                    Ok(("--user-id", id))
                }
            }
            FeishuDestination::Chat { id } => {
                let id = id.trim().to_string();
                if id.is_empty() {
                    Err(anyhow!("飞书接收群聊不能为空"))
                } else {
                    Ok(("--chat-id", id))
                }
            }
        }
    }

    async fn latest_release_spec(&self) -> Result<ReleaseSpec> {
        let client = download_client()?;
        let metadata = client
            .get(NPM_LATEST_URL)
            .timeout(Duration::from_secs(20))
            .send()
            .await?
            .error_for_status()?
            .json::<NpmLatestMetadata>()
            .await?;
        let version = Version::parse(metadata.version.trim()).context("飞书官方版本号格式无效")?;
        let archive_name = release_archive_name(&version)?;
        let package_url = format!(
            "https://registry.npmjs.org/@larksuite/cli/-/cli-{}.tgz",
            version
        );
        let package = client
            .get(package_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if package.len() > MAX_METADATA_BYTES {
            return Err(anyhow!("飞书 CLI 校验元数据异常过大"));
        }
        let checksums = extract_checksums(&package)?;
        let checksum = checksum_for_archive(&checksums, &archive_name)
            .ok_or_else(|| anyhow!("飞书官方校验清单缺少 {archive_name}"))?;
        Ok(ReleaseSpec {
            version: version.clone(),
            archive_name: archive_name.clone(),
            checksum,
            github_url: format!("{GITHUB_RELEASE_BASE}/v{version}/{archive_name}"),
            mirror_url: format!("{NPM_MIRROR_BASE}/v{version}/{archive_name}"),
        })
    }

    async fn run(
        &self,
        arguments: &[&str],
        cwd: &Path,
        timeout_duration: Duration,
    ) -> Result<CliResult> {
        let executable = self
            .resolve_executable()
            .ok_or_else(|| anyhow!("尚未安装飞书官方连接组件"))?;
        self.run_with(&executable.path, arguments, cwd, timeout_duration)
            .await
    }

    async fn run_with(
        &self,
        executable: &Path,
        arguments: &[&str],
        cwd: &Path,
        timeout_duration: Duration,
    ) -> Result<CliResult> {
        let mut command = Command::new(executable);
        command.args(arguments).current_dir(cwd).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let output = timeout(timeout_duration, command.output())
            .await
            .map_err(|_| anyhow!("lark-cli 执行超时（{} 秒）", timeout_duration.as_secs()))??;
        Ok(CliResult {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    fn managed_executable(&self) -> PathBuf {
        self.managed_directory.join(cli_binary_name())
    }

    fn resolve_executable(&self) -> Option<CliLocation> {
        let managed = self.managed_executable();
        if is_executable_file(&managed) {
            return Some(CliLocation {
                path: managed,
                source: "managed",
            });
        }
        for path in &self.fallback_executables {
            if is_executable_file(path) {
                return Some(CliLocation {
                    path: path.clone(),
                    source: "bundled-legacy",
                });
            }
        }
        find_cli_in_path().map(|path| CliLocation {
            path,
            source: "path",
        })
    }
}

async fn download_verified_release_archive(
    release: &ReleaseSpec,
    app: &AppHandle,
) -> Result<Vec<u8>> {
    let client = download_client()?;
    let sources = [
        DownloadSource {
            label: "国内加速线路",
            url: &release.mirror_url,
        },
        DownloadSource {
            label: "GitHub Releases",
            url: &release.github_url,
        },
    ];
    let max_attempts = sources.len() * DOWNLOAD_ATTEMPTS_PER_SOURCE;
    let mut attempt_number = 0;
    let mut last_error = None;
    for (source_index, source) in sources.iter().enumerate() {
        for source_attempt in 1..=DOWNLOAD_ATTEMPTS_PER_SOURCE {
            attempt_number += 1;
            let attempt = DownloadAttempt {
                current: attempt_number,
                total: max_attempts,
                source: source.label,
            };
            emit_cli_progress_event(
                app,
                FeishuCliProgress {
                    stage: "downloading".to_string(),
                    downloaded: 0,
                    total: None,
                    message: format!("正在从 {} 下载飞书官方组件…", source.label),
                    done: false,
                    attempt: Some(attempt.current),
                    max_attempts: Some(attempt.total),
                    source: Some(attempt.source.to_string()),
                },
            );

            let result: Result<()> =
                match download_archive_from(&client, source.url, app, attempt).await {
                    Ok(bytes) => {
                        let archive_size = bytes.len() as u64;
                        emit_cli_progress_event(
                            app,
                            FeishuCliProgress {
                                stage: "verifying".to_string(),
                                downloaded: archive_size,
                                total: Some(archive_size),
                                message: "下载完成，正在核对官方 SHA-256…".to_string(),
                                done: false,
                                attempt: Some(attempt.current),
                                max_attempts: Some(attempt.total),
                                source: Some(attempt.source.to_string()),
                            },
                        );
                        let actual_checksum = hex::encode(Sha256::digest(&bytes));
                        if actual_checksum == release.checksum {
                            return Ok(bytes);
                        }
                        Err(anyhow!(
                            "{} 返回的文件 SHA-256 校验不一致（期望 {}，实际 {}）",
                            source.label,
                            release.checksum,
                            actual_checksum
                        ))
                    }
                    Err(error) => Err(error),
                };

            if let Err(error) = result {
                let error_message = error.to_string();
                last_error = Some(error);
                if attempt_number < max_attempts {
                    let delay_seconds = retry_delay_seconds(source_attempt);
                    let next_source = if source_attempt < DOWNLOAD_ATTEMPTS_PER_SOURCE {
                        source.label
                    } else {
                        sources
                            .get(source_index + 1)
                            .map(|next| next.label)
                            .unwrap_or(source.label)
                    };
                    emit_cli_progress_event(
                        app,
                        FeishuCliProgress {
                            stage: "retrying".to_string(),
                            downloaded: 0,
                            total: None,
                            message: format!(
                                "第 {attempt_number}/{max_attempts} 次失败：{error_message}。{delay_seconds} 秒后尝试 {next_source}…"
                            ),
                            done: false,
                            attempt: Some(attempt_number),
                            max_attempts: Some(max_attempts),
                            source: Some(source.label.to_string()),
                        },
                    );
                    sleep(Duration::from_secs(delay_seconds)).await;
                }
            }
        }
    }
    Err(anyhow!(
        "飞书 CLI 下载失败，已尝试 {max_attempts} 次：{}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "没有可用下载源".to_string())
    ))
}

fn retry_delay_seconds(source_attempt: usize) -> u64 {
    if source_attempt <= 1 {
        1
    } else {
        2
    }
}

async fn download_archive_from(
    client: &Client,
    url: &str,
    app: &AppHandle,
    attempt: DownloadAttempt,
) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(180))
        .send()
        .await?
        .error_for_status()?;
    if !is_allowed_download_url(response.url()) {
        return Err(anyhow!("下载被重定向到非官方地址"));
    }
    let total = response.content_length();
    if total.is_some_and(|value| value > MAX_ARCHIVE_BYTES as u64) {
        return Err(anyhow!("飞书 CLI 下载文件异常过大"));
    }
    let mut bytes =
        Vec::with_capacity(total.unwrap_or_default().min(MAX_ARCHIVE_BYTES as u64) as usize);
    let mut stream = response.bytes_stream();
    loop {
        let next_chunk = timeout(DOWNLOAD_IDLE_TIMEOUT, stream.next())
            .await
            .map_err(|_| anyhow!("下载连续 30 秒没有收到新数据，准备重试"))?;
        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = chunk?;
        if bytes.len() + chunk.len() > MAX_ARCHIVE_BYTES {
            return Err(anyhow!("飞书 CLI 下载文件超过安全大小限制"));
        }
        bytes.extend_from_slice(&chunk);
        emit_cli_progress_event(
            app,
            FeishuCliProgress {
                stage: "downloading".to_string(),
                downloaded: bytes.len() as u64,
                total,
                message: format!("正在从 {} 下载飞书官方组件…", attempt.source),
                done: false,
                attempt: Some(attempt.current),
                max_attempts: Some(attempt.total),
                source: Some(attempt.source.to_string()),
            },
        );
    }
    if bytes.is_empty() {
        return Err(anyhow!("飞书 CLI 下载结果为空"));
    }
    Ok(bytes)
}

fn download_client() -> Result<Client> {
    let redirect = Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || !is_allowed_download_url(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    Ok(Client::builder()
        .user_agent("sticker-relay/0.3.0")
        .redirect(redirect)
        .build()?)
}

fn is_allowed_download_url(url: &url::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if !matches!(url.port(), None | Some(443)) {
        return false;
    }
    matches!(
        url.host_str().unwrap_or_default(),
        "registry.npmjs.org"
            | "github.com"
            | "objects.githubusercontent.com"
            | "release-assets.githubusercontent.com"
            | "registry.npmmirror.com"
            | "cdn.npmmirror.com"
    )
}

pub fn validate_feishu_auth_url(raw: &str) -> Result<()> {
    let url = url::Url::parse(raw).context("飞书授权地址格式无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
    {
        return Err(anyhow!("飞书授权地址未通过安全校验"));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed = host == "feishu.cn"
        || host.ends_with(".feishu.cn")
        || host == "larksuite.com"
        || host.ends_with(".larksuite.com");
    if !allowed {
        return Err(anyhow!("飞书授权地址不属于飞书官方域名"));
    }
    Ok(())
}

fn release_archive_name(version: &Version) -> Result<String> {
    release_archive_name_for(version, std::env::consts::OS, std::env::consts::ARCH)
}

fn release_archive_name_for(version: &Version, os: &str, architecture: &str) -> Result<String> {
    let platform = match os {
        "windows" => "windows",
        "macos" => "darwin",
        _ => return Err(anyhow!("当前系统暂不支持自动安装飞书 CLI")),
    };
    let arch = match architecture {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return Err(anyhow!("当前 CPU 架构暂不支持自动安装飞书 CLI")),
    };
    let extension = if platform == "windows" {
        "zip"
    } else {
        "tar.gz"
    };
    Ok(format!("lark-cli-{version}-{platform}-{arch}.{extension}"))
}

fn extract_checksums(package: &[u8]) -> Result<String> {
    let decoder = GzDecoder::new(Cursor::new(package));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        if path.to_string_lossy().replace('\\', "/") == "package/checksums.txt" {
            let mut output = String::new();
            entry
                .take(MAX_METADATA_BYTES as u64)
                .read_to_string(&mut output)?;
            return Ok(output);
        }
    }
    Err(anyhow!("飞书官方 npm 包缺少 checksums.txt"))
}

fn checksum_for_archive(checksums: &str, archive_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let checksum = fields.next()?;
        let name = fields.next()?;
        if name == archive_name
            && checksum.len() == 64
            && checksum
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            Some(checksum.to_ascii_lowercase())
        } else {
            None
        }
    })
}

fn extract_cli_binary(archive_name: &str, archive: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    if archive_name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
        for index in 0..zip.len() {
            let entry = zip.by_index(index)?;
            let matches = Path::new(entry.name())
                .file_name()
                .and_then(|name| name.to_str())
                == Some(cli_binary_name());
            if matches {
                entry
                    .take(MAX_ARCHIVE_BYTES as u64)
                    .read_to_end(&mut output)?;
                break;
            }
        }
    } else if archive_name.ends_with(".tar.gz") {
        let decoder = GzDecoder::new(Cursor::new(archive));
        let mut tar = tar::Archive::new(decoder);
        for entry in tar.entries()? {
            let entry = entry?;
            let matches =
                entry.path()?.file_name().and_then(|name| name.to_str()) == Some(cli_binary_name());
            if matches {
                entry
                    .take(MAX_ARCHIVE_BYTES as u64)
                    .read_to_end(&mut output)?;
                break;
            }
        }
    } else {
        return Err(anyhow!("不支持的飞书 CLI 压缩格式"));
    }
    if output.len() < 1024 * 1024 {
        return Err(anyhow!("飞书 CLI 压缩包中没有有效的可执行文件"));
    }
    Ok(output)
}

fn persist_managed_binary(
    destination: &Path,
    version: &str,
    checksum: &str,
    binary: &[u8],
) -> Result<()> {
    let directory = destination
        .parent()
        .ok_or_else(|| anyhow!("飞书 CLI 安装目录无效"))?;
    fs::create_dir_all(directory)?;
    let temporary = destination.with_extension(format!("download-{}", std::process::id()));
    let backup = destination.with_extension("previous");
    fs::write(&temporary, binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
    }
    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if had_previous {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error).context("无法替换飞书 CLI 可执行文件");
    }
    if backup.exists() {
        let _ = fs::remove_file(backup);
    }
    let metadata = serde_json::to_vec_pretty(&serde_json::json!({
        "version": version,
        "sha256": checksum,
        "source": "https://github.com/larksuite/cli",
        "installedAt": now_millis(),
    }))?;
    fs::write(directory.join("component.json"), metadata)?;
    Ok(())
}

pub(crate) fn cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "lark-cli.exe"
    } else {
        "lark-cli"
    }
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn find_cli_in_path() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(cli_binary_name()))
        .find(|candidate| is_executable_file(candidate))
}

fn unavailable_status(detail: String) -> FeishuCliStatus {
    FeishuCliStatus {
        installed: false,
        version: None,
        authenticated: false,
        detail: Some(detail),
        source: None,
        executable_path: None,
        latest_version: None,
        update_available: false,
    }
}

fn progress(
    current: usize,
    total: usize,
    record: &StickerRecord,
    sent: usize,
    failed: usize,
    message: &str,
    done: bool,
) -> FeishuSendProgress {
    FeishuSendProgress {
        current,
        total,
        sticker_id: Some(record.id.clone()),
        sent,
        failed,
        message: Some(message.to_string()),
        done,
    }
}

fn emit_progress(app: &AppHandle, progress: FeishuSendProgress) {
    let _ = app.emit("feishu-progress", progress);
}

fn emit_cli_progress(
    app: &AppHandle,
    stage: &str,
    downloaded: u64,
    total: Option<u64>,
    message: &str,
    done: bool,
) {
    emit_cli_progress_event(
        app,
        FeishuCliProgress {
            stage: stage.to_string(),
            downloaded,
            total,
            message: message.to_string(),
            done,
            attempt: None,
            max_attempts: None,
            source: None,
        },
    );
}

fn emit_cli_progress_event(app: &AppHandle, progress: FeishuCliProgress) {
    let _ = app.emit("feishu-cli-progress", progress);
}

fn normalized_version(result: &CliResult) -> String {
    let raw = if result.stdout.trim().is_empty() {
        &result.stderr
    } else {
        &result.stdout
    };
    extract_version(raw).unwrap_or_else(|| raw.trim().to_string())
}

fn extract_version(raw: &str) -> Option<String> {
    regex::Regex::new(r"\b\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\b")
        .ok()?
        .find(raw)
        .map(|value| value.as_str().to_string())
}

fn error_text(result: &CliResult) -> String {
    let raw = if !result.stderr.trim().is_empty() {
        result.stderr.as_str()
    } else if !result.stdout.trim().is_empty() {
        result.stdout.as_str()
    } else {
        return format!("lark-cli 退出码 {}", result.code);
    };
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

fn parse_json(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        serde_json::from_str(&text[start..=end]).ok()
    })
}

fn find_deep_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let value = value?;
    if let Value::Object(map) = value {
        for key in keys {
            if let Some(Value::String(candidate)) = map.get(*key) {
                if !candidate.is_empty() {
                    return Some(candidate.clone());
                }
            }
        }
        for child in map.values() {
            if let Some(found) = find_deep_string(Some(child), keys) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for child in items {
            if let Some(found) = find_deep_string(Some(child), keys) {
                return Some(found);
            }
        }
    }
    None
}

fn find_deep_number(value: Option<&Value>, keys: &[&str]) -> Option<u64> {
    let value = value?;
    if let Value::Object(map) = value {
        for key in keys {
            if let Some(candidate) = map.get(*key) {
                if let Some(value) = candidate.as_u64() {
                    return Some(value);
                }
                if let Some(value) = candidate.as_str().and_then(|text| text.parse().ok()) {
                    return Some(value);
                }
            }
        }
        for child in map.values() {
            if let Some(found) = find_deep_number(Some(child), keys) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for child in items {
            if let Some(found) = find_deep_number(Some(child), keys) {
                return Some(found);
            }
        }
    }
    None
}

fn user_identity_state(value: Option<&Value>) -> (Option<bool>, Option<String>) {
    let Some(user) = value
        .and_then(|value| value.get("identities"))
        .and_then(|value| value.get("user"))
    else {
        return (None, None);
    };
    let available = user.get("available").and_then(Value::as_bool);
    let status = user
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let authenticated = available.or_else(|| {
        status.map(|value| matches!(value.as_str(), "ready" | "authenticated" | "logged_in"))
    });
    let detail = user
        .get("message")
        .or_else(|| user.get("hint"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (authenticated, detail)
}

fn parse_login_payload(value: Option<&Value>) -> Result<PendingLogin> {
    let device_code = find_deep_string(value, &["device_code", "deviceCode"])
        .ok_or_else(|| anyhow!("飞书连接组件未返回 device_code"))?;
    let verification_url = find_deep_string(
        value,
        &[
            "verification_uri_complete",
            "verificationUriComplete",
            "verification_url",
            "verificationUrl",
            "verification_uri",
            "verificationUri",
        ],
    )
    .filter(|url| validate_feishu_auth_url(url).is_ok())
    .ok_or_else(|| anyhow!("飞书连接组件未返回有效的官方授权地址，请稍后重试"))?;
    let user_code = find_deep_string(value, &["user_code", "userCode"]).or_else(|| {
        url::Url::parse(&verification_url)
            .ok()?
            .query_pairs()
            .find(|(key, _)| key == "user_code")
            .map(|(_, value)| value.to_string())
    });
    let expires_at = find_deep_number(value, &["expires_in", "expiresIn"])
        .map(|seconds| now_millis() + seconds * 1000);
    Ok(PendingLogin {
        device_code,
        session: FeishuLoginSession {
            verification_url,
            user_code,
            expires_at,
        },
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        checksum_for_archive, cli_binary_name, extract_version, is_allowed_download_url,
        migrate_managed_component, release_archive_name_for, retry_delay_seconds,
        validate_feishu_auth_url, DOWNLOAD_ATTEMPTS_PER_SOURCE,
    };
    use semver::Version;
    use std::{fs, time::SystemTime};

    #[test]
    fn extracts_cli_version() {
        assert_eq!(
            extract_version("lark-cli version 1.0.90"),
            Some("1.0.90".to_string())
        );
    }

    #[test]
    fn validates_only_official_auth_domains() {
        assert!(validate_feishu_auth_url("https://accounts.feishu.cn/open").is_ok());
        assert!(validate_feishu_auth_url("https://open.larksuite.com/auth").is_ok());
        assert!(validate_feishu_auth_url("https://feishu.cn.evil.example/auth").is_err());
        assert!(validate_feishu_auth_url("http://accounts.feishu.cn/auth").is_err());
    }

    #[test]
    fn reads_expected_checksum() {
        let checksum = "a".repeat(64);
        let manifest = format!("{checksum}  lark-cli-1.0.90-windows-amd64.zip\n");
        assert_eq!(
            checksum_for_archive(&manifest, "lark-cli-1.0.90-windows-amd64.zip"),
            Some(checksum)
        );
    }

    #[test]
    fn retries_each_download_source_with_bounded_backoff() {
        assert_eq!(DOWNLOAD_ATTEMPTS_PER_SOURCE, 2);
        assert_eq!(retry_delay_seconds(1), 1);
        assert_eq!(retry_delay_seconds(2), 2);
    }

    #[test]
    fn allows_the_supported_domestic_mirror_redirect() {
        assert!(is_allowed_download_url(
            &url::Url::parse("https://registry.npmmirror.com/-/binary/lark-cli/file.zip")
                .expect("valid registry URL")
        ));
        assert!(is_allowed_download_url(
            &url::Url::parse("https://cdn.npmmirror.com/binaries/lark-cli/file.zip")
                .expect("valid CDN URL")
        ));
        assert!(!is_allowed_download_url(
            &url::Url::parse("https://cdn.npmmirror.com.evil.example/file.zip")
                .expect("valid hostile URL")
        ));
    }

    #[test]
    fn selects_the_official_archive_for_each_supported_desktop_target() {
        let version = Version::parse("1.0.90").expect("valid version");
        assert_eq!(
            release_archive_name_for(&version, "windows", "x86_64").expect("Windows x64 archive"),
            "lark-cli-1.0.90-windows-amd64.zip"
        );
        assert_eq!(
            release_archive_name_for(&version, "windows", "aarch64")
                .expect("Windows ARM64 archive"),
            "lark-cli-1.0.90-windows-arm64.zip"
        );
        assert_eq!(
            release_archive_name_for(&version, "macos", "x86_64").expect("macOS Intel archive"),
            "lark-cli-1.0.90-darwin-amd64.tar.gz"
        );
        assert_eq!(
            release_archive_name_for(&version, "macos", "aarch64")
                .expect("macOS Apple Silicon archive"),
            "lark-cli-1.0.90-darwin-arm64.tar.gz"
        );
    }

    #[test]
    fn rejects_unsupported_cli_targets() {
        let version = Version::parse("1.0.90").expect("valid version");
        assert!(release_archive_name_for(&version, "linux", "x86_64").is_err());
        assert!(release_archive_name_for(&version, "windows", "x86").is_err());
    }

    #[test]
    fn migrates_the_managed_cli_to_its_persistent_directory() {
        let root = std::env::temp_dir().join(format!(
            "sticker-relay-cli-migration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let source = root.join("app-data").join("components").join("lark-cli");
        let destination = root.join("persistent-components").join("lark-cli");
        fs::create_dir_all(&source).expect("create source directory");
        fs::write(source.join(cli_binary_name()), b"old managed cli")
            .expect("write source executable");
        fs::write(source.join("component.json"), b"{\"version\":\"1.0.90\"}")
            .expect("write source metadata");

        assert!(migrate_managed_component(&source, &destination).expect("migrate component"));
        assert_eq!(
            fs::read(destination.join(cli_binary_name())).expect("read migrated executable"),
            b"old managed cli"
        );
        assert!(destination.join("component.json").is_file());
        assert!(!source.join(cli_binary_name()).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_an_existing_persistent_cli() {
        let root = std::env::temp_dir().join(format!(
            "sticker-relay-cli-existing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let source = root.join("old");
        let destination = root.join("persistent");
        fs::create_dir_all(&source).expect("create source directory");
        fs::create_dir_all(&destination).expect("create destination directory");
        fs::write(source.join(cli_binary_name()), b"old cli").expect("write old executable");
        fs::write(destination.join(cli_binary_name()), b"current cli")
            .expect("write current executable");

        assert!(!migrate_managed_component(&source, &destination).expect("skip migration"));
        assert_eq!(
            fs::read(destination.join(cli_binary_name())).expect("read current executable"),
            b"current cli"
        );
        assert!(source.join(cli_binary_name()).is_file());
        let _ = fs::remove_dir_all(root);
    }
}
