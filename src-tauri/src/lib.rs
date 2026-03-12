mod config;
mod platform;
mod presets;
mod tiling;
mod tray;

use config::{PaveConfig, Preset};
use platform::kwin::KWinBackend;
use platform::MonitorInfo;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, RwLock};

struct AppState {
    config: Arc<RwLock<PaveConfig>>,
    tiling_state: Arc<tiling::TilingState>,
    backend: Arc<KWinBackend>,
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
    app: AppHandle,
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
    tray::refresh_tray_menu(&app, &cfg);
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
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let mut cfg = state.config.write().await;
    cfg.presets.retain(|p| p.name != name);
    cfg.save()?;
    tray::refresh_tray_menu(&app, &cfg);
    Ok(())
}

#[tauri::command]
async fn throw_to_monitor(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let cfg = state.config.read().await.clone();
    tiling::throw_to_next_monitor(state.backend.as_ref(), &cfg).await
}

#[tauri::command]
async fn resurface_zones(state: tauri::State<'_, AppState>) -> Result<(), String> {
    tiling::resurface_all_zones(state.backend.as_ref(), &state.tiling_state).await
}

/// Kill any stale Pave processes left from a previous run.
fn kill_stale_processes() {
    let my_pid = std::process::id();
    let output = match std::process::Command::new("pgrep")
        .args(["-x", "pave"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return,
    };

    let pids = String::from_utf8_lossy(&output.stdout);
    for line in pids.lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            if pid != my_pid {
                log::info!("Killing stale Pave process (PID {pid})");
                let _ = std::process::Command::new("kill")
                    .arg(pid.to_string())
                    .status();
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Work around WebKit Wayland protocol error when creating windows
    // by disabling the DMABuf renderer that triggers the crash
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    env_logger::init();

    // Kill any stale Pave processes from previous runs (zombie quit, crash, etc.)
    // This prevents D-Bus name conflicts that break shortcut registration.
    kill_stale_processes();

    let port: u16 = 9527;

    tauri::Builder::default()
        .plugin(tauri_plugin_localhost::Builder::new(port).build())
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
            throw_to_monitor,
            resurface_zones
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // Initialize backend and state in async context
            let handle2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                // Retry KWin backend init with backoff — on login, KWin's D-Bus
                // may not be ready yet when Pave autostarts.
                let (backend, mut shortcut_rx, mut resize_rx, mut preset_rx, mut window_event_rx) = {
                    let mut attempt = 0u32;
                    loop {
                        match KWinBackend::new().await {
                            Ok(b) => break b,
                            Err(e) => {
                                attempt += 1;
                                if attempt >= 10 {
                                    log::error!("Failed to initialize KWin backend after {attempt} attempts: {e}");
                                    return;
                                }
                                log::warn!("KWin backend not ready (attempt {attempt}/10): {e}");
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            }
                        }
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
                };

                handle2.manage(state);

                // Show settings window on first run
                if first_run {
                    let url = format!("http://localhost:{port}/index.html").parse().unwrap();
                    if let Err(e) = tauri::WebviewWindowBuilder::new(
                        &handle2,
                        "settings",
                        tauri::WebviewUrl::External(url),
                    )
                    .title("Pave Settings")
                    .inner_size(600.0, 700.0)
                    .resizable(false)
                    .center()
                    .build()
                    {
                        log::warn!("Failed to show first-run settings: {e}");
                    }
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

                // Scan existing windows to populate zone tracker + last_action
                // (runs after session restore so windows are in their final positions)
                if let Err(e) = tiling::scan_existing_windows(
                    backend_arc.as_ref(),
                    &config,
                    &tiling_state_arc,
                ).await {
                    log::error!("Startup window scan failed: {e}");
                }

                // Delayed re-scan: KWin may still be settling windows after reboot.
                // Re-scan after a few seconds to catch any stragglers.
                {
                    let backend = backend_arc.clone();
                    let cfg = config.clone();
                    let ts = tiling_state_arc.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        log::info!("Running delayed startup re-scan");
                        if let Err(e) = tiling::scan_existing_windows(
                            backend.as_ref(),
                            &cfg,
                            &ts,
                        ).await {
                            log::error!("Delayed startup scan failed: {e}");
                        }
                    });
                }

                // Helper closure for preset/throw dispatch via D-Bus or tray channels
                async fn handle_preset_or_throw(
                    name: &str,
                    source: &str,
                    backend: &KWinBackend,
                    config_arc: &Arc<RwLock<PaveConfig>>,
                    tiling_state: &Arc<tiling::TilingState>,
                ) {
                    if name == "__throw__" {
                        let cfg = config_arc.read().await.clone();
                        if let Err(e) = tiling::throw_to_next_monitor(backend, &cfg).await {
                            log::error!("Throw to monitor failed: {e}");
                        }
                    } else if name == "__resurface__" {
                        if let Err(e) = tiling::resurface_all_zones(backend, tiling_state).await {
                            log::error!("Resurface zones failed: {e}");
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
                                handle_preset_or_throw(&name, "D-Bus", backend_arc.as_ref(), &config_arc, &tiling_state_arc).await;
                            }
                        }
                        // Preset activation or throw via tray menu
                        result = preset_tray_rx.recv() => {
                            if let Ok(name) = result {
                                handle_preset_or_throw(&name, "tray", backend_arc.as_ref(), &config_arc, &tiling_state_arc).await;
                            }
                        }
                        // Window closed: remove from zone tracker, surface next in stack
                        result = window_event_rx.recv() => {
                            if let Ok(window_id) = result {
                                log::info!("Window removed event: {window_id}");
                                let cfg = config_arc.read().await.clone();
                                if let Some((_zone_id, surface_entries)) = tiling_state_arc.zone_find_and_remove(&window_id) {
                                    if cfg.auto_surface_tabs && !surface_entries.is_empty() {
                                        for entry in &surface_entries {
                                            if let Err(e) = tiling::surface_zone_entry(backend_arc.as_ref(), &tiling_state_arc, entry).await {
                                                log::error!("Failed to surface after window close: {e}");
                                            }
                                        }
                                    }
                                }
                                // Also clean last_action for the closed window
                                tiling_state_arc.clear_last_action(&window_id);
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
