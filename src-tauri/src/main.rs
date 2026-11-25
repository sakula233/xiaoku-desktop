#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod brain;
mod db;
mod model_engine;
mod weather;

use brain::ChatOutcome;
use chrono::Utc;
use db::{AppSettings, ChatMessage, CompanionDb, DrawingItem, EventItem, ProjectItem};
use model_engine::{LocalModelManager, LocalModelStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    db: Arc<CompanionDb>,
    engine: Arc<LocalModelManager>,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BootstrapPayload {
    settings: AppSettings,
    local_status: LocalModelStatus,
    messages: Vec<ChatMessage>,
    events: Vec<EventItem>,
    projects: Vec<ProjectItem>,
    drawings: Vec<DrawingItem>,
    models_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatch {
    partner_name: Option<String>,
    brain_mode: Option<String>,
    custom_model_path: Option<String>,
    api_base_url: Option<String>,
    api_key: Option<String>,
    cloud_model: Option<String>,
    image_model: Option<String>,
    theme: Option<String>,
}

const DAILY_POOL: &[&str] = &[
    "刚整理完昨晚的雨声歌单，顺手泡了杯热茶。",
    "翻到一本老科幻短篇集，正看到最离谱的反转。",
    "去楼下便利店晃了一圈，纠结了三分钟还是拿了热汤面。",
    "把虚拟电台今晚的备选配乐挑好了两首偏安静的。",
    "窗外风有点凉，刚把桌面乱七八糟的文件归了下类。",
    "听完一张冷门原声带，感觉下午脑子清醒了不少。",
];

async fn build_bootstrap(state: &AppState, ensure_engine: bool) -> BootstrapPayload {
    let settings = state.db.load_settings();
    let local_status = if ensure_engine {
        state
            .engine
            .ensure_started(&state.http, &settings.custom_model_path, false)
            .await
    } else {
        state
            .engine
            .status(&state.http, &settings.custom_model_path)
            .await
    };
    BootstrapPayload {
        settings,
        local_status,
        messages: state.db.list_messages(120),
        events: state.db.search_events("", 30),
        projects: state.db.list_projects(),
        drawings: state.db.list_drawings(20),
        models_dir: state.engine.models_dir().to_string_lossy().to_string(),
    }
}

#[tauri::command]
async fn get_bootstrap(state: State<'_, AppState>, wake_model: Option<bool>) -> Result<BootstrapPayload, String> {
    Ok(build_bootstrap(&state, wake_model.unwrap_or(false)).await)
}

#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    text: String,
    image_data: Option<String>,
) -> Result<(ChatOutcome, BootstrapPayload), String> {
    let settings = state.db.load_settings();
    let local_status = if settings.brain_mode != "cloud-only" {
        state
            .engine
            .ensure_started(&state.http, &settings.custom_model_path, false)
            .await
    } else {
        state
            .engine
            .status(&state.http, &settings.custom_model_path)
            .await
    };
    let outcome = brain::run_chat_turn(&state.http, &state.db, &local_status, &text, image_data).await?;
    let boot = build_bootstrap(&state, false).await;
    Ok((outcome, boot))
}

#[tauri::command]
async fn update_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<BootstrapPayload, String> {
    let mut s = state.db.load_settings();
    let mut model_changed = false;
    if let Some(v) = patch.partner_name {
        let clean = v.trim();
        if !clean.is_empty() && clean.chars().count() <= 16 {
            s.partner_name = clean.to_string();
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(clean);
            }
        }
    }
    if let Some(v) = patch.brain_mode {
        if matches!(v.as_str(), "local-only" | "hybrid" | "cloud-only") {
            s.brain_mode = v;
        }
    }
    if let Some(v) = patch.custom_model_path {
        if v.trim() != s.custom_model_path {
            s.custom_model_path = v.trim().to_string();
            model_changed = true;
        }
    }
    if let Some(v) = patch.api_base_url {
        let clean = v.trim();
        if !clean.is_empty() {
            s.api_base_url = clean.to_string();
        }
    }
    if let Some(v) = patch.api_key {
        s.api_key = v.trim().to_string();
    }
    if let Some(v) = patch.cloud_model {
        let clean = v.trim();
        if !clean.is_empty() {
            s.cloud_model = clean.to_string();
        }
    }
    if let Some(v) = patch.image_model {
        let clean = v.trim();
        if !clean.is_empty() {
            s.image_model = clean.to_string();
        }
    }
    if let Some(v) = patch.theme {
        if matches!(v.as_str(), "system" | "light" | "dark") {
            s.theme = v;
        }
    }
    state.db.save_settings(&s)?;
    if model_changed {
        let _ = state
            .engine
            .ensure_started(&state.http, &s.custom_model_path, true)
            .await;
    }
    Ok(build_bootstrap(&state, false).await)
}

#[tauri::command]
async fn pick_local_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<BootstrapPayload>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("GGUF 本地模型", &["gguf"])
        .blocking_pick_file();

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path_str = file_path.to_string();
    let mut s = state.db.load_settings();
    s.custom_model_path = path_str.clone();
    state.db.save_settings(&s)?;
    let _ = state.engine.ensure_started(&state.http, &path_str, true).await;
    Ok(Some(build_bootstrap(&state, false).await))
}

#[tauri::command]
async fn advance_daily(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    let mut s = state.db.load_settings();
    let idx = DAILY_POOL
        .iter()
        .position(|&x| x == s.daily_summary)
        .map(|i| (i + 1) % DAILY_POOL.len())
        .unwrap_or(0);
    s.daily_summary = DAILY_POOL[idx].to_string();
    s.daily_date = Utc::now().format("%Y-%m-%d").to_string();
    state.db.save_settings(&s)?;
    Ok(build_bootstrap(&state, false).await)
}

#[tauri::command]
async fn forget_event(state: State<'_, AppState>, event_id: String) -> Result<BootstrapPayload, String> {
    state.db.delete_event(&event_id)?;
    Ok(build_bootstrap(&state, false).await)
}

#[tauri::command]
async fn clear_chat(state: State<'_, AppState>) -> Result<BootstrapPayload, String> {
    state.db.clear_messages()?;
    Ok(build_bootstrap(&state, false).await)
}

#[tauri::command]
async fn export_capsule_file(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    let default_name = format!("xiaoku-memory-{}.json", Utc::now().format("%Y-%m-%d"));
    let target = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("记忆胶囊 JSON", &["json"])
        .blocking_save_file();
    let Some(dest) = target else {
        return Ok(false);
    };
    let payload = state.db.export_capsule();
    let pretty = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(dest.to_string(), pretty).map_err(|e| format!("写出失败: {e}"))?;
    Ok(true)
}

#[tauri::command]
async fn import_capsule_file(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Option<BootstrapPayload>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("记忆胶囊 JSON", &["json"])
        .blocking_pick_file();
    let Some(src) = picked else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(src.to_string()).map_err(|e| format!("读取失败: {e}"))?;
    let val: Value = serde_json::from_str(&raw).map_err(|e| format!("JSON 格式无效: {e}"))?;
    state.db.import_capsule(&val)?;
    Ok(Some(build_bootstrap(&state, false).await))
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let log_dir = std::path::PathBuf::from(appdata).join("com.xiaoku.desktop");
            let _ = std::fs::create_dir_all(&log_dir);
            let _ = std::fs::write(log_dir.join("crash.log"), format!("{info}\n"));
        }
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            let db_path = app_data_dir.join("xiaoku-native.sqlite");
            let db = Arc::new(CompanionDb::open(&db_path).expect("failed to open sqlite db"));
            let resource_dir = app.path().resource_dir().ok();
            let engine = Arc::new(LocalModelManager::new(app_data_dir, resource_dir));
            let http = reqwest::Client::builder()
                .user_agent("XiaokuNative/1.0")
                .build()
                .expect("failed to build http client");

            let saved_name = db.load_settings().partner_name;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_title(&saved_name);
                let _ = win.show();
                let _ = win.set_focus();
            }

            // Native System Tray (non-fatal if OS tray unavailable)
            if let (Ok(show_item), Ok(quit_item)) = (
                MenuItem::with_id(app, "show", "打开聊天窗口", true, None::<&str>),
                MenuItem::with_id(app, "quit", "彻底退出", true, None::<&str>),
            ) {
                if let Ok(tray_menu) = Menu::with_items(app, &[&show_item, &quit_item]) {
                    let mut tray_builder = TrayIconBuilder::new()
                        .menu(&tray_menu)
                        .tooltip(&saved_name)
                        .on_menu_event(|app_handle, event| match event.id.as_ref() {
                            "show" => {
                                if let Some(w) = app_handle.get_webview_window("main") {
                                    let _ = w.show();
                                    let _ = w.unminimize();
                                    let _ = w.set_focus();
                                }
                            }
                            "quit" => {
                                if let Some(st) = app_handle.try_state::<AppState>() {
                                    st.engine.stop();
                                }
                                app_handle.exit(0);
                            }
                            _ => {}
                        })
                        .on_tray_icon_event(|tray, event| {
                            if let TrayIconEvent::Click {
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                ..
                            } = event
                            {
                                let app_handle = tray.app_handle();
                                if let Some(w) = app_handle.get_webview_window("main") {
                                    if w.is_visible().unwrap_or(false) {
                                        let _ = w.hide();
                                    } else {
                                        let _ = w.show();
                                        let _ = w.unminimize();
                                        let _ = w.set_focus();
                                    }
                                }
                            }
                        });

                    if let Some(icon) = app.default_window_icon() {
                        tray_builder = tray_builder.icon(icon.clone());
                    }
                    let _ = tray_builder.build(app);
                }
            }

            // Warm up local model silently in background if present
            let bg_engine = engine.clone();
            let bg_db = db.clone();
            let bg_http = http.clone();
            tauri::async_runtime::spawn(async move {
                let s = bg_db.load_settings();
                if s.brain_mode != "cloud-only" {
                    let _ = bg_engine
                        .ensure_started(&bg_http, &s.custom_model_path, false)
                        .await;
                }
            });

            app.manage(AppState { db, engine, http });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Clicking close hides to native system tray so local GPU model stays warm!
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            send_message,
            update_settings,
            pick_local_model,
            advance_daily,
            forget_event,
            clear_chat,
            export_capsule_file,
            import_capsule_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Xiaoku native app");
}
