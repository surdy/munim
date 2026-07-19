//! The invoke() bridge — the 5 operations the original exposed via WKScriptMessageHandler
//! (BUILD_SPEC §6) plus settings get/save for the new settings panel (§5.2b).
//!
//! All handlers are stubs. Return shapes match what the frontend expects (BUILD_SPEC §3).

use munim_core::{collect_and_persist, Pricing};
use serde_json::{json, Value};
use tauri::{path::BaseDirectory, AppHandle, Manager};

/// Load pricing from the bundled `pricing.toml` resource, falling back to the embedded
/// default if the resource is missing or unreadable (BUILD_SPEC §4.5).
fn load_pricing(app: &AppHandle) -> Pricing {
    if let Ok(path) = app.path().resolve("pricing.toml", BaseDirectory::Resource) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(p) = Pricing::load(&text) {
                return p;
            }
        }
    }
    Pricing::embedded_default()
}

/// Run the collector and return the dashboard payload.
/// Shape: { summary, claude: [...], codex: [...], openclaw: [...] } — see BUILD_SPEC §3/§4.
#[tauri::command]
pub async fn get_usage_data(app: AppHandle) -> Result<Value, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let pricing = load_pricing(&app);
    // Incremental collect + cache persistence (BUILD_SPEC §4.7). TODO(#6): spawn_blocking
    // so the file scan doesn't sit on an async worker.
    let out = collect_and_persist(&home, &pricing, &data_dir).map_err(|e| e.to_string())?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// Force a re-collect (manual refresh: FAB / menu ⌘R / tray "Refresh Now").
#[tauri::command]
pub async fn refresh(app: AppHandle) -> Result<Value, String> {
    // TODO(#6): route through the shared debounced, non-overlapping collect entrypoint.
    get_usage_data(app).await
}

/// Show a save dialog and write the exported JSON (already assembled by the frontend).
/// Returns `{ saved, count }`. BUILD_SPEC §4.7 / issue #9.
#[tauri::command]
pub async fn export_data(app: AppHandle, json: String) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let count = serde_json::from_str::<Value>(&json)
        .ok()
        .and_then(|v| {
            v.get("sessions")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0);
    let file = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name("munim-usage.json")
        .blocking_save_file();
    match file {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())?;
            Ok(json!({ "saved": true, "count": count }))
        }
        None => Ok(json!({ "saved": false })),
    }
}

/// Show an open dialog and return the chosen file's raw text (or null if cancelled). The
/// frontend validates the format and merges. BUILD_SPEC §4.7 / issue #9.
#[tauri::command]
pub async fn import_data(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .blocking_pick_file();
    match file {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| e.to_string())?;
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            Ok(Some(text))
        }
        None => Ok(None),
    }
}

/// Persist the frontend-merged records to the session cache (atomic). The frontend already
/// merged + deduped; we validate + write. BUILD_SPEC §4.7 / issue #9.
#[tauri::command]
pub async fn save_imported_data(app: AppHandle, records: Value) -> Result<Value, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let recs: Vec<munim_core::SessionRecord> =
        serde_json::from_value(records).map_err(|e| e.to_string())?;
    munim_core::cache::save_cache(&data_dir, &recs).map_err(|e| e.to_string())?;
    Ok(json!({ "saved": recs.len() }))
}

/// Read a single session file for the detail modal's conversation history.
/// SECURITY (BUILD_SPEC §6): the allowlist + 8 MB cap + non-dir + symlink resolution live in
/// `munim_core::detail`. Returns the raw file text; the frontend parses it. Issue #10.
#[tauri::command]
pub async fn load_session_detail(app: AppHandle, file_path: String) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    munim_core::load_session_file(&home, &file_path)
}

/// Return persisted settings (budget, autostart pref, alert-fired flags). BUILD_SPEC §5.2b.
/// The `Settings` struct serializes camelCase (monthlyBudget, launchAtLogin, ...).
#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Result<Value, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let settings = munim_core::settings::load(&dir);
    serde_json::to_value(settings).map_err(|e| e.to_string())
}

/// Persist settings from the settings panel. Only `monthlyBudget` and `launchAtLogin` come
/// from the panel; the alert-fired flags are preserved from disk. If launch-at-login changed,
/// the autostart plugin is toggled to match. BUILD_SPEC §5.2b.
#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Value) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // Load existing settings so we preserve the alert-fired flags / alert_month.
    let mut current = munim_core::settings::load(&dir);
    let was_launch_at_login = current.launch_at_login;

    // Overwrite only the panel-owned fields. `monthlyBudget` may be a number or null.
    current.monthly_budget =
        settings
            .get("monthlyBudget")
            .and_then(|v| if v.is_null() { None } else { v.as_f64() });
    let launch_at_login = settings
        .get("launchAtLogin")
        .and_then(Value::as_bool)
        .unwrap_or(current.launch_at_login);
    current.launch_at_login = launch_at_login;

    munim_core::settings::save(&dir, &current).map_err(|e| e.to_string())?;

    // Sync the OS autostart state only when the preference actually changed. Errors here
    // are non-fatal (the pref is still persisted), so we log and continue.
    if launch_at_login != was_launch_at_login {
        let autolaunch = app.autolaunch();
        let result = if launch_at_login {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(e) = result {
            eprintln!("munim: failed to update autostart to {launch_at_login}: {e}");
        }
    }

    Ok(())
}
