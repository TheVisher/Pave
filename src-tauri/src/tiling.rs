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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapSide {
    Left,
    Right,
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

    #[allow(dead_code)]
    fn clear_last_action(&self, window_id: &str) {
        let mut actions = self.last_action.lock().unwrap();
        actions.remove(window_id);
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

    // Check if already snapped to this side on this monitor
    let already_snapped = rects_approx_equal(&current_rect, &standard_rect);

    // Also check if already in a smart-detected space (via last action tracking)
    let was_last_action_same = state
        .get_last_action(&window.id)
        .map(|(a, m)| a == action_name && m == mon_idx)
        .unwrap_or(false);

    if already_snapped || was_last_action_same {
        // Already snapped to this side -> cycle to next monitor
        let next_idx = next_monitor_index(mon_idx, &sorted, &config.excluded_monitors);
        if next_idx != mon_idx {
            let next_monitor = &monitors[next_idx];
            // move_window handles unmaximize atomically
            let next_rect = match side {
                SnapSide::Left => snap_left_rect(next_monitor, config.gap_size),
                SnapSide::Right => snap_right_rect(next_monitor, config.gap_size),
            };
            wm.move_window(
                &window.id,
                next_rect.x,
                next_rect.y,
                next_rect.width,
                next_rect.height,
            )
            .await?;
            state.set_last_action(&window.id, action_name, next_idx);
        }
        return Ok(());
    }

    // Not already snapped -> try smart space detection first
    // move_window handles unmaximize atomically if needed
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

/// Find the adjacent window on the given edge of the resized window.
/// Uses the **old** geometry of the resized window for adjacency detection,
/// since the edge has already moved.
fn find_adjacent_window<'a>(
    edge: ResizedEdge,
    old_rect: &Rect,
    windows: &'a [WindowInfo],
    resized_window_id: &str,
    monitor: &MonitorInfo,
    gap: u32,
) -> Option<&'a WindowInfo> {
    let tolerance = gap as i32 + 20;

    windows.iter().find(|w| {
        if w.id == resized_window_id || w.minimized || !is_window_on_monitor(w, monitor) {
            return false;
        }

        match edge {
            ResizedEdge::Right => {
                // Adjacent window's left edge should be near the old right edge
                let old_right = old_rect.x + old_rect.width;
                let distance = (w.x - old_right).abs();
                distance <= tolerance && vertical_overlap(old_rect.y, old_rect.height, w.y, w.height)
            }
            ResizedEdge::Left => {
                // Adjacent window's right edge should be near the old left edge
                let adj_right = w.x + w.width;
                let distance = (adj_right - old_rect.x).abs();
                distance <= tolerance && vertical_overlap(old_rect.y, old_rect.height, w.y, w.height)
            }
            ResizedEdge::Bottom => {
                // Adjacent window's top edge should be near the old bottom edge
                let old_bottom = old_rect.y + old_rect.height;
                let distance = (w.y - old_bottom).abs();
                distance <= tolerance && horizontal_overlap(old_rect.x, old_rect.width, w.x, w.width)
            }
            ResizedEdge::Top => {
                // Adjacent window's bottom edge should be near the old top edge
                let adj_bottom = w.y + w.height;
                let distance = (adj_bottom - old_rect.y).abs();
                distance <= tolerance && horizontal_overlap(old_rect.x, old_rect.width, w.x, w.width)
            }
        }
    })
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

    let adj = find_adjacent_window(
        edge,
        &event.old_geometry,
        &windows,
        &event.window_id,
        monitor,
        config.gap_size,
    );

    let adj = match adj {
        Some(w) => w,
        None => {
            log::debug!("Resize event: no adjacent window found on {:?} edge", edge);
            return Ok(());
        }
    };

    log::info!(
        "Resize event: adjacent window '{}' ({}), resizing",
        adj.title, adj.id
    );

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
            "Resize event: skipping — adjacent window would be too small ({}x{})",
            new_rect.width, new_rect.height
        );
        return Ok(());
    }

    wm.move_window(&adj.id, new_rect.x, new_rect.y, new_rect.width, new_rect.height)
        .await?;

    log::info!(
        "Resize event: resized '{}' to ({},{} {}x{})",
        adj.title, new_rect.x, new_rect.y, new_rect.width, new_rect.height
    );

    Ok(())
}
