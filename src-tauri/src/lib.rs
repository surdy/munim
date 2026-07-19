//! munim — Tauri app entry point.
//!
//! Wires plugins, the invoke() command bridge, the system tray, and the auto-refresh
//! watcher. Pure domain logic lives in the `munim-core` crate; this shell stays thin.
//! Remaining stubs are marked `TODO(spec §…)` — see BUILD_SPEC.md.

mod commands;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt;

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
        .menu(|handle| {
            let refresh =
                MenuItem::with_id(handle, "refresh", "Refresh", true, Some("CmdOrCtrl+R"))?;

            let app_menu = Submenu::with_items(
                handle,
                "munim",
                true,
                &[
                    &PredefinedMenuItem::about(handle, None, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::quit(handle, None)?,
                ],
            )?;

            let edit_menu = Submenu::with_items(
                handle,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                ],
            )?;

            let view_menu = Submenu::with_items(handle, "View", true, &[&refresh])?;

            let window_menu = Submenu::with_items(
                handle,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(handle, None)?,
                    &PredefinedMenuItem::close_window(handle, None)?,
                ],
            )?;

            Menu::with_items(handle, &[&app_menu, &edit_menu, &view_menu, &window_menu])
        })
        .on_menu_event(|app, event| {
            if event.id() == "refresh" {
                let _ = app.emit("menu-refresh", ());
            }
        })
        .on_window_event(|window, event| {
            // Close-to-tray (BUILD_SPEC §0.5 #7): hide the window instead of quitting;
            // the app keeps running in the tray. The tray "Quit" item exits for real.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().ok();
                api.prevent_close();
            }
        })
        .setup(|app| {
            build_tray(app)?;
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

/// Build the system tray (BUILD_SPEC §6.1): status icon + menu-on-click with quick
/// stats, Open Dashboard / Refresh Now / Launch at login / Settings / Quit. The tray
/// model is icon + menu (no inline live-text) so it works on both macOS and Linux.
fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Quick stats: run one collect at startup and format Today / This month. "This week"
    // is left as an em-dash until the live watcher wires it up. If anything fails we fall
    // back to "—" placeholders so the tray still builds.
    let (today, week, month) = collect_stats(app);

    let stat_today = MenuItem::with_id(
        app,
        "stat_today",
        format!("Today  {today}"),
        false,
        None::<&str>,
    )?;
    let stat_week = MenuItem::with_id(
        app,
        "stat_week",
        format!("This week  {week}"),
        false,
        None::<&str>,
    )?;
    let stat_month = MenuItem::with_id(
        app,
        "stat_month",
        format!("This month  {month}"),
        false,
        None::<&str>,
    )?;

    let open = MenuItem::with_id(
        app,
        "tray_open_dashboard",
        "Open Dashboard",
        true,
        None::<&str>,
    )?;
    let refresh = MenuItem::with_id(app, "tray_refresh", "Refresh Now", true, None::<&str>)?;
    // Launch-at-login is OFF by default (BUILD_SPEC §0.5 #9); reflect the real plugin state.
    let launch_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let launch = CheckMenuItem::with_id(
        app,
        "tray_launch_login",
        "Launch at login",
        true,
        launch_enabled,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "tray_settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &stat_today,
            &stat_week,
            &stat_month,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &refresh,
            &launch,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    // Keep a handle to the checkable item so the toggle can reflect the new state.
    let launch_handle = launch.clone();

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "tray_open_dashboard" => show_main(app),
            "tray_refresh" => {
                // Reuse the existing refresh signal the webview already listens for.
                let _ = app.emit("menu-refresh", ());
            }
            "tray_launch_login" => {
                let manager = app.autolaunch();
                let now_enabled = manager.is_enabled().unwrap_or(false);
                let result = if now_enabled {
                    manager.disable()
                } else {
                    manager.enable()
                };
                // Reflect the effective state (revert the checkmark if the call failed).
                let checked = if result.is_ok() {
                    !now_enabled
                } else {
                    now_enabled
                };
                let _ = launch_handle.set_checked(checked);
            }
            "tray_settings" => {
                // The settings panel is a later ticket; emitting the intent is harmless.
                let _ = app.emit("open-settings", ());
            }
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click the icon → show + focus the main window. Right-click shows the menu.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    // Reuse the app's existing default window icon (a fresh brand icon lands in another
    // ticket). If it's somehow absent, the tray still builds without an icon.
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

/// Run one collect at startup and return formatted (today, week, month) stat strings.
/// "This week" is a placeholder until the live watcher wires it up (TODO #6).
fn collect_stats(app: &tauri::App) -> (String, String, String) {
    let dash = || "—".to_string();
    let home = match app.path().home_dir() {
        Ok(h) => h,
        Err(_) => return (dash(), dash(), dash()),
    };
    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(_) => return (dash(), dash(), dash()),
    };
    let _ = std::fs::create_dir_all(&data_dir);
    match munim_core::collect_and_persist(
        &home,
        &munim_core::Pricing::embedded_default(),
        &data_dir,
    ) {
        Ok(out) => (
            format!("${:.2}", out.summary.today_cost),
            // TODO(#6): live week cost — collect only exposes today/month today.
            dash(),
            format!("${:.2}", out.summary.month_cost),
        ),
        Err(_) => (dash(), dash(), dash()),
    }
}

/// Show and focus the main dashboard window.
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
