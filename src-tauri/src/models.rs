use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FeishuSendState {
    Pending,
    Sending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerRecord {
    pub id: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wechat_md5: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub imported_at: u64,
    pub feishu_state: FeishuSendState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feishu_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerImportEvent {
    pub imported: usize,
    pub skipped: usize,
    pub unsupported: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<StickerRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WechatLoginState {
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatQrResult {
    pub uuid: String,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerLibraryLocation {
    pub path: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerLibraryChangeResult {
    pub canceled: bool,
    pub path: String,
    pub is_default: bool,
    pub migrated_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub canceled: bool,
    pub path: Option<String>,
    pub count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuCliStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub authenticated: bool,
    pub detail: Option<String>,
    pub source: Option<String>,
    pub executable_path: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuCliProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub message: String,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuSelf {
    pub open_id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuLoginSession {
    pub stage: String,
    pub verification_url: String,
    pub user_code: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuLoginAdvance {
    pub status: Option<FeishuCliStatus>,
    pub session: Option<FeishuLoginSession>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub enum FeishuDestination {
    #[serde(rename = "self")]
    SelfTarget,
    #[serde(rename = "user")]
    User { id: String },
    #[serde(rename = "chat")]
    Chat { id: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuSendRequest {
    pub destination: FeishuDestination,
    pub only_pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuSendProgress {
    pub current: usize,
    pub total: usize,
    pub sticker_id: Option<String>,
    pub sent: usize,
    pub failed: usize,
    pub message: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncKeyItem {
    #[serde(rename = "Key")]
    pub key: i64,
    #[serde(rename = "Val")]
    pub val: i64,
}
