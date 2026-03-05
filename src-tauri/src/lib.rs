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

#[tauri::command]
async fn throw_to_monitor(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let cfg = state.config.read().await.clone();
    tiling::throw_to_next_monitor(state.backend.as_ref(), &cfg).await
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
            delete_preset,
            throw_to_monitor
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
                let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

                // Setup tray icon (needs config for preset menu items)
                if let Err(e) = tray::setup_tray(
                    &handle2,
                    &config,
                    preset_tx.clone(),
                    shutdown_tx.clone(),
                    backend_arc.clone(),
                    config_arc.clone(),
                ) {
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

                // Scan existing windows to populate zone tracker + last_action
                if let Err(e) = tiling::scan_existing_windows(
                    backend_arc.as_ref(),
                    &config,
                    &tiling_state_arc,
                ).await {
                    log::error!("Startup window scan failed: {e}");
                }

                // Session Ghost: restore last session on startup if enabled
                if config.restore_session {
                    let cfg = config_arc.read().await;
                    if let Some(session_preset) = cfg.presets.iter().find(|p| p.name == "__last_session__").cloned() {
                        drop(cfg);
                        log::info!("Restoring last session ({} windows)", session_preset.slots.len());
                        if let Err(e) = presets::activate_preset(backend_arc.as_ref(), &session_preset).await {
                            log::error!("Session restore failed: {e}");
                        }
                    }
                }

                // Helper closure for preset/throw dispatch via D-Bus or tray channels
                async fn handle_preset_or_throw(
                    name: &str,
                    source: &str,
                    backend: &KWinBackend,
                    config_arc: &Arc<RwLock<PaveConfig>>,
                ) {
                    if name == "__throw__" {
                        let cfg = config_arc.read().await.clone();
                        if let Err(e) = tiling::throw_to_next_monitor(backend, &cfg).await {
                            log::error!("Throw to monitor failed: {e}");
                        }
                    } else {
                        log::info!("Activating preset via {source}: {name}");
                        let cfg = config_arc.read().await;
                        if let Some(preset) = cfg.presets.iter().find(|p| p.name == name).cloned() {
                            drop(cfg);
                            if let Err(e) = presets::activate_preset(backend, &preset).await {
                                log::error!("Preset activation failed: {e}");
                            }
                        } else {
                            log::warn!("Preset '{name}' not found");
                        }
                    }
                }

                // Listen for shortcut presses, resize events, presets, and shutdown
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
                                        "pave_snap_up" => {
                                            tiling::handle_snap_vertical(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                tiling::SnapVertical::Up,
                                            )
                                            .await
                                        }
                                        "pave_snap_down" => {
                                            tiling::handle_snap_vertical(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                tiling::SnapVertical::Down,
                                            )
                                            .await
                                        }
                                        "pave_restore" => {
                                            tiling::handle_restore(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                            )
                                            .await
                                        }
                                        "pave_grow" => {
                                            tiling::handle_grow_shrink(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                true,
                                            )
                                            .await
                                        }
                                        "pave_shrink" => {
                                            tiling::handle_grow_shrink(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
                                                false,
                                            )
                                            .await
                                        }
                                        "pave_tab_cycle" => {
                                            tiling::handle_tab_cycle(
                                                wm.as_ref(),
                                                &cfg,
                                                &ts,
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
                        // Preset activation or throw via D-Bus (CLI / CiderDeck)
                        result = preset_rx.recv() => {
                            if let Ok(name) = result {
                                handle_preset_or_throw(&name, "D-Bus", backend_arc.as_ref(), &config_arc).await;
                            }
                        }
                        // Preset activation or throw via tray menu
                        result = preset_tray_rx.recv() => {
                            if let Ok(name) = result {
                                handle_preset_or_throw(&name, "tray", backend_arc.as_ref(), &config_arc).await;
                            }
                        }
                        // Session Ghost: capture session on shutdown
                        _ = shutdown_rx.recv() => {
                            log::info!("Shutdown signal received, saving session...");
                            match presets::capture_preset(backend_arc.as_ref(), "__last_session__".to_string()).await {
                                Ok(session_preset) => {
                                    let mut cfg = config_arc.write().await;
                                    if let Some(pos) = cfg.presets.iter().position(|p| p.name == "__last_session__") {
                                        cfg.presets[pos] = session_preset;
                                    } else {
                                        cfg.presets.push(session_preset);
                                    }
                                    if let Err(e) = cfg.save() {
                                        log::error!("Failed to save session preset: {e}");
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to capture session: {e}");
                                }
                            }
                            handle2.exit(0);
                            break;
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
