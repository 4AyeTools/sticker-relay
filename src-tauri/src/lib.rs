mod feishu;
mod models;
mod secure_storage;
mod settings;
mod store;
mod wechat;

use std::{path::PathBuf, sync::Arc};

use models::{
    ExportResult, FeishuCliStatus, FeishuLoginAdvance, FeishuSelf, FeishuSendProgress,
    FeishuSendRequest, FeishuSendState, StickerLibraryChangeResult, StickerLibraryLocation,
    StickerRecord, WechatLoginState, WechatQrResult,
};
use settings::AppSettings;
use store::StickerStore;
use tauri::{path::BaseDirectory, AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

use crate::{
    feishu::{cli_binary_name, migrate_managed_component, validate_feishu_auth_url, FeishuCli},
    wechat::WechatCollector,
};

const PERSISTENT_COMPONENTS_DIRECTORY: &str = "com.ayecode.wechatfeishustickers-components";

struct AppState {
    store: Arc<Mutex<StickerStore>>,
    settings: Arc<Mutex<AppSettings>>,
    collector: Arc<WechatCollector>,
    feishu: Arc<Mutex<FeishuCli>>,
}

#[tauri::command]
async fn wechat_request_qr(state: State<'_, AppState>) -> Result<WechatQrResult, String> {
    state.collector.request_qr().await.map_err(error_text)
}

#[tauri::command]
async fn wechat_poll(
    uuid: String,
    tip: u8,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WechatLoginState, String> {
    state
        .collector
        .poll(&uuid, tip, app, state.store.clone())
        .await
        .map_err(error_text)
}

#[tauri::command]
async fn wechat_logout(state: State<'_, AppState>) -> Result<(), String> {
    state.collector.logout().await.map_err(error_text)
}

#[tauri::command]
async fn wechat_status(state: State<'_, AppState>) -> Result<WechatLoginState, String> {
    Ok(state.collector.status().await)
}

#[tauri::command]
async fn wechat_prepare_exit(state: State<'_, AppState>) -> Result<(), String> {
    state.collector.prepare_exit().await.map_err(error_text)
}

#[tauri::command]
async fn stickers_list(state: State<'_, AppState>) -> Result<Vec<StickerRecord>, String> {
    Ok(state.store.lock().await.list())
}

#[tauri::command]
async fn stickers_data_url(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    Ok(state.store.lock().await.get_data_url(&id))
}

#[tauri::command]
async fn stickers_location(state: State<'_, AppState>) -> Result<StickerLibraryLocation, String> {
    let store = state.store.lock().await;
    let settings = state.settings.lock().await;
    Ok(StickerLibraryLocation {
        path: store.root_directory().to_string_lossy().to_string(),
        is_default: same_path(
            store.root_directory(),
            settings.default_sticker_library_root(),
        ),
    })
}

#[tauri::command]
async fn stickers_open_location(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let path = state
        .store
        .lock()
        .await
        .root_directory()
        .to_string_lossy()
        .to_string();
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(error_text)
}

#[tauri::command]
async fn stickers_delete(ids: Vec<String>, state: State<'_, AppState>) -> Result<usize, String> {
    let removed = state
        .store
        .lock()
        .await
        .delete_ids(&ids)
        .map_err(error_text)?;
    let md5s: Vec<String> = removed
        .iter()
        .filter_map(|record| record.wechat_md5.clone())
        .collect();
    state.collector.forget_sticker_md5s(&md5s).await;
    Ok(removed.len())
}

#[tauri::command]
async fn stickers_choose_location(
    destination: String,
    state: State<'_, AppState>,
) -> Result<StickerLibraryChangeResult, String> {
    let destination = PathBuf::from(destination);
    let mut store = state.store.lock().await;
    let current = store.root_directory().to_path_buf();
    if same_path(&current, &destination) {
        let settings = state.settings.lock().await;
        return Ok(StickerLibraryChangeResult {
            canceled: false,
            path: current.to_string_lossy().to_string(),
            is_default: same_path(&current, settings.default_sticker_library_root()),
            migrated_count: 0,
        });
    }
    let migration = store.migrate_to(destination).map_err(error_text)?;
    {
        let mut settings = state.settings.lock().await;
        if let Err(error) = settings.set_sticker_library_root(store.root_directory().to_path_buf())
        {
            let _ = store.restore_root(migration.previous_root.clone());
            return Err(error.to_string());
        }
    }
    store.cleanup_previous_library(&migration);
    let settings = state.settings.lock().await;
    Ok(StickerLibraryChangeResult {
        canceled: false,
        path: store.root_directory().to_string_lossy().to_string(),
        is_default: same_path(
            store.root_directory(),
            settings.default_sticker_library_root(),
        ),
        migrated_count: migration.migrated_count,
    })
}

#[tauri::command]
async fn stickers_export_zip(
    destination: String,
    state: State<'_, AppState>,
) -> Result<ExportResult, String> {
    let destination_path = PathBuf::from(&destination);
    let count = state
        .store
        .lock()
        .await
        .export_zip(&destination_path)
        .map_err(error_text)?;
    Ok(ExportResult {
        canceled: false,
        path: Some(destination),
        count: Some(count),
    })
}

#[tauri::command]
async fn feishu_status(state: State<'_, AppState>) -> Result<FeishuCliStatus, String> {
    Ok(state.feishu.lock().await.status().await)
}

#[tauri::command]
async fn feishu_check_update(state: State<'_, AppState>) -> Result<FeishuCliStatus, String> {
    Ok(state.feishu.lock().await.check_update().await)
}

#[tauri::command]
async fn feishu_cli_install(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FeishuCliStatus, String> {
    state
        .feishu
        .lock()
        .await
        .install_latest(&app)
        .await
        .map_err(error_text)
}

#[tauri::command]
async fn feishu_self(state: State<'_, AppState>) -> Result<FeishuSelf, String> {
    state
        .feishu
        .lock()
        .await
        .get_self()
        .await
        .map_err(error_text)
}

#[tauri::command]
async fn feishu_login_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<models::FeishuLoginSession, String> {
    let session = state
        .feishu
        .lock()
        .await
        .start_login()
        .await
        .map_err(error_text)?;
    open_feishu_authorization(&app, &session.verification_url)?;
    Ok(session)
}

#[tauri::command]
async fn feishu_login_open(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let url = state
        .feishu
        .lock()
        .await
        .pending_login_url()
        .ok_or_else(|| "当前没有待完成的飞书授权".to_string())?;
    open_feishu_authorization(&app, &url)
}

#[tauri::command]
async fn feishu_login_finish(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FeishuLoginAdvance, String> {
    let advance = state
        .feishu
        .lock()
        .await
        .finish_login()
        .await
        .map_err(error_text)?;
    if let Some(session) = advance.session.as_ref() {
        open_feishu_authorization(&app, &session.verification_url)?;
    }
    Ok(advance)
}

#[tauri::command]
async fn feishu_login_cancel(state: State<'_, AppState>) -> Result<(), String> {
    state.feishu.lock().await.cancel_login();
    Ok(())
}

#[tauri::command]
async fn feishu_send(
    request: FeishuSendRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let records: Vec<StickerRecord> = state
        .store
        .lock()
        .await
        .list()
        .into_iter()
        .filter(|record| !request.only_pending || record.feishu_state != FeishuSendState::Sent)
        .collect();
    let total = records.len();
    let feishu = state.feishu.clone();
    let store = state.store.clone();
    let destination = request.destination;
    let background_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = feishu
            .lock()
            .await
            .send_batch(records, destination, store, background_app.clone())
            .await;
        if let Err(error) = result {
            let _ = background_app.emit(
                "feishu-progress",
                FeishuSendProgress {
                    current: 0,
                    total,
                    sticker_id: None,
                    sent: 0,
                    failed: total,
                    message: Some(error.to_string()),
                    done: true,
                },
            );
        }
    });
    Ok(serde_json::json!({ "started": true, "total": total }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let default_library = app_data.join("sticker-library");
            let legacy_settings = std::env::var_os("APPDATA").map(PathBuf::from).map(|path| {
                path.join("wechat-feishu-sticker-migrator")
                    .join("settings.json")
            });
            let settings = AppSettings::initialize(
                app_data.join("settings.json"),
                default_library,
                legacy_settings,
            )?;
            let mut store = StickerStore::new(settings.sticker_library_root().to_path_buf());
            store.initialize()?;

            let cli_filename = if cfg!(windows) {
                "lark-cli.exe"
            } else {
                "lark-cli"
            };
            let legacy_cli_directory = app_data.join("components").join("lark-cli");
            let persistent_cli_directory = app
                .path()
                .local_data_dir()?
                .join(PERSISTENT_COMPONENTS_DIRECTORY)
                .join("lark-cli");
            if let Err(error) =
                migrate_managed_component(&legacy_cli_directory, &persistent_cli_directory)
            {
                eprintln!("迁移飞书 CLI 到持久组件目录失败：{error}");
            }
            let packaged_cli = app
                .path()
                .resolve(format!("bin/{cli_filename}"), BaseDirectory::Resource)?;
            let development_cli = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join(cli_filename);
            let legacy_managed_cli = legacy_cli_directory.join(cli_binary_name());

            let store = Arc::new(Mutex::new(store));
            let collector = Arc::new(WechatCollector::new(app_data.join("wechat-session.bin"))?);
            app.manage(AppState {
                store: store.clone(),
                settings: Arc::new(Mutex::new(settings)),
                collector: collector.clone(),
                feishu: Arc::new(Mutex::new(FeishuCli::new(
                    persistent_cli_directory,
                    vec![legacy_managed_cli, packaged_cli, development_cli],
                ))),
            });
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = collector.restore(app_handle, store).await {
                    eprintln!("恢复微信登录状态失败：{error}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            wechat_request_qr,
            wechat_poll,
            wechat_logout,
            wechat_status,
            wechat_prepare_exit,
            stickers_list,
            stickers_data_url,
            stickers_location,
            stickers_open_location,
            stickers_delete,
            stickers_choose_location,
            stickers_export_zip,
            feishu_status,
            feishu_check_update,
            feishu_cli_install,
            feishu_self,
            feishu_login_start,
            feishu_login_open,
            feishu_login_finish,
            feishu_login_cancel,
            feishu_send,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}

fn error_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn open_feishu_authorization(app: &AppHandle, url: &str) -> Result<(), String> {
    validate_feishu_auth_url(url).map_err(error_text)?;
    app.opener().open_url(url, None::<&str>).map_err(error_text)
}

fn same_path(first: &std::path::Path, second: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        first
            .to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
            == second
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        first.to_string_lossy().trim_end_matches('/')
            == second.to_string_lossy().trim_end_matches('/')
    }
}
