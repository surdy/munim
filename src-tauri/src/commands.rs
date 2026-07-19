//! The invoke() bridge — the 5 operations the original exposed via WKScriptMessageHandler
//! (BUILD_SPEC §6) plus settings get/save for the new settings panel (§5.2b).
//!
//! All handlers are stubs. Return shapes match what the frontend expects (BUILD_SPEC §3).

use serde_json::{json, Value};

/// Run the collector (incremental) and return the dashboard payload.
/// Shape: { summary, claude: [...], codex: [...], openclaw: [...] } — see BUILD_SPEC §3/§4.
#[tauri::command]
pub async fn get_usage_data() -> Result<Value, String> {
    // TODO(spec §4): crate::collector::collect() → serialize summary + session arrays.
    Ok(json!({ "summary": {}, "claude": [], "codex": [], "openclaw": [] }))
}

/// Force a re-collect (manual refresh: FAB / menu ⌘R / tray "Refresh Now").
#[tauri::command]
pub async fn refresh() -> Result<Value, String> {
    // TODO(spec §4.8): run the same debounced, non-overlapping collect entrypoint,
    //   then let the caller re-fetch via get_usage_data (or return the payload here).
    get_usage_data().await
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
