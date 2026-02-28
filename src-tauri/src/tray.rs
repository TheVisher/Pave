use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewWindowBuilder,
};
use tauri::WebviewUrl;

fn show_settings(app: &AppHandle) {
    // If window already exists, just focus it
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_focus();
        return;
    }

    // Create a new settings window on demand (avoids Wayland hide/show crash)
    match WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("Pave Settings")
        .inner_size(480.0, 400.0)
        .resizable(false)
        .center()
        .build()
    {
        Ok(_) => {}
        Err(e) => log::error!("Failed to create settings window: {e}"),
    }
}

pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
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
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&settings_item)
        .item(&separator1)
        .item(&maximize_label)
        .item(&left_label)
        .item(&right_label)
        .item(&separator2)
        .item(&quit_item)
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("No default icon found")?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Pave - Window Tiling")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "settings" => {
                show_settings(app);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
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
