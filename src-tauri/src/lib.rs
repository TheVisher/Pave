mod config;
mod platform;
mod presets;
mod tiling;
mod tray;

use config::{PaveConfig, Preset};
use platform::kwin::KWinBackend;
use platform::MonitorInfo;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::{broadcast, RwLock};

struct AppState {
    config: Arc<RwLock<PaveConfig>>,
    tiling_state: Arc<tiling::TilingState>,
    backend: Arc<KWinBackend>,
    preset_tx: broadcast::Sender<String>,
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<PaveConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn update_config(
    state: tauri::State<'_, AppState>,
    config: PaveConfig,
) -> Result<(), String> {
    config.save()?;
    if let Some(r) = config.corner_radius {
        KWinBackend::ensure_shapecorners_defaults().map_err(|e| {
            log::error!("Failed to set ShapeCorners defaults: {e}");
            e
        })?;
        if let Err(e) = state.backend.apply_corner_radius(r).await {
            log::error!("Failed to apply corner radius: {e}");
        }
    }
    *state.config.write().await = config;
    Ok(())
}

#[tauri::command]
async fn get_monitors(state: tauri::State<'_, AppState>) -> Result<Vec<MonitorInfo>, String> {
    state.backend.get_monitors().await
}

#[tauri::command]
async fn is_first_run() -> Result<bool, String> {
    Ok(PaveConfig::is_first_run())
}

#[tauri::command]
async fn capture_preset(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Preset, String> {
    let preset = presets::capture_preset(state.backend.as_ref(), name).await?;
    let mut cfg = state.config.write().await;
    // Replace existing preset with same name, or push new
    if let Some(pos) = cfg.presets.iter().position(|p| p.name == preset.name) {
        cfg.presets[pos] = preset.clone();
    } else {
        cfg.presets.push(preset.clone());
    }
    cfg.save()?;
    Ok(preset)
}

#[tauri::command]
async fn activate_preset(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let cfg = state.config.read().await;
    let preset = cfg
        .presets
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("Preset '{}' not found", name))?
        .clone();
    drop(cfg);
    presets::activate_preset(state.backend.as_ref(), &preset).await
}

#[tauri::command]
async fn delete_preset(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let mut cfg = state.config.write().await;
    cfg.presets.retain(|p| p.name != name);
    cfg.save()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Work around WebKit Wayland protocol error when creating windows
    // by disabling the DMABuf renderer that triggers the crash
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_config,
            update_config,
            get_monitors,
            is_first_run,
            capture_preset,
            activate_preset,
            delete_preset
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Initialize backend and state in async context
            let handle2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                let (backend, mut shortcut_rx, mut resize_rx, mut preset_rx) =
                    match KWinBackend::new().await {
                    Ok(b) => b,
                    Err(e) => {
                        log::error!("Failed to initialize KWin backend: {e}");
                        return;
                    }
                };

                let config = PaveConfig::load();
                let first_run = PaveConfig::is_first_run();

                // Save default config on first run
                if first_run {
                    if let Err(e) = config.save() {
                        log::warn!("Failed to save default config: {e}");
                    }
                }

                // Register global shortcuts via KWin script
                if let Err(e) = backend.register_all_shortcuts().await {
                    log::error!("Failed to register shortcuts: {e}");
                }

                // Apply corner radius on startup if configured
                if let Some(r) = config.corner_radius {
                    if let Err(e) = KWinBackend::ensure_shapecorners_defaults() {
                        log::error!("Failed to set ShapeCorners defaults: {e}");
                    }
                    if let Err(e) = backend.apply_corner_radius(r).await {
                        log::error!("Failed to apply corner radius on startup: {e}");
                    }
                }

                let backend_arc = Arc::new(backend);
                let config_arc = Arc::new(RwLock::new(config.clone()));
                let tiling_state_arc = Arc::new(tiling::TilingState::new());
                let (preset_tx, mut preset_tray_rx) = broadcast::channel::<String>(16);

                // Setup tray icon (needs config for preset menu items)
                if let Err(e) = tray::setup_tray(&handle2, &config, preset_tx.clone()) {
                    log::error!("Failed to setup tray: {e}");
                }

                let state = AppState {
                    config: config_arc.clone(),
                    tiling_state: tiling_state_arc.clone(),
                    backend: backend_arc.clone(),
                    preset_tx: preset_tx.clone(),
                };

                handle2.manage(state);

                // Show settings window on first run
                if first_run {
                    if let Err(e) = tauri::WebviewWindowBuilder::new(
                        &handle2,
                        "settings",
                        tauri::WebviewUrl::App("index.html".into()),
                    )
                    .title("Pave Settings")
                    .inner_size(480.0, 560.0)
                    .resizable(false)
                    .center()
                    .build()
                    {
                        log::warn!("Failed to show first-run settings: {e}");
                    }
                }

                // Listen for shortcut presses and resize events via D-Bus callbacks
                loop {
                    tokio::select! {
                        result = shortcut_rx.recv() => {
                            match result {
                                Ok(action) => {
                                    log::info!("Processing shortcut: {action}");
                                    let cfg = config_arc.read().await.clone();
                                    let wm = backend_arc.clone();
                                    let ts = tiling_state_arc.clone();

                                    let result = match action.as_str() {
                                        "pave_maximize" => {
                                            tiling::handle_maximize(wm.as_ref(), &cfg, &ts).await
                                        }
                                        "pave_snap_left" => {
                                            tiling::handle_snap(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                tiling::SnapSide::Left,
                                            )
                                            .await
                                        }
                                        "pave_snap_right" => {
                                            tiling::handle_snap(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                tiling::SnapSide::Right,
                                            )
                                            .await
                                        }
                                        _ => {
                                            log::debug!("Unknown shortcut action: {action}");
                                            Ok(())
                                        }
                                    };

                                    if let Err(e) = result {
                                        log::error!("Tiling action '{action}' failed: {e}");
                                    }
                                }
                                Err(e) => {
                                    log::error!("Shortcut receiver error: {e}");
                                    break;
                                }
                            }
                        }
                        result = resize_rx.recv() => {
                            match result {
                                Ok(payload) => {
                                    log::info!("Processing resize event");
                                    match serde_json::from_str::<tiling::ResizeEvent>(&payload) {
                                        Ok(event) => {
                                            let cfg = config_arc.read().await.clone();
                                            if let Err(e) = tiling::handle_resize_event(
                                                backend_arc.as_ref(),
                                                &cfg,
                                                &event,
                                            ).await {
                                                log::error!("Resize event handling failed: {e}");
                                            }
                                        }
                                        Err(e) => {
                                            log::error!("Failed to parse resize event: {e}");
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::error!("Resize receiver error: {e}");
                                    break;
                                }
                            }
                        }
                        // Preset activation via D-Bus (CLI / CiderDeck)
                        result = preset_rx.recv() => {
                            if let Ok(name) = result {
                                log::info!("Activating preset via D-Bus: {name}");
                                let cfg = config_arc.read().await;
                                if let Some(preset) = cfg.presets.iter().find(|p| p.name == name).cloned() {
                                    drop(cfg);
                                    if let Err(e) = presets::activate_preset(backend_arc.as_ref(), &preset).await {
                                        log::error!("Preset activation failed: {e}");
                                    }
                                } else {
                                    log::warn!("Preset '{name}' not found");
                                }
                            }
                        }
                        // Preset activation via tray menu
                        result = preset_tray_rx.recv() => {
                            if let Ok(name) = result {
                                log::info!("Activating preset via tray: {name}");
                                let cfg = config_arc.read().await;
                                if let Some(preset) = cfg.presets.iter().find(|p| p.name == name).cloned() {
                                    drop(cfg);
                                    if let Err(e) = presets::activate_preset(backend_arc.as_ref(), &preset).await {
                                        log::error!("Preset activation failed: {e}");
                                    }
                                } else {
                                    log::warn!("Preset '{name}' not found");
                                }
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // Keep the app running when all windows are closed (tray-only app)
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
