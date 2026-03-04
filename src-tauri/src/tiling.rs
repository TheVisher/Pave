use crate::config::PaveConfig;
use crate::platform::kwin::KWinBackend;
use crate::platform::{MonitorInfo, WindowInfo};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Tracks state for repeat-press detection and snap positions
pub struct TilingState {
    /// Last action per window: (action_name, monitor_index, timestamp)
    last_action: Mutex<HashMap<String, (String, usize, Instant)>>,
    /// Original geometry before first snap/maximize/grow (for restore)
    pre_snap_geometry: Mutex<HashMap<String, Rect>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapVertical {
    Up,
    Down,
}

/// Geometry rectangle
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A resize event from KWin's interactiveMoveResizeFinished signal
#[derive(Debug, Clone, Deserialize)]
pub struct ResizeEvent {
    #[serde(rename = "windowId")]
    pub window_id: String,
    pub screen: String,
    #[serde(rename = "oldGeometry")]
    pub old_geometry: Rect,
    #[serde(rename = "newGeometry")]
    pub new_geometry: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResizedEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl TilingState {
    pub fn new() -> Self {
        Self {
            last_action: Mutex::new(HashMap::new()),
            pre_snap_geometry: Mutex::new(HashMap::new()),
        }
    }

    /// Get the last action for a window, if it was recent (within 2 seconds)
    fn get_last_action(&self, window_id: &str) -> Option<(String, usize)> {
        let actions = self.last_action.lock().unwrap();
        if let Some((action, monitor_idx, time)) = actions.get(window_id) {
            if time.elapsed().as_secs() < 2 {
                return Some((action.clone(), *monitor_idx));
            }
        }
        None
    }

    fn set_last_action(&self, window_id: &str, action: &str, monitor_idx: usize) {
        let mut actions = self.last_action.lock().unwrap();
        actions.insert(
            window_id.to_string(),
            (action.to_string(), monitor_idx, Instant::now()),
        );
    }

    fn clear_last_action(&self, window_id: &str) {
        let mut actions = self.last_action.lock().unwrap();
        actions.remove(window_id);
    }

    /// Save original geometry before first snap. Returns true if saved (no prior entry).
    fn save_pre_snap_geometry(&self, window_id: &str, rect: Rect) -> bool {
        let mut geo = self.pre_snap_geometry.lock().unwrap();
        if geo.contains_key(window_id) {
            false
        } else {
            geo.insert(window_id.to_string(), rect);
            true
        }
    }

    /// Remove and return pre-snap geometry for restore.
    fn take_pre_snap_geometry(&self, window_id: &str) -> Option<Rect> {
        let mut geo = self.pre_snap_geometry.lock().unwrap();
        geo.remove(window_id)
    }
}

/// Sort monitors by physical position (left-to-right, then top-to-bottom)
pub fn sort_monitors(monitors: &[MonitorInfo]) -> Vec<(usize, &MonitorInfo)> {
    let mut indexed: Vec<(usize, &MonitorInfo)> = monitors.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.x.cmp(&b.1.x).then(a.1.y.cmp(&b.1.y))
    });
    indexed
}

/// Find which monitor a window is primarily on
pub fn find_window_monitor(window: &WindowInfo, monitors: &[MonitorInfo]) -> usize {
    let win_cx = window.x + window.width / 2;
    let win_cy = window.y + window.height / 2;

    monitors
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| {
            let m_cx = m.x + m.width / 2;
            let m_cy = m.y + m.height / 2;
            let dx = (win_cx - m_cx).abs();
            let dy = (win_cy - m_cy).abs();
            dx + dy
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Calculate the "almost maximize" geometry for a monitor with gaps
fn almost_maximize_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    Rect {
        x: monitor.x + g,
        y: monitor.y + g,
        width: monitor.width - 2 * g,
        height: monitor.height - 2 * g,
    }
}

/// Calculate left-half snap geometry
fn snap_left_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    Rect {
        x: monitor.x + g,
        y: monitor.y + g,
        width: monitor.width / 2 - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Calculate right-half snap geometry
fn snap_right_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let half_w = monitor.width / 2;
    Rect {
        x: monitor.x + half_w + g / 2,
        y: monitor.y + g,
        width: monitor.width - half_w - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Calculate top-left quarter snap geometry
fn snap_top_left_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    Rect {
        x: monitor.x + g,
        y: monitor.y + g,
        width: monitor.width / 2 - g - g / 2,
        height: monitor.height / 2 - g - g / 2,
    }
}

/// Calculate top-right quarter snap geometry
fn snap_top_right_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let half_w = monitor.width / 2;
    Rect {
        x: monitor.x + half_w + g / 2,
        y: monitor.y + g,
        width: monitor.width - half_w - g - g / 2,
        height: monitor.height / 2 - g - g / 2,
    }
}

/// Calculate bottom-left quarter snap geometry
fn snap_bottom_left_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let half_h = monitor.height / 2;
    Rect {
        x: monitor.x + g,
        y: monitor.y + half_h + g / 2,
        width: monitor.width / 2 - g - g / 2,
        height: monitor.height - half_h - g - g / 2,
    }
}

/// Calculate bottom-right quarter snap geometry
fn snap_bottom_right_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let half_w = monitor.width / 2;
    let half_h = monitor.height / 2;
    Rect {
        x: monitor.x + half_w + g / 2,
        y: monitor.y + half_h + g / 2,
        width: monitor.width - half_w - g - g / 2,
        height: monitor.height - half_h - g - g / 2,
    }
}

/// Calculate left two-thirds snap geometry
fn snap_left_two_thirds_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    Rect {
        x: monitor.x + g,
        y: monitor.y + g,
        width: monitor.width * 2 / 3 - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Calculate left one-third snap geometry
fn snap_left_one_third_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    Rect {
        x: monitor.x + g,
        y: monitor.y + g,
        width: monitor.width / 3 - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Calculate right two-thirds snap geometry
fn snap_right_two_thirds_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let one_third = monitor.width / 3;
    Rect {
        x: monitor.x + one_third + g / 2,
        y: monitor.y + g,
        width: monitor.width - one_third - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Calculate right one-third snap geometry
fn snap_right_one_third_rect(monitor: &MonitorInfo, gap: u32) -> Rect {
    let g = gap as i32;
    let two_thirds = monitor.width * 2 / 3;
    Rect {
        x: monitor.x + two_thirds + g / 2,
        y: monitor.y + g,
        width: monitor.width - two_thirds - g - g / 2,
        height: monitor.height - 2 * g,
    }
}

/// Check if two rects are approximately equal (within 15px tolerance)
fn rects_approx_equal(a: &Rect, b: &Rect) -> bool {
    let tol = 15;
    (a.x - b.x).abs() <= tol
        && (a.y - b.y).abs() <= tol
        && (a.width - b.width).abs() <= tol
        && (a.height - b.height).abs() <= tol
}

fn window_to_rect(w: &WindowInfo) -> Rect {
    Rect {
        x: w.x,
        y: w.y,
        width: w.width,
        height: w.height,
    }
}

/// Try to find empty space on a given side of the monitor and return a rect that fills it.
/// Looks at other non-minimized windows on the same monitor to find gaps.
fn find_empty_space(
    side: SnapSide,
    monitor: &MonitorInfo,
    windows: &[WindowInfo],
    active_window_id: &str,
    gap: u32,
) -> Option<Rect> {
    let g = gap as i32;

    // Get other visible windows on this monitor
    let other_windows: Vec<&WindowInfo> = windows
        .iter()
        .filter(|w| {
            w.id != active_window_id
                && !w.minimized
                && is_window_on_monitor(w, monitor)
        })
        .collect();

    if other_windows.is_empty() {
        return None; // No other windows to detect space around
    }

    match side {
        SnapSide::Left => {
            // Look for empty space on the left side of the monitor.
            // Find the leftmost edge of other windows.
            let mut min_left_edge = monitor.x + monitor.width;
            for w in &other_windows {
                if w.x < min_left_edge && w.x > monitor.x + monitor.width / 4 {
                    min_left_edge = w.x;
                }
            }

            if min_left_edge > monitor.x + g + 50 && min_left_edge < monitor.x + monitor.width - 50 {
                // There's space on the left
                let available_width = min_left_edge - monitor.x - g - g / 2;
                if available_width > 100 {
                    return Some(Rect {
                        x: monitor.x + g,
                        y: monitor.y + g,
                        width: available_width,
                        height: monitor.height - 2 * g,
                    });
                }
            }
        }
        SnapSide::Right => {
            // Look for empty space on the right side of the monitor.
            // Find the rightmost edge of other windows.
            let mut max_right_edge = monitor.x;
            for w in &other_windows {
                let right = w.x + w.width;
                if right > max_right_edge && right < monitor.x + monitor.width * 3 / 4 {
                    max_right_edge = right;
                }
            }

            if max_right_edge > monitor.x + 50 && max_right_edge < monitor.x + monitor.width - g - 50 {
                // There's space on the right
                let start_x = max_right_edge + g / 2;
                let available_width = monitor.x + monitor.width - g - start_x;
                if available_width > 100 {
                    return Some(Rect {
                        x: start_x,
                        y: monitor.y + g,
                        width: available_width,
                        height: monitor.height - 2 * g,
                    });
                }
            }
        }
    }

    None
}

fn is_window_on_monitor(window: &WindowInfo, monitor: &MonitorInfo) -> bool {
    let win_cx = window.x + window.width / 2;
    let win_cy = window.y + window.height / 2;
    win_cx >= monitor.x
        && win_cx < monitor.x + monitor.width
        && win_cy >= monitor.y
        && win_cy < monitor.y + monitor.height
}

/// Get the next monitor index, skipping excluded ones, wrapping around
fn next_monitor_index(
    current: usize,
    sorted_monitors: &[(usize, &MonitorInfo)],
    excluded: &[String],
) -> usize {
    let total = sorted_monitors.len();
    if total <= 1 {
        return current;
    }

    // Find current position in sorted list
    let current_pos = sorted_monitors
        .iter()
        .position(|(idx, _)| *idx == current)
        .unwrap_or(0);

    // Try each subsequent monitor
    for offset in 1..total {
        let next_pos = (current_pos + offset) % total;
        let (idx, mon) = &sorted_monitors[next_pos];
        if !excluded.contains(&mon.name) {
            return *idx;
        }
    }

    current // All other monitors excluded, stay on current
}

/// Handle the almost-maximize action
pub async fn handle_maximize(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let mon_idx = find_window_monitor(&window, &monitors);
    let monitor = &monitors[mon_idx];

    let almost_rect = almost_maximize_rect(monitor, config.gap_size);
    let current_rect = window_to_rect(&window);

    log::info!(
        "Maximize: window '{}' at ({},{} {}x{}), monitor '{}' ({},{} {}x{}), target ({},{} {}x{}), maximized={}",
        window.title, window.x, window.y, window.width, window.height,
        monitor.name, monitor.x, monitor.y, monitor.width, monitor.height,
        almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height,
        window.maximized
    );

    // Full-size rect = monitor geometry with 1px gap so KWin's window shadow
    // is already rendered, preventing a visual border artifact on transition
    let full_rect = almost_maximize_rect(monitor, 1);

    // Use TilingState tracking (reliable regardless of geometry drift or KDE reporting quirks)
    let last_action = state
        .get_last_action(&window.id)
        .map(|(a, _)| a);
    let was_almost_maximized = last_action.as_deref() == Some("almost_maximize");
    let was_full_maximized = last_action.as_deref() == Some("full_maximize");

    if window.maximized {
        // KWin-maximized (user did it manually) -> unmaximize, then almost-maximize
        log::info!("Action: unmaximize then almost-maximize");
        wm.unmaximize_window(&window.id).await?;
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
    } else if was_full_maximized || rects_approx_equal(&current_rect, &full_rect) {
        // Full-size -> almost-maximize
        log::info!("Action: full-size to almost-maximize");
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
    } else if was_almost_maximized || rects_approx_equal(&current_rect, &almost_rect) {
        // Almost-maximized -> full-size (just geometry, no KWin maximize)
        log::info!("Action: almost-maximize to full-size");
        wm.move_window(&window.id, full_rect.x, full_rect.y, full_rect.width, full_rect.height)
            .await?;
        state.set_last_action(&window.id, "full_maximize", mon_idx);
    } else {
        // Neither -> almost-maximize
        log::info!("Action: almost-maximize");
        state.save_pre_snap_geometry(&window.id, current_rect);
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
    }

    Ok(())
}

/// Handle snap left/right action
pub async fn handle_snap(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
    side: SnapSide,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let windows = wm.get_windows().await?;
    let mon_idx = find_window_monitor(&window, &monitors);
    let monitor = &monitors[mon_idx];
    let sorted = sort_monitors(&monitors);

    let action_name = match side {
        SnapSide::Left => "snap_left",
        SnapSide::Right => "snap_right",
    };

    let standard_rect = match side {
        SnapSide::Left => snap_left_rect(monitor, config.gap_size),
        SnapSide::Right => snap_right_rect(monitor, config.gap_size),
    };

    let current_rect = window_to_rect(&window);

    // If currently in a quarter, left/right escapes to full half snap on that side
    let last_action_name = state
        .get_last_action(&window.id)
        .map(|(a, _)| a);
    let is_quarter = matches!(
        last_action_name.as_deref(),
        Some("snap_top_left") | Some("snap_top_right") | Some("snap_bottom_left") | Some("snap_bottom_right")
    );

    if is_quarter {
        // Escape from quarter to full half snap on the pressed side
        wm.move_window(
            &window.id,
            standard_rect.x,
            standard_rect.y,
            standard_rect.width,
            standard_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, action_name, mon_idx);
        return Ok(());
    }

    // Build fraction rects for this side on the current monitor
    let half_rect = standard_rect;
    let two_thirds_rect = match side {
        SnapSide::Left => snap_left_two_thirds_rect(monitor, config.gap_size),
        SnapSide::Right => snap_right_two_thirds_rect(monitor, config.gap_size),
    };
    let one_third_rect = match side {
        SnapSide::Left => snap_left_one_third_rect(monitor, config.gap_size),
        SnapSide::Right => snap_right_one_third_rect(monitor, config.gap_size),
    };

    // Detect current fraction by geometry match or last_action fallback
    let current_fraction = if rects_approx_equal(&current_rect, &half_rect) {
        Some("half")
    } else if rects_approx_equal(&current_rect, &two_thirds_rect) {
        Some("two_thirds")
    } else if rects_approx_equal(&current_rect, &one_third_rect) {
        Some("one_third")
    } else {
        // Fall back to last_action for smart-space positions
        let on_same_monitor = state
            .get_last_action(&window.id)
            .map(|(_, m)| m == mon_idx)
            .unwrap_or(false);
        if on_same_monitor {
            match last_action_name.as_deref() {
                Some(a) if a == action_name => Some("half"),
                Some(a) if a == &format!("{action_name}_two_thirds") => Some("two_thirds"),
                Some(a) if a == &format!("{action_name}_one_third") => Some("one_third"),
                _ => None,
            }
        } else {
            None
        }
    };

    if let Some(fraction) = current_fraction {
        // Cycle fractions: 1/2 → 2/3 → 1/3 → next monitor 1/2 → ...
        let (target_rect, target_action, target_mon) = match fraction {
            "half" => (two_thirds_rect, format!("{action_name}_two_thirds"), mon_idx),
            "two_thirds" => (one_third_rect, format!("{action_name}_one_third"), mon_idx),
            "one_third" | _ => {
                // Try next monitor
                let next_idx = next_monitor_index(mon_idx, &sorted, &config.excluded_monitors);
                if next_idx != mon_idx {
                    let next_monitor = &monitors[next_idx];
                    let next_rect = match side {
                        SnapSide::Left => snap_left_rect(next_monitor, config.gap_size),
                        SnapSide::Right => snap_right_rect(next_monitor, config.gap_size),
                    };
                    (next_rect, action_name.to_string(), next_idx)
                } else {
                    // Single monitor: wrap back to 1/2
                    (half_rect, action_name.to_string(), mon_idx)
                }
            }
        };

        wm.move_window(
            &window.id,
            target_rect.x,
            target_rect.y,
            target_rect.width,
            target_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, &target_action, target_mon);
        return Ok(());
    }

    // Fresh snap — save pre-snap geometry, try smart space, default to 1/2
    state.save_pre_snap_geometry(&window.id, current_rect);

    if let Some(smart_rect) = find_empty_space(side, monitor, &windows, &window.id, config.gap_size) {
        wm.move_window(
            &window.id,
            smart_rect.x,
            smart_rect.y,
            smart_rect.width,
            smart_rect.height,
        )
        .await?;
    } else {
        wm.move_window(
            &window.id,
            standard_rect.x,
            standard_rect.y,
            standard_rect.width,
            standard_rect.height,
        )
        .await?;
    }

    state.set_last_action(&window.id, action_name, mon_idx);
    Ok(())
}

/// Handle snap up/down action (quarter tiling)
pub async fn handle_snap_vertical(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
    direction: SnapVertical,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let mon_idx = find_window_monitor(&window, &monitors);
    let monitor = &monitors[mon_idx];

    let last_action = state
        .get_last_action(&window.id)
        .map(|(a, _)| a);

    // Determine side context from last action
    let side_context = last_action.as_deref().and_then(|a| {
        if a.starts_with("snap_left") || a.starts_with("snap_top_left") || a.starts_with("snap_bottom_left") {
            Some("left")
        } else if a.starts_with("snap_right") || a.starts_with("snap_top_right") || a.starts_with("snap_bottom_right") {
            Some("right")
        } else {
            None
        }
    });

    let side_context = match side_context {
        Some(s) => s,
        None => {
            log::debug!("Snap vertical: no snap context, ignoring");
            return Ok(());
        }
    };

    // Preserve current width and x — just split vertically
    let g = config.gap_size as i32;
    let current_rect = window_to_rect(&window);
    let half_h = monitor.height / 2;

    let target_rect = match direction {
        SnapVertical::Up => Rect {
            x: current_rect.x,
            y: monitor.y + g,
            width: current_rect.width,
            height: half_h - g - g / 2,
        },
        SnapVertical::Down => Rect {
            x: current_rect.x,
            y: monitor.y + half_h + g / 2,
            width: current_rect.width,
            height: monitor.height - half_h - g - g / 2,
        },
    };

    let target_action = match (side_context, direction) {
        ("left", SnapVertical::Up) => "snap_top_left",
        ("left", SnapVertical::Down) => "snap_bottom_left",
        ("right", SnapVertical::Up) => "snap_top_right",
        ("right", SnapVertical::Down) => "snap_bottom_right",
        _ => unreachable!(),
    };

    log::info!(
        "Snap vertical: {} -> {} at ({},{} {}x{})",
        last_action.as_deref().unwrap_or("none"),
        target_action,
        target_rect.x, target_rect.y, target_rect.width, target_rect.height
    );

    wm.move_window(
        &window.id,
        target_rect.x,
        target_rect.y,
        target_rect.width,
        target_rect.height,
    )
    .await?;

    state.set_last_action(&window.id, target_action, mon_idx);
    Ok(())
}

/// Restore a window to its pre-snap geometry
pub async fn handle_restore(
    wm: &KWinBackend,
    _config: &PaveConfig,
    state: &TilingState,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let saved = state.take_pre_snap_geometry(&window.id);
    match saved {
        Some(rect) => {
            log::info!(
                "Restore: window '{}' -> ({},{} {}x{})",
                window.title, rect.x, rect.y, rect.width, rect.height
            );
            wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height)
                .await?;
            state.clear_last_action(&window.id);
            Ok(())
        }
        None => {
            log::debug!("Restore: no saved geometry for '{}'", window.title);
            Ok(())
        }
    }
}

/// Grow or shrink the active window by 10%
pub async fn handle_grow_shrink(
    wm: &KWinBackend,
    _config: &PaveConfig,
    state: &TilingState,
    grow: bool,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let mon_idx = find_window_monitor(&window, &monitors);
    let monitor = &monitors[mon_idx];

    let current_rect = window_to_rect(&window);

    // Save original geometry before first grow/shrink
    state.save_pre_snap_geometry(&window.id, current_rect);

    let scale = if grow { 1.1_f64 } else { 1.0 / 1.1 };
    let new_w = ((current_rect.width as f64) * scale).round() as i32;
    let new_h = ((current_rect.height as f64) * scale).round() as i32;

    // Center: adjust x/y by half the size delta
    let dw = new_w - current_rect.width;
    let dh = new_h - current_rect.height;
    let new_x = current_rect.x - dw / 2;
    let new_y = current_rect.y - dh / 2;

    // Clamp to monitor bounds, minimum 100px
    let clamped_x = new_x.max(monitor.x);
    let clamped_y = new_y.max(monitor.y);
    let clamped_w = new_w
        .max(100)
        .min(monitor.x + monitor.width - clamped_x);
    let clamped_h = new_h
        .max(100)
        .min(monitor.y + monitor.height - clamped_y);

    log::info!(
        "Grow/shrink: '{}' {} -> ({},{} {}x{})",
        window.title,
        if grow { "grow" } else { "shrink" },
        clamped_x, clamped_y, clamped_w, clamped_h
    );

    wm.move_window(&window.id, clamped_x, clamped_y, clamped_w, clamped_h)
        .await?;

    // No longer in a snap position
    state.clear_last_action(&window.id);
    Ok(())
}

/// Move all non-minimized windows from the active window's monitor to the next monitor
pub async fn throw_to_next_monitor(
    wm: &KWinBackend,
    config: &PaveConfig,
) -> Result<(), String> {
    let active = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    let monitors = wm.get_monitors().await?;
    if monitors.len() < 2 {
        return Err("Need at least 2 monitors".to_string());
    }

    let sorted = sort_monitors(&monitors);
    let source_idx = find_window_monitor(&active, &monitors);
    let target_idx = next_monitor_index(source_idx, &sorted, &config.excluded_monitors);

    if target_idx == source_idx {
        return Err("No other monitor available".to_string());
    }

    let source = &monitors[source_idx];
    let target = &monitors[target_idx];
    let dx = target.x - source.x;
    let dy = target.y - source.y;

    let windows = wm.get_windows().await?;
    let on_source: Vec<&WindowInfo> = windows
        .iter()
        .filter(|w| !w.minimized && is_window_on_monitor(w, source))
        .collect();

    log::info!(
        "Throwing {} windows from '{}' to '{}' (dx={}, dy={})",
        on_source.len(), source.name, target.name, dx, dy
    );

    for w in &on_source {
        wm.move_window(
            &w.id,
            w.x + dx,
            w.y + dy,
            w.width,
            w.height,
        )
        .await?;
    }

    Ok(())
}

// --- Resize event handling ---

/// Detect which edge of the window was resized by comparing old and new geometry.
/// Picks the edge with the largest delta.
fn detect_resized_edge(old: &Rect, new: &Rect) -> Option<ResizedEdge> {
    let left_delta = (new.x - old.x).abs();
    let right_delta = ((new.x + new.width) - (old.x + old.width)).abs();
    let top_delta = (new.y - old.y).abs();
    let bottom_delta = ((new.y + new.height) - (old.y + old.height)).abs();

    let max = left_delta.max(right_delta).max(top_delta).max(bottom_delta);
    if max < 5 {
        return None; // No meaningful edge change
    }

    if max == left_delta {
        Some(ResizedEdge::Left)
    } else if max == right_delta {
        Some(ResizedEdge::Right)
    } else if max == top_delta {
        Some(ResizedEdge::Top)
    } else {
        Some(ResizedEdge::Bottom)
    }
}

/// Check if two windows have sufficient vertical overlap (>50% of the smaller window's height)
fn vertical_overlap(a_y: i32, a_h: i32, b_y: i32, b_h: i32) -> bool {
    let overlap_start = a_y.max(b_y);
    let overlap_end = (a_y + a_h).min(b_y + b_h);
    let overlap = (overlap_end - overlap_start).max(0);
    let min_height = a_h.min(b_h);
    if min_height <= 0 {
        return false;
    }
    overlap * 100 / min_height > 50
}

/// Check if two windows have sufficient horizontal overlap (>50% of the smaller window's width)
fn horizontal_overlap(a_x: i32, a_w: i32, b_x: i32, b_w: i32) -> bool {
    let overlap_start = a_x.max(b_x);
    let overlap_end = (a_x + a_w).min(b_x + b_w);
    let overlap = (overlap_end - overlap_start).max(0);
    let min_width = a_w.min(b_w);
    if min_width <= 0 {
        return false;
    }
    overlap * 100 / min_width > 50
}

/// Find all adjacent windows on the given edge of the resized window.
/// Uses the **old** geometry of the resized window for adjacency detection,
/// since the edge has already moved. Any window whose edge is close enough
/// counts — no overlap threshold, so stacked windows (e.g. two quarters
/// next to a half) are all detected.
fn find_adjacent_windows<'a>(
    edge: ResizedEdge,
    old_rect: &Rect,
    windows: &'a [WindowInfo],
    resized_window_id: &str,
    monitor: &MonitorInfo,
    gap: u32,
) -> Vec<&'a WindowInfo> {
    let tolerance = gap as i32 + 20;

    windows.iter().filter(|w| {
        if w.id == resized_window_id || w.minimized || !is_window_on_monitor(w, monitor) {
            return false;
        }

        match edge {
            ResizedEdge::Right => {
                let old_right = old_rect.x + old_rect.width;
                let distance = (w.x - old_right).abs();
                distance <= tolerance && vertical_overlap(old_rect.y, old_rect.height, w.y, w.height)
            }
            ResizedEdge::Left => {
                let adj_right = w.x + w.width;
                let distance = (adj_right - old_rect.x).abs();
                distance <= tolerance && vertical_overlap(old_rect.y, old_rect.height, w.y, w.height)
            }
            ResizedEdge::Bottom => {
                let old_bottom = old_rect.y + old_rect.height;
                let distance = (w.y - old_bottom).abs();
                distance <= tolerance && horizontal_overlap(old_rect.x, old_rect.width, w.x, w.width)
            }
            ResizedEdge::Top => {
                let adj_bottom = w.y + w.height;
                let distance = (adj_bottom - old_rect.y).abs();
                distance <= tolerance && horizontal_overlap(old_rect.x, old_rect.width, w.x, w.width)
            }
        }
    }).collect()
}

/// Calculate the new geometry for the adjacent window so it fills the remaining
/// space between the resized window's new edge and the monitor edge.
fn calculate_adjacent_resize(
    edge: ResizedEdge,
    new_resized_rect: &Rect,
    adj_window: &WindowInfo,
    monitor: &MonitorInfo,
    gap: u32,
) -> Rect {
    let g = gap as i32;

    match edge {
        ResizedEdge::Right => {
            // Resized window's right edge moved. Adjacent window is to the right.
            // Adjacent window's left edge should start at new right edge + gap.
            let new_x = new_resized_rect.x + new_resized_rect.width + g;
            let new_width = (monitor.x + monitor.width - g) - new_x;
            Rect {
                x: new_x,
                y: adj_window.y,
                width: new_width,
                height: adj_window.height,
            }
        }
        ResizedEdge::Left => {
            // Resized window's left edge moved. Adjacent window is to the left.
            // Adjacent window's right edge should end at new left edge - gap.
            let new_width = (new_resized_rect.x - g) - (monitor.x + g);
            Rect {
                x: monitor.x + g,
                y: adj_window.y,
                width: new_width,
                height: adj_window.height,
            }
        }
        ResizedEdge::Bottom => {
            // Resized window's bottom edge moved. Adjacent window is below.
            let new_y = new_resized_rect.y + new_resized_rect.height + g;
            let new_height = (monitor.y + monitor.height - g) - new_y;
            Rect {
                x: adj_window.x,
                y: new_y,
                width: adj_window.width,
                height: new_height,
            }
        }
        ResizedEdge::Top => {
            // Resized window's top edge moved. Adjacent window is above.
            let new_height = (new_resized_rect.y - g) - (monitor.y + g);
            Rect {
                x: adj_window.x,
                y: monitor.y + g,
                width: adj_window.width,
                height: new_height,
            }
        }
    }
}

/// Handle a resize event: detect which edge moved, find the adjacent window,
/// and resize it to fill the remaining space.
pub async fn handle_resize_event(
    wm: &KWinBackend,
    config: &PaveConfig,
    event: &ResizeEvent,
) -> Result<(), String> {
    let edge = match detect_resized_edge(&event.old_geometry, &event.new_geometry) {
        Some(e) => e,
        None => {
            log::debug!("Resize event: no meaningful edge change detected");
            return Ok(());
        }
    };
    log::info!("Resize event: edge={:?}, window={}", edge, event.window_id);

    let monitors = wm.get_monitors().await?;
    let windows = wm.get_windows().await?;

    // Find the monitor this window is on
    let monitor = if !event.screen.is_empty() {
        monitors.iter().find(|m| m.name == event.screen)
    } else {
        None
    }
    .or_else(|| {
        // Fallback: find monitor by center of old geometry
        let cx = event.old_geometry.x + event.old_geometry.width / 2;
        let cy = event.old_geometry.y + event.old_geometry.height / 2;
        monitors.iter().min_by_key(|m| {
            let mx = m.x + m.width / 2;
            let my = m.y + m.height / 2;
            (cx - mx).abs() + (cy - my).abs()
        })
    });

    let monitor = match monitor {
        Some(m) => m,
        None => {
            log::warn!("Resize event: could not find monitor");
            return Ok(());
        }
    };

    let adjacents = find_adjacent_windows(
        edge,
        &event.old_geometry,
        &windows,
        &event.window_id,
        monitor,
        config.gap_size,
    );

    if adjacents.is_empty() {
        log::debug!("Resize event: no adjacent windows found on {:?} edge", edge);
        return Ok(());
    }

    log::info!("Resize event: found {} adjacent window(s) on {:?} edge", adjacents.len(), edge);

    let adjacent_ids: Vec<String> = adjacents.iter().map(|w| w.id.clone()).collect();

    for adj in &adjacents {
        let new_rect = calculate_adjacent_resize(
            edge,
            &event.new_geometry,
            adj,
            monitor,
            config.gap_size,
        );

        // Sanity check: skip if result would be too small
        if new_rect.width < 100 || new_rect.height < 100 {
            log::info!(
                "Resize event: skipping '{}' — would be too small ({}x{})",
                adj.title, new_rect.width, new_rect.height
            );
            continue;
        }

        wm.move_window(&adj.id, new_rect.x, new_rect.y, new_rect.width, new_rect.height)
            .await?;

        log::info!(
            "Resize event: resized '{}' to ({},{} {}x{})",
            adj.title, new_rect.x, new_rect.y, new_rect.width, new_rect.height
        );
    }

    // Sibling detection: find windows sharing the same edge as the resized window
    // (e.g. Dolphin stacked below Zed — both have the same left x).
    // Move their edge by the same delta so the stack stays aligned.
    let tolerance = config.gap_size as i32 + 20;
    let siblings: Vec<&WindowInfo> = windows.iter().filter(|w| {
        if w.id == event.window_id || w.minimized || !is_window_on_monitor(w, monitor) {
            return false;
        }
        if adjacent_ids.contains(&w.id) {
            return false; // already handled as adjacent
        }
        match edge {
            ResizedEdge::Left => (w.x - event.old_geometry.x).abs() <= tolerance,
            ResizedEdge::Right => {
                let w_right = w.x + w.width;
                let old_right = event.old_geometry.x + event.old_geometry.width;
                (w_right - old_right).abs() <= tolerance
            }
            ResizedEdge::Top => (w.y - event.old_geometry.y).abs() <= tolerance,
            ResizedEdge::Bottom => {
                let w_bottom = w.y + w.height;
                let old_bottom = event.old_geometry.y + event.old_geometry.height;
                (w_bottom - old_bottom).abs() <= tolerance
            }
        }
    }).collect();

    if !siblings.is_empty() {
        log::info!("Resize event: found {} sibling window(s) sharing {:?} edge", siblings.len(), edge);
    }

    for sib in &siblings {
        let new_rect = match edge {
            ResizedEdge::Left => {
                let dx = event.new_geometry.x - event.old_geometry.x;
                Rect {
                    x: sib.x + dx,
                    y: sib.y,
                    width: sib.width - dx,
                    height: sib.height,
                }
            }
            ResizedEdge::Right => {
                let old_right = event.old_geometry.x + event.old_geometry.width;
                let new_right = event.new_geometry.x + event.new_geometry.width;
                let dw = new_right - old_right;
                Rect {
                    x: sib.x,
                    y: sib.y,
                    width: sib.width + dw,
                    height: sib.height,
                }
            }
            ResizedEdge::Top => {
                let dy = event.new_geometry.y - event.old_geometry.y;
                Rect {
                    x: sib.x,
                    y: sib.y + dy,
                    width: sib.width,
                    height: sib.height - dy,
                }
            }
            ResizedEdge::Bottom => {
                let old_bottom = event.old_geometry.y + event.old_geometry.height;
                let new_bottom = event.new_geometry.y + event.new_geometry.height;
                let dh = new_bottom - old_bottom;
                Rect {
                    x: sib.x,
                    y: sib.y,
                    width: sib.width,
                    height: sib.height + dh,
                }
            }
        };

        if new_rect.width < 100 || new_rect.height < 100 {
            log::info!(
                "Resize event: skipping sibling '{}' — would be too small ({}x{})",
                sib.title, new_rect.width, new_rect.height
            );
            continue;
        }

        wm.move_window(&sib.id, new_rect.x, new_rect.y, new_rect.width, new_rect.height)
            .await?;

        log::info!(
            "Resize event: resized sibling '{}' to ({},{} {}x{})",
            sib.title, new_rect.x, new_rect.y, new_rect.width, new_rect.height
        );
    }

    Ok(())
}
