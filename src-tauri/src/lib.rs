//! munim — Tauri app entry point.
//!
//! Wires plugins, the invoke() command bridge, the system tray, and the auto-refresh
//! watcher. Pure domain logic lives in the `munim-core` crate; this shell stays thin.
//! Remaining stubs are marked `TODO(spec §…)` — see BUILD_SPEC.md.

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            // Off by default (BUILD_SPEC §0.5 #9) — we never enable it here; the user
            // opts in via tray/settings, which call the autostart plugin's enable().
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    // Updater is macOS-only (Linux updates via Flatpak). BUILD_SPEC §7.
    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .invoke_handler(tauri::generate_handler![
            commands::get_usage_data,
            commands::refresh,
            commands::export_data,
            commands::import_data,
            commands::save_imported_data,
            commands::load_session_detail,
            commands::get_settings,
            commands::save_settings,
        ])
        .setup(|_app| {
            // TODO(spec §6.1): build the system tray (icon + menu-on-click with quick
            //   stats, Open/Refresh/Launch-at-login/Settings/Quit); close-hides-to-tray.
            // TODO(spec §4.8): start the auto-refresh watcher (notify file-watch on the
            //   resolved source dirs + 60s interval fallback, debounced, non-overlapping);
            //   on each collect, emit an event to the webview and update tray labels.
            // TODO(spec §5.2b): after each collect, evaluate the monthly budget and fire
            //   the 80%/100% notification once per calendar month (dedupe in settings.json).
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running munim");
}
