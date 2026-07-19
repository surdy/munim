//! The invoke() bridge — the 5 operations the original exposed via WKScriptMessageHandler
//! (BUILD_SPEC §6) plus settings get/save for the new settings panel (§5.2b).
//!
//! All handlers are stubs. Return shapes match what the frontend expects (BUILD_SPEC §3).

use munim_core::{collect, Pricing};
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
    let pricing = load_pricing(&app);
    // TODO(#3): move to incremental collect with the scan-index cache; TODO: spawn_blocking
    // so the file scan doesn't sit on an async worker.
    let out = collect(&home, &pricing);
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// Force a re-collect (manual refresh: FAB / menu ⌘R / tray "Refresh Now").
#[tauri::command]
pub async fn refresh(app: AppHandle) -> Result<Value, String> {
    // TODO(#6): route through the shared debounced, non-overlapping collect entrypoint.
    get_usage_data(app).await
}

/// Export the merged session cache as JSON (native save dialog handled on the JS side
/// via tauri-plugin-dialog, or return the bytes here).
#[tauri::command]
pub async fn export_data() -> Result<Value, String> {
    // TODO(spec §4.7): read sessions-cache.json and return it for saving.
    Ok(json!({ "sessions": [], "format": "munim", "version": 1 }))
}

/// Parse an imported JSON file (accept both munim and legacy claude-usage-tracker shapes).
#[tauri::command]
pub async fn import_data(_json_text: String) -> Result<Value, String> {
    // TODO(spec §4.7): validate + normalize (back-fill `provider`), return records to merge.
    Ok(json!({ "imported": 0, "records": [] }))
}

/// Merge imported records into the cache (dedupe key: provider|source|file|date) and persist.
#[tauri::command]
pub async fn save_imported_data(_records: Value) -> Result<Value, String> {
    // TODO(spec §4.2/§4.7): merge + atomic-write sessions-cache.json, return merged count.
    Ok(json!({ "merged": 0 }))
}

/// Read a single session file for the detail modal's conversation history.
/// SECURITY (BUILD_SPEC §6): enforce the path allowlist (known tool roots), an 8 MB cap,
/// a non-directory check, and symlink resolution — reject anything outside the allowlist.
#[tauri::command]
pub async fn load_session_detail(_file_path: String) -> Result<Value, String> {
    // TODO(spec §6): validate path against allowlist, size-cap, then parse JSONL → messages.
    Err("load_session_detail: not implemented".into())
}

/// Return persisted settings (budget, autostart pref, alert-fired flags). BUILD_SPEC §5.2b.
#[tauri::command]
pub async fn get_settings() -> Result<Value, String> {
    // TODO(spec §5.2b): crate::settings::load().
    Ok(json!({ "monthlyBudget": null, "launchAtLogin": false }))
}

/// Persist settings from the settings panel.
#[tauri::command]
pub async fn save_settings(_settings: Value) -> Result<(), String> {
    // TODO(spec §5.2b): crate::settings::save(); if launchAtLogin changed, call the
    //   autostart plugin enable()/disable() and keep the tray checkbox in sync.
    Ok(())
}
