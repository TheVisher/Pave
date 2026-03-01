use crate::config::{Preset, WindowSlot};
use crate::platform::kwin::KWinBackend;
use std::time::Duration;

/// Snapshot all non-minimized windows into a new Preset.
pub async fn capture_preset(wm: &KWinBackend, name: String) -> Result<Preset, String> {
    let windows = wm.get_windows().await?;

    let slots: Vec<WindowSlot> = windows
        .into_iter()
        .filter(|w| !w.minimized && !w.resource_class.is_empty())
        .map(|w| WindowSlot {
            window_class: w.resource_class.to_lowercase(),
            launch_command: None,
            monitor: w.screen,
            x: w.x,
            y: w.y,
            width: w.width,
            height: w.height,
        })
        .collect();

    if slots.is_empty() {
        return Err("No visible windows to capture".to_string());
    }

    Ok(Preset { name, slots })
}

/// Activate a preset: move matching windows into position, launch missing apps.
pub async fn activate_preset(wm: &KWinBackend, preset: &Preset) -> Result<(), String> {
    let windows = wm.get_windows().await?;

    // Track which slots still need launching
    let mut needs_launch: Vec<&WindowSlot> = Vec::new();

    for slot in &preset.slots {
        // Find first window matching this resource class
        let matched = windows
            .iter()
            .find(|w| w.resource_class.to_lowercase() == slot.window_class);

        match matched {
            Some(win) => {
                // Unmaximize first if needed, then move
                if win.maximized {
                    let _ = wm.unmaximize_window(&win.id).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                wm.move_window(&win.id, slot.x, slot.y, slot.width, slot.height)
                    .await?;
            }
            None => {
                if slot.launch_command.is_some() {
                    needs_launch.push(slot);
                }
            }
        }
    }

    // Launch missing apps
    if !needs_launch.is_empty() {
        for slot in &needs_launch {
            if let Some(cmd) = &slot.launch_command {
                log::info!("Launching: {cmd}");
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if let Some((program, args)) = parts.split_first() {
                    let _ = std::process::Command::new(program)
                        .args(args)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
            }
        }

        // Wait for apps to start, then arrange them
        tokio::time::sleep(Duration::from_secs(2)).await;

        let windows = wm.get_windows().await?;
        for slot in &needs_launch {
            if let Some(win) = windows
                .iter()
                .find(|w| w.resource_class.to_lowercase() == slot.window_class)
            {
                if win.maximized {
                    let _ = wm.unmaximize_window(&win.id).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let _ = wm
                    .move_window(&win.id, slot.x, slot.y, slot.width, slot.height)
                    .await;
            }
        }
    }

    Ok(())
}
