use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewWindowBuilder,
};
use tauri::WebviewUrl;
use tokio::sync::broadcast;

use crate::config::PaveConfig;

fn show_settings(app: &AppHandle) {
    // If window already exists, just focus it
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_focus();
        return;
    }

    // Create a new settings window on demand (avoids Wayland hide/show crash)
    match WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("Pave Settings")
        .inner_size(480.0, 560.0)
        .resizable(false)
        .center()
        .build()
    {
        Ok(_) => {}
        Err(e) => log::error!("Failed to create settings window: {e}"),
    }
}

pub fn setup_tray(
    app: &AppHandle,
    config: &PaveConfig,
    preset_tx: broadcast::Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings_item = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
    let separator1 = PredefinedMenuItem::separator(app)?;

    let maximize_label =
        MenuItemBuilder::with_id("label_maximize", "Almost Maximize: Ctrl+Alt+Enter")
            .enabled(false)
            .build(app)?;
    let left_label = MenuItemBuilder::with_id("label_left", "Snap Left: Ctrl+Alt+Left")
        .enabled(false)
        .build(app)?;
    let right_label = MenuItemBuilder::with_id("label_right", "Snap Right: Ctrl+Alt+Right")
        .enabled(false)
        .build(app)?;

    let separator2 = PredefinedMenuItem::separator(app)?;

    let mut menu_builder = MenuBuilder::new(app)
        .item(&settings_item)
        .item(&separator1)
        .item(&maximize_label)
        .item(&left_label)
        .item(&right_label)
        .item(&separator2);

    // Add preset items
    let preset_names: Vec<String> = config.presets.iter().map(|p| p.name.clone()).collect();
    for name in &preset_names {
        let item_id = format!("preset_{}", name);
        let preset_item = MenuItemBuilder::with_id(&item_id, name).build(app)?;
        menu_builder = menu_builder.item(&preset_item);
    }

    if !preset_names.is_empty() {
        let separator3 = PredefinedMenuItem::separator(app)?;
        menu_builder = menu_builder.item(&separator3);
    }

    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = menu_builder.item(&quit_item).build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("No default icon found")?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Pave - Window Tiling")
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            match id {
                "settings" => {
                    show_settings(app);
                }
                "quit" => {
                    app.exit(0);
                }
                _ if id.starts_with("preset_") => {
                    let name = id.strip_prefix("preset_").unwrap_or("");
                    log::info!("Tray: activating preset '{name}'");
                    let _ = preset_tx.send(name.to_string());
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_settings(app);
            }
        })
        .build(app)?;

    Ok(())
}
