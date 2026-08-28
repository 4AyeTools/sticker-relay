use std::{
    collections::HashSet,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock as StdRwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use cookie_store::{CookieStore as SerializableCookieStore, RawCookie};
use rand::Rng;
use regex::Regex;
use reqwest::{
    cookie::CookieStore as ReqwestCookieStore,
    header::{
        HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, ORIGIN, REFERER,
    },
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
    time::sleep,
};
use url::Url;

use crate::{
    models::{StickerImportEvent, SyncKeyItem, WechatLoginState, WechatQrResult},
    secure_storage,
    store::{sniff_image_mime, AddResult, StickerStore},
};

const FILEHELPER_ORIGIN: &str = "https://szfilehelper.weixin.qq.com";
const NEW_LOGIN_PAGE: &str =
    "https://szfilehelper.weixin.qq.com/cgi-bin/mmwebwx-bin/webwxnewloginpage";
const LOGOUT_URL: &str = "https://szfilehelper.weixin.qq.com/cgi-bin/mmwebwx-bin/webwxlogout";
const LOGIN_ORIGIN: &str = "https://login.wx2.qq.com";
const QR_ORIGIN: &str = "https://login.weixin.qq.com";
const APP_ID: &str = "wx_webfilehelper";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const SESSION_FILE_VERSION: u8 = 1;
const SESSION_SAVE_INTERVAL: Duration = Duration::from_secs(30);
const PRODUCT_STICKER_UNSUPPORTED_TIP: &str = "这是微信专辑或表情商店表情。微信网页版只返回占位消息，没有提供图片链接，因此无法自动迁移。请在手机端截图或保存为普通图片，发送到飞书后点击“＋ 添加表情”；动态表情需要先转换为 GIF。";

#[derive(Clone, Serialize, Deserialize)]
struct WechatSession {
    uin: String,
    sid: String,
    skey: String,
    pass_ticket: String,
    device_id: String,
    sync_key: Vec<SyncKeyItem>,
}

#[derive(Serialize, Deserialize)]
struct PersistedWechatState {
    version: u8,
    session: WechatSession,
    cookies: String,
}

#[derive(Default)]
struct PersistentCookieStore {
    inner: StdRwLock<SerializableCookieStore>,
}

impl PersistentCookieStore {
    fn from_json(value: &str) -> Result<Self> {
        let store = cookie_store::serde::json::load(Cursor::new(value.as_bytes()))
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self {
            inner: StdRwLock::new(store),
        })
    }

    fn to_json(&self) -> Result<String> {
        let store = self
            .inner
            .read()
            .map_err(|_| anyhow!("微信 Cookie 存储已损坏"))?;
        let mut output = Vec::new();
        cookie_store::serde::json::save_incl_expired_and_nonpersistent(&store, &mut output)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(String::from_utf8(output)?)
    }
}

impl ReqwestCookieStore for PersistentCookieStore {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let cookies = cookie_headers.filter_map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| RawCookie::parse(value.to_owned()).ok())
                .map(RawCookie::into_owned)
        });
        if let Ok(mut store) = self.inner.write() {
            store.store_response_cookies(cookies, url);
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let store = self.inner.read().ok()?;
        let value = store
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if value.is_empty() {
            None
        } else {
            HeaderValue::from_str(&value).ok()
        }
    }
}

struct HttpState {
    client: Client,
    jar: Arc<PersistentCookieStore>,
}

pub struct WechatCollector {
    http: Arc<RwLock<HttpState>>,
    session: Arc<Mutex<Option<WechatSession>>>,
    seen: Arc<Mutex<HashSet<String>>>,
    scan_task: Mutex<Option<JoinHandle<()>>>,
    session_path: PathBuf,
    restoring: Arc<AtomicBool>,
}

impl WechatCollector {
    pub fn new(session_path: PathBuf) -> Result<Self> {
        let restored = match load_persisted_state(&session_path) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("无法恢复微信登录状态，将重新登录：{error}");
                let _ = secure_storage::remove(&session_path);
                None
            }
        };
        let has_restored_session = restored.is_some();
        let (http, session) = match restored {
            Some((http, session)) => (http, Some(session)),
            None => (create_http_state(None)?, None),
        };
        Ok(Self {
            http: Arc::new(RwLock::new(http)),
            session: Arc::new(Mutex::new(session)),
            seen: Arc::new(Mutex::new(HashSet::new())),
            scan_task: Mutex::new(None),
            session_path,
            restoring: Arc::new(AtomicBool::new(has_restored_session)),
        })
    }

    pub async fn restore(&self, app: AppHandle, store: Arc<Mutex<StickerStore>>) -> Result<()> {
        if self.session.lock().await.is_none() {
            self.restoring.store(false, Ordering::Release);
            return Ok(());
        }

        match scan_once(&self.http, &self.session, &self.seen, &store, &app).await {
            Ok(ScanOutcome::Active) => {
                self.restoring.store(false, Ordering::Release);
                if let Err(error) = self.persist_state().await {
                    emit_session_warning(&app, &format!("恢复成功，但刷新登录状态失败：{error}"));
                }
                self.start_scanner(app, store).await;
            }
            Ok(ScanOutcome::SessionExpired(_)) => {
                self.reset_local_state().await?;
                emit_session_warning(&app, "保存的微信登录已失效，请重新扫码。");
            }
            Err(error) => {
                self.restoring.store(false, Ordering::Release);
                emit_session_warning(&app, &format!("已恢复微信登录，暂时无法校验连接：{error}"));
                self.start_scanner(app, store).await;
            }
        }
        Ok(())
    }

    pub async fn status(&self) -> WechatLoginState {
        if self.restoring.load(Ordering::Acquire) {
            return login_state("restoring");
        }
        if self.session.lock().await.is_some() {
            login_state("logged-in")
        } else {
            login_state("idle")
        }
    }

    pub async fn prepare_exit(&self) -> Result<()> {
        self.persist_state().await
    }

    pub async fn request_qr(&self) -> Result<WechatQrResult> {
        self.logout().await?;
        let encoded_login_page = urlencoding::encode(NEW_LOGIN_PAGE);
        let redirect_uri = urlencoding::encode(encoded_login_page.as_ref());
        let login_url = format!(
            "{LOGIN_ORIGIN}/jslogin?appid={APP_ID}&redirect_uri={redirect_uri}&fun=new&lang=zh_CN&_={}",
            now_millis()
        );
        let client = self.client().await;
        let response = client.get(login_url).headers(headers(false)).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!("获取微信二维码失败（{}）", response.status()));
        }
        let text = response.text().await?;
        if !text.contains("window.QRLogin.code = 200") {
            return Err(anyhow!("微信未返回有效登录 UUID"));
        }
        let uuid = parse_script_string(&text, "window.QRLogin.uuid")
            .ok_or_else(|| anyhow!("无法解析微信登录 UUID"))?;
        if !Regex::new(r"^[A-Za-z0-9_+/=-]+$")?.is_match(&uuid) {
            return Err(anyhow!("微信登录 UUID 格式无效"));
        }
        let qr_response = client
            .get(format!("{QR_ORIGIN}/qrcode/{uuid}"))
            .headers(headers(false))
            .send()
            .await?;
        if !qr_response.status().is_success() {
            return Err(anyhow!("下载微信二维码失败（{}）", qr_response.status()));
        }
        let bytes = qr_response.bytes().await?;
        let mime_type = sniff_image_mime(&bytes)
            .filter(|_| bytes.len() >= 100)
            .ok_or_else(|| anyhow!("微信二维码响应不是有效图片，请稍后刷新"))?;
        Ok(WechatQrResult {
            uuid,
            data_url: format!("data:{mime_type};base64,{}", STANDARD.encode(bytes)),
        })
    }

    pub async fn poll(
        &self,
        uuid: &str,
        tip: u8,
        app: AppHandle,
        store: Arc<Mutex<StickerStore>>,
    ) -> Result<WechatLoginState> {
        let url = format!(
            "{LOGIN_ORIGIN}/cgi-bin/mmwebwx-bin/login?loginicon=true&uuid={}&tip={tip}&appid={APP_ID}&_={}",
            urlencoding::encode(uuid),
            now_millis()
        );
        let response = self
            .client()
            .await
            .get(url)
            .headers(headers(false))
            .timeout(Duration::from_secs(35))
            .send()
            .await;
        let Ok(response) = response else {
            return Ok(login_state("waiting"));
        };
        if !response.status().is_success() {
            return Ok(login_state("waiting"));
        }
        let text = response.text().await?;
        match parse_script_number(&text) {
            Some(201) => return Ok(login_state("scanned")),
            Some(400) => return Ok(login_state("expired")),
            Some(200) => {}
            _ => return Ok(login_state("waiting")),
        }

        let redirect_uri = parse_script_string(&text, "window.redirect_uri")
            .ok_or_else(|| anyhow!("微信登录成功，但未返回会话地址"))?;
        let session = self.establish_session(&redirect_uri).await?;
        *self.session.lock().await = Some(session);
        self.restoring.store(false, Ordering::Release);
        if let Err(error) = self.persist_state().await {
            emit_session_warning(&app, &format!("微信已连接，但登录状态保存失败：{error}"));
        }
        self.start_scanner(app, store).await;
        Ok(login_state("logged-in"))
    }

    pub async fn logout(&self) -> Result<()> {
        if let Some(task) = self.scan_task.lock().await.take() {
            task.abort();
        }
        if let Err(error) = self.remote_logout().await {
            eprintln!("微信服务端退出失败，仍会清理本地登录状态：{error}");
        }
        self.reset_local_state().await
    }

    pub async fn forget_sticker_md5s(&self, md5s: &[String]) {
        let mut seen = self.seen.lock().await;
        for md5 in md5s {
            seen.remove(&format!("md5:{md5}"));
        }
    }

    async fn establish_session(&self, redirect_uri: &str) -> Result<WechatSession> {
        let source = Url::parse(redirect_uri)?;
        let mut login_page = Url::parse(NEW_LOGIN_PAGE)?;
        for (key, value) in source.query_pairs() {
            login_page.query_pairs_mut().append_pair(&key, &value);
        }
        let existing: HashSet<String> = login_page
            .query_pairs()
            .map(|(key, _)| key.to_string())
            .collect();
        {
            let mut pairs = login_page.query_pairs_mut();
            if !existing.contains("fun") {
                pairs.append_pair("fun", "new");
            }
            if !existing.contains("version") {
                pairs.append_pair("version", "v2");
            }
            if !existing.contains("lang") {
                pairs.append_pair("lang", "zh_CN");
            }
        }

        let client = self.client().await;
        let response = client
            .get(login_page)
            .headers(headers(false))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("建立微信会话失败（{}）", response.status()));
        }
        let xml = response.text().await?;
        let ret = parse_xml_field(&xml, "ret");
        if !ret.is_empty() && ret != "0" {
            return Err(anyhow!("微信登录返回错误（{ret}）"));
        }
        let mut session = WechatSession {
            uin: parse_xml_field(&xml, "wxuin"),
            sid: parse_xml_field(&xml, "wxsid"),
            skey: parse_xml_field(&xml, "skey"),
            pass_ticket: parse_xml_field(&xml, "pass_ticket"),
            device_id: create_device_id(),
            sync_key: Vec::new(),
        };
        if session.uin.is_empty()
            || session.sid.is_empty()
            || session.skey.is_empty()
            || session.pass_ticket.is_empty()
        {
            return Err(anyhow!("微信登录凭证不完整，请重新扫码"));
        }

        let mut init_url = Url::parse(&format!(
            "{FILEHELPER_ORIGIN}/cgi-bin/mmwebwx-bin/webwxinit"
        ))?;
        init_url
            .query_pairs_mut()
            .append_pair("r", &format!("{}", !(now_millis() as i64)))
            .append_pair("lang", "zh_CN")
            .append_pair("pass_ticket", &session.pass_ticket)
            .append_pair("skey", &session.skey);
        let response = client
            .post(init_url)
            .headers(headers(true))
            .json(&json!({ "BaseRequest": base_request(&session) }))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("初始化微信会话失败（{}）", response.status()));
        }
        let data: Value = response.json().await?;
        if data
            .pointer("/BaseResponse/Ret")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            != 0
        {
            return Err(anyhow!("微信初始化返回错误"));
        }
        session.sync_key = serde_json::from_value(
            data.pointer("/SyncKey/List")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )?;
        if session.sync_key.is_empty() {
            return Err(anyhow!("微信未返回消息同步凭证"));
        }
        Ok(session)
    }

    async fn start_scanner(&self, app: AppHandle, store: Arc<Mutex<StickerStore>>) {
        if let Some(task) = self.scan_task.lock().await.take() {
            task.abort();
        }
        let http = self.http.clone();
        let session = self.session.clone();
        let seen = self.seen.clone();
        let session_path = self.session_path.clone();
        let restoring = self.restoring.clone();
        let task = tokio::spawn(async move {
            let mut last_persisted = Instant::now();
            let mut last_warning_at: Option<Instant> = None;
            loop {
                match scan_once(&http, &session, &seen, &store, &app).await {
                    Ok(ScanOutcome::Active) => {
                        if last_persisted.elapsed() >= SESSION_SAVE_INTERVAL {
                            if let Err(error) =
                                persist_snapshot(&http, &session, &session_path).await
                            {
                                emit_session_warning(
                                    &app,
                                    &format!("保存微信登录状态失败：{error}"),
                                );
                            }
                            last_persisted = Instant::now();
                        }
                    }
                    Ok(ScanOutcome::SessionExpired(ret)) => {
                        if let Err(error) =
                            reset_runtime_state(&http, &session, &seen, &session_path, &restoring)
                                .await
                        {
                            emit_session_warning(&app, &format!("清理失效登录状态失败：{error}"));
                        }
                        emit_session_warning(
                            &app,
                            &format!("微信登录已失效（{ret}），请重新扫码。"),
                        );
                        break;
                    }
                    Err(error) => {
                        let should_warn = last_warning_at
                            .map(|value| value.elapsed() >= SESSION_SAVE_INTERVAL)
                            .unwrap_or(true);
                        if should_warn {
                            emit_session_warning(&app, &error.to_string());
                            last_warning_at = Some(Instant::now());
                        }
                    }
                }
                sleep(Duration::from_millis(1500)).await;
            }
        });
        *self.scan_task.lock().await = Some(task);
    }

    async fn client(&self) -> Client {
        self.http.read().await.client.clone()
    }

    async fn persist_state(&self) -> Result<()> {
        persist_snapshot(&self.http, &self.session, &self.session_path).await
    }

    async fn reset_local_state(&self) -> Result<()> {
        reset_runtime_state(
            &self.http,
            &self.session,
            &self.seen,
            &self.session_path,
            &self.restoring,
        )
        .await
    }

    async fn remote_logout(&self) -> Result<()> {
        let Some(session) = self.session.lock().await.clone() else {
            return Ok(());
        };
        let mut url = Url::parse(LOGOUT_URL)?;
        url.query_pairs_mut()
            .append_pair("redirect", "1")
            .append_pair("type", "0")
            .append_pair("skey", &session.skey);
        let response = self
            .client()
            .await
            .post(url)
            .headers(headers(false))
            .form(&[("sid", session.sid), ("uin", session.uin)])
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("微信退出接口返回 {}", response.status()));
        }
        Ok(())
    }
}

enum ScanOutcome {
    Active,
    SessionExpired(i64),
}

async fn scan_once(
    http: &Arc<RwLock<HttpState>>,
    session_state: &Arc<Mutex<Option<WechatSession>>>,
    seen: &Arc<Mutex<HashSet<String>>>,
    store: &Arc<Mutex<StickerStore>>,
    app: &AppHandle,
) -> Result<ScanOutcome> {
    let Some(session) = session_state.lock().await.clone() else {
        return Ok(ScanOutcome::SessionExpired(0));
    };
    let mut sync_url = Url::parse(&format!(
        "{FILEHELPER_ORIGIN}/cgi-bin/mmwebwx-bin/webwxsync"
    ))?;
    sync_url
        .query_pairs_mut()
        .append_pair("sid", &session.sid)
        .append_pair("skey", &session.skey)
        .append_pair("pass_ticket", &session.pass_ticket);
    let client = http.read().await.client.clone();
    let response = client
        .post(sync_url)
        .headers(headers(true))
        .json(&json!({
            "BaseRequest": base_request(&session),
            "SyncKey": { "Count": session.sync_key.len(), "List": session.sync_key },
            "rr": -1
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Ok(ScanOutcome::Active);
    }
    let data: Value = response.json().await?;
    let ret = data
        .pointer("/BaseResponse/Ret")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("微信同步响应缺少状态码"))?;
    if ret != 0 {
        return Ok(ScanOutcome::SessionExpired(ret));
    }
    if let Some(list) = data.pointer("/SyncKey/List") {
        let next: Vec<SyncKeyItem> = serde_json::from_value(list.clone()).unwrap_or_default();
        if !next.is_empty() {
            if let Some(current) = session_state.lock().await.as_mut() {
                current.sync_key = next;
            }
        }
    }
    let mut messages = Vec::new();
    if let Some(items) = data.get("AddMsgList").and_then(Value::as_array) {
        messages.extend(items.iter().cloned());
    }
    if let Some(items) = data.get("MsgList").and_then(Value::as_array) {
        messages.extend(items.iter().cloned());
    }
    process_messages(messages, &client, &session, seen, store, app).await?;
    Ok(ScanOutcome::Active)
}

async fn process_messages(
    messages: Vec<Value>,
    client: &Client,
    session: &WechatSession,
    seen: &Arc<Mutex<HashSet<String>>>,
    store: &Arc<Mutex<StickerStore>>,
    app: &AppHandle,
) -> Result<()> {
    let mut imported = 0;
    let mut skipped = 0;
    let mut unsupported = 0;
    let mut latest = None;
    let mut download_warning = None;

    for message in messages {
        if message.get("ToUserName").and_then(Value::as_str) != Some("filehelper") {
            continue;
        }
        let content = message
            .get("Content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message_id = message
            .get("NewMsgId")
            .or_else(|| message.get("MsgId"))
            .map(value_to_string);
        let msg_type = message
            .get("MsgType")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let is_product_sticker = is_product_sticker_message(&message, content);
        if msg_type != 47 && !is_product_sticker {
            continue;
        }
        if content.is_empty() && message_id.is_none() {
            continue;
        }

        let md5 = extract_emoji_md5(content);
        let key = md5
            .as_ref()
            .map(|value| format!("md5:{value}"))
            .or_else(|| message_id.as_ref().map(|value| format!("message:{value}")))
            .unwrap_or_else(|| content.chars().take(120).collect());
        if seen.lock().await.contains(&key) {
            continue;
        }
        if is_product_sticker {
            seen.lock().await.insert(key);
            unsupported += 1;
            if download_warning.is_none() {
                download_warning = Some(PRODUCT_STICKER_UNSUPPORTED_TIP.to_string());
            }
            continue;
        }
        let urls = sticker_download_urls(content, message_id.as_deref(), &session.skey);
        if urls.is_empty() {
            seen.lock().await.insert(key);
            unsupported += 1;
            continue;
        }

        let download = download_first_valid(client, urls, md5, store).await?;
        match download.result {
            Some(AddResult {
                added: true,
                record: Some(record),
            }) => {
                imported += 1;
                latest = Some(record);
                seen.lock().await.insert(key);
            }
            Some(AddResult {
                record: Some(_), ..
            }) => {
                skipped += 1;
                seen.lock().await.insert(key);
            }
            _ => {
                unsupported += 1;
                if download_warning.is_none() {
                    let img_status = message
                        .get("ImgStatus")
                        .and_then(Value::as_i64)
                        .unwrap_or_default();
                    let has_product_id = message
                        .get("HasProductId")
                        .and_then(Value::as_i64)
                        .unwrap_or_default();
                    download_warning = download.diagnostic.map(|diagnostic| {
                        format!(
                            "微信表情下载失败（消息类型 {msg_type}，图片状态 {img_status}，专辑标记 {has_product_id}）：{diagnostic}"
                        )
                    });
                }
            }
        }
    }

    if imported > 0 || skipped > 0 || unsupported > 0 {
        let _ = app.emit(
            "stickers-imported",
            StickerImportEvent {
                imported,
                skipped,
                unsupported,
                latest,
                warning: download_warning,
            },
        );
    }
    Ok(())
}

struct StickerDownloadOutcome {
    result: Option<AddResult>,
    diagnostic: Option<String>,
}

async fn download_first_valid(
    client: &Client,
    urls: Vec<String>,
    wechat_md5: Option<String>,
    store: &Arc<Mutex<StickerStore>>,
) -> Result<StickerDownloadOutcome> {
    let mut diagnostics = Vec::new();
    for url in urls {
        let source = sticker_download_source_label(&url);
        let response = match client
            .get(&url)
            .headers(headers(false))
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                diagnostics.push(format!("{source}返回 HTTP {}", response.status().as_u16()));
                continue;
            }
            Err(error) if error.is_timeout() => {
                diagnostics.push(format!("{source}请求超时"));
                continue;
            }
            Err(_) => {
                diagnostics.push(format!("{source}请求失败"));
                continue;
            }
        };
        let mime_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => {
                diagnostics.push(format!("{source}响应读取失败"));
                continue;
            }
        };
        if sniff_image_mime(&bytes).is_none() {
            diagnostics.push(format!(
                "{source}返回非图片内容（{}，{} 字节）",
                mime_type.as_deref().unwrap_or("未知类型"),
                bytes.len()
            ));
            continue;
        }
        let source_url = sanitize_sticker_source_url(&url);
        let result = store.lock().await.add_buffer(
            &bytes,
            mime_type.as_deref(),
            wechat_md5.clone(),
            Some(source_url),
        )?;
        if result.record.is_some() {
            return Ok(StickerDownloadOutcome {
                result: Some(result),
                diagnostic: None,
            });
        }
        diagnostics.push(format!("{source}图片无法写入本地表情库"));
    }
    Ok(StickerDownloadOutcome {
        result: None,
        diagnostic: (!diagnostics.is_empty()).then(|| diagnostics.join("；")),
    })
}

fn sticker_download_source_label(url: &str) -> &'static str {
    if !url.contains("/webwxgetmsgimg") {
        return "微信 CDN";
    }
    let Ok(url) = Url::parse(url) else {
        return "微信消息图片接口";
    };
    let mut has_official_message_id = false;
    let mut is_big = false;
    for (name, value) in url.query_pairs() {
        if name == "MsgID" {
            has_official_message_id = true;
        } else if name == "type" && value == "big" {
            is_big = true;
        }
    }
    if has_official_message_id && is_big {
        "微信表情原图接口"
    } else if has_official_message_id {
        "微信消息图片接口"
    } else {
        "微信旧版图片接口"
    }
}

fn create_http_state(jar: Option<Arc<PersistentCookieStore>>) -> Result<HttpState> {
    let jar = jar.unwrap_or_else(|| Arc::new(PersistentCookieStore::default()));
    let client = Client::builder()
        .cookie_provider(jar.clone())
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    Ok(HttpState { client, jar })
}

fn load_persisted_state(path: &Path) -> Result<Option<(HttpState, WechatSession)>> {
    if !path.exists() {
        return Ok(None);
    }
    let plaintext = secure_storage::read_encrypted(path)?;
    let persisted: PersistedWechatState = serde_json::from_slice(&plaintext)?;
    if persisted.version != SESSION_FILE_VERSION {
        return Err(anyhow!("不支持的微信登录状态版本：{}", persisted.version));
    }
    let jar = Arc::new(PersistentCookieStore::from_json(&persisted.cookies)?);
    Ok(Some((create_http_state(Some(jar))?, persisted.session)))
}

async fn persist_snapshot(
    http: &Arc<RwLock<HttpState>>,
    session_state: &Arc<Mutex<Option<WechatSession>>>,
    session_path: &Path,
) -> Result<()> {
    let Some(session) = session_state.lock().await.clone() else {
        return secure_storage::remove(session_path);
    };
    let cookies = http.read().await.jar.to_json()?;
    let payload = serde_json::to_vec(&PersistedWechatState {
        version: SESSION_FILE_VERSION,
        session,
        cookies,
    })?;
    secure_storage::write_encrypted(session_path, &payload)
}

async fn reset_runtime_state(
    http: &Arc<RwLock<HttpState>>,
    session: &Arc<Mutex<Option<WechatSession>>>,
    seen: &Arc<Mutex<HashSet<String>>>,
    session_path: &Path,
    restoring: &Arc<AtomicBool>,
) -> Result<()> {
    *session.lock().await = None;
    seen.lock().await.clear();
    *http.write().await = create_http_state(None)?;
    restoring.store(false, Ordering::Release);
    secure_storage::remove(session_path)
}

fn emit_session_warning(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "stickers-imported",
        StickerImportEvent {
            imported: 0,
            skipped: 0,
            unsupported: 0,
            latest: None,
            warning: Some(message.to_string()),
        },
    );
}

fn headers(json: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(if json {
            "application/json, text/plain, */*"
        } else {
            "*/*"
        }),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://szfilehelper.weixin.qq.com/"),
    );
    headers.insert(ORIGIN, HeaderValue::from_static(FILEHELPER_ORIGIN));
    headers.insert(
        HeaderName::from_static("mmweb_appid"),
        HeaderValue::from_static(APP_ID),
    );
    if json {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json;charset=UTF-8"),
        );
    }
    headers
}

fn base_request(session: &WechatSession) -> Value {
    json!({
        "Uin": session.uin.parse::<u64>().unwrap_or_default(),
        "Sid": session.sid,
        "Skey": session.skey,
        "DeviceID": session.device_id,
    })
}

fn login_state(state: &str) -> WechatLoginState {
    WechatLoginState {
        state: state.to_string(),
    }
}

fn parse_script_number(text: &str) -> Option<i64> {
    Regex::new(r"window\.code\s*=\s*(\d+)")
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn parse_script_string(text: &str, field: &str) -> Option<String> {
    let pattern = format!(r#"{}\s*=\s*"([^"]+)""#, regex::escape(field));
    Regex::new(&pattern)
        .ok()?
        .captures(text)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_xml_field(xml: &str, tag: &str) -> String {
    let escaped = regex::escape(tag);
    let cdata = Regex::new(&format!(
        r"(?s)<{escaped}><!\[CDATA\[(.*?)\]\]></{escaped}>"
    ))
    .ok();
    let plain = Regex::new(&format!(r"<{escaped}>([^<]*)</{escaped}>")).ok();
    let value = cdata
        .as_ref()
        .and_then(|regex| regex.captures(xml))
        .and_then(|captures| captures.get(1))
        .or_else(|| {
            plain
                .as_ref()
                .and_then(|regex| regex.captures(xml))
                .and_then(|captures| captures.get(1))
        })
        .map(|value| value.as_str())
        .unwrap_or_default();
    urlencoding::decode(value)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn decode_sticker_xml_text(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn normalize_sticker_url(raw: &str) -> String {
    let decoded = decode_sticker_xml_text(raw).replace("\\\"", "");
    decoded.trim().trim_end_matches('\\').to_string()
}

fn is_sticker_asset_url(raw: &str) -> bool {
    let value = normalize_sticker_url(raw).to_lowercase();
    (value.starts_with("http://") || value.starts_with("https://"))
        && (value.contains(".qq.com")
            || value.contains(".qpic.cn")
            || value.contains("/webwxgetmsgimg")
            || value.contains("/emoji")
            || value.contains("/emotion"))
}

fn extract_emoji_urls(content: &str) -> Vec<String> {
    let decoded = decode_sticker_xml_text(content);
    let mut urls = HashSet::new();
    if let Ok(attributes) =
        Regex::new(r#"(?i)(?:cdnurl|encrypturl|externurl|thumburl)\s*=\s*\\?"([^"\\]+)\\?""#)
    {
        for capture in attributes.captures_iter(&decoded) {
            if let Some(value) = capture.get(1) {
                let value = normalize_sticker_url(value.as_str());
                if is_sticker_asset_url(&value) {
                    urls.insert(value);
                }
            }
        }
    }
    if let Ok(all_urls) = Regex::new(r#"https?://[^\s"'<>]+"#) {
        for value in all_urls.find_iter(&decoded) {
            let value = normalize_sticker_url(value.as_str());
            if is_sticker_asset_url(&value) {
                urls.insert(value);
            }
        }
    }
    urls.into_iter().collect()
}

fn extract_emoji_md5(content: &str) -> Option<String> {
    let decoded = decode_sticker_xml_text(content);
    Regex::new(r#"(?i)md5\s*=\s*\\?"([a-f0-9]{32})\\?""#)
        .ok()?
        .captures(&decoded)?
        .get(1)
        .map(|value| value.as_str().to_lowercase())
}

fn is_product_sticker_message(message: &Value, content: &str) -> bool {
    content.contains("该类型暂不支持")
        || (message.get("MsgType").and_then(Value::as_i64) == Some(1)
            && message.get("ImgStatus").and_then(Value::as_i64) == Some(2)
            && message.get("HasProductId").and_then(Value::as_i64) == Some(1))
}

fn message_image_urls(message_id: &str, skey: &str) -> Vec<String> {
    let endpoint = format!("{FILEHELPER_ORIGIN}/cgi-bin/mmwebwx-bin/webwxgetmsgimg");
    let mut urls = Vec::new();

    // This is the request shape used by the current WeChat File Transfer
    // frontend for MsgType=47. In particular, `MsgID` is case-sensitive and
    // stickers require `type=big` rather than the image thumbnail endpoint.
    if let Ok(mut url) = Url::parse(&endpoint) {
        url.query_pairs_mut()
            .append_pair("MsgID", message_id)
            .append_pair("skey", skey)
            .append_pair("mmweb_appid", APP_ID)
            .append_pair("type", "big");
        urls.push(url.to_string());
    }

    // Some Web WeChat deployments return the full sticker from the same
    // endpoint without a `type` parameter, so keep that form as a fallback.
    if let Ok(mut url) = Url::parse(&endpoint) {
        url.query_pairs_mut()
            .append_pair("MsgID", message_id)
            .append_pair("skey", skey)
            .append_pair("mmweb_appid", APP_ID);
        urls.push(url.to_string());
    }

    // Older Web WeChat deployments accepted the lowercase parameter. Retain
    // it as the final compatibility fallback after the official current form.
    if let Ok(mut url) = Url::parse(&endpoint) {
        url.query_pairs_mut()
            .append_pair("msgid", message_id)
            .append_pair("skey", skey);
        urls.push(url.to_string());
    }

    urls
}

fn sanitize_sticker_source_url(raw: &str) -> String {
    if !raw.contains("/webwxgetmsgimg") {
        return raw.to_string();
    }
    let Ok(mut url) = Url::parse(raw) else {
        return raw.to_string();
    };
    url.set_query(None);
    url.to_string()
}

fn sticker_download_urls(content: &str, message_id: Option<&str>, skey: &str) -> Vec<String> {
    let mut urls = extract_emoji_urls(content);
    if let Some(message_id) = message_id {
        urls.extend(message_image_urls(message_id, skey));
    }
    rank_urls(urls)
}

fn rank_urls(mut urls: Vec<String>) -> Vec<String> {
    urls.sort_by_key(|url| {
        if url.contains("/webwxgetmsgimg") {
            0
        } else if url.contains("/20401/") {
            1
        } else if url.contains("cdnurl") {
            2
        } else if url.contains("/20402/") {
            3
        } else if url.contains("extern") {
            4
        } else if url.contains("encrypt") {
            5
        } else if url.contains("thumb") {
            6
        } else {
            3
        }
    });
    urls.dedup();
    urls
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn create_device_id() -> String {
    let value: u64 = rand::rng().random_range(0..1_000_000_000_000_000);
    format!("e{value:015}")
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use reqwest::{cookie::CookieStore as _, header::HeaderValue};
    use serde_json::json;
    use url::Url;

    use super::{
        extract_emoji_md5, extract_emoji_urls, is_product_sticker_message, now_millis,
        sanitize_sticker_source_url, sticker_download_urls, PersistentCookieStore, WechatCollector,
    };

    #[test]
    fn extracts_wechat_sticker_metadata() {
        let content = r#"&lt;emoji md5=\"0123456789abcdef0123456789abcdef\" cdnurl=\"https://emoji.qpic.cn/example.gif\" /&gt;"#;
        assert_eq!(
            extract_emoji_md5(content).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            extract_emoji_urls(content),
            vec!["https://emoji.qpic.cn/example.gif"]
        );
    }

    #[test]
    fn prefers_authenticated_message_image_for_store_stickers() {
        let content = r#"<msg><emoji md5="0123456789abcdef0123456789abcdef" productid="com.tencent.xin.emoticon.album" cdnurl="https://emoji.qpic.cn/wx_emoji/direct" encrypturl="https://emoji.qpic.cn/wx_emoji/encrypted" aeskey="a911cc2ec96ddb781b5ca85d24143642" /></msg>"#;
        let urls = sticker_download_urls(content, Some("123456789"), "@crypt_skey");

        let fallback = Url::parse(&urls[0]).expect("message image URL");
        assert_eq!(fallback.path(), "/cgi-bin/mmwebwx-bin/webwxgetmsgimg");
        assert!(fallback
            .query_pairs()
            .any(|(name, value)| name == "MsgID" && value == "123456789"));
        assert!(fallback
            .query_pairs()
            .any(|(name, value)| name == "skey" && value == "@crypt_skey"));
        assert!(fallback
            .query_pairs()
            .any(|(name, value)| name == "mmweb_appid" && value == "wx_webfilehelper"));
        assert!(fallback
            .query_pairs()
            .any(|(name, value)| name == "type" && value == "big"));
        let stored_source = sanitize_sticker_source_url(&urls[0]);
        assert!(!stored_source.contains("crypt_skey"));
        assert!(!stored_source.contains("123456789"));
        assert_eq!(
            Url::parse(&stored_source).expect("sanitized source").path(),
            "/cgi-bin/mmwebwx-bin/webwxgetmsgimg"
        );
        assert!(urls.iter().any(|url| url.ends_with("/direct")));
        assert!(urls.iter().any(|url| url.ends_with("/encrypted")));
    }

    #[test]
    fn recognizes_product_sticker_placeholders_as_web_unsupported() {
        let message = json!({
            "MsgType": 1,
            "ImgStatus": 2,
            "HasProductId": 1,
            "Content": "该类型暂不支持，请在手机上查看"
        });

        assert!(is_product_sticker_message(
            &message,
            message["Content"].as_str().expect("content")
        ));
    }

    #[test]
    fn restores_non_persistent_session_cookies() {
        let store = PersistentCookieStore::default();
        let url = Url::parse("https://szfilehelper.weixin.qq.com/").expect("url");
        let cookie = HeaderValue::from_static("wxuin=123456; Path=/; Secure; HttpOnly");
        store.set_cookies(&mut std::iter::once(&cookie), &url);

        let saved = store.to_json().expect("serialize cookies");
        let restored = PersistentCookieStore::from_json(&saved).expect("restore cookies");
        let header = restored
            .cookies(&url)
            .expect("restored cookie header")
            .to_str()
            .expect("valid cookie header")
            .to_string();

        assert!(header.contains("wxuin=123456"));
    }

    #[tokio::test]
    async fn prepare_exit_without_session_returns_immediately() {
        let path = std::env::temp_dir().join(format!(
            "sticker-relay-empty-session-{}-{}.bin",
            std::process::id(),
            now_millis()
        ));
        let collector = WechatCollector::new(path.clone()).expect("collector");
        tokio::time::timeout(Duration::from_secs(1), collector.prepare_exit())
            .await
            .expect("prepare exit timeout")
            .expect("prepare exit");
        assert!(!path.exists());
    }
}
