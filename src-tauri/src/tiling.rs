use crate::config::PaveConfig;
use crate::platform::kwin::KWinBackend;
use crate::platform::{MonitorInfo, WindowInfo};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Which logical zone a snap action belongs to
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZoneSide {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Maximize,
}

impl ZoneSide {
    fn from_action(action: &str) -> Option<Self> {
        if action.starts_with("snap_top_left") {
            Some(ZoneSide::TopLeft)
        } else if action.starts_with("snap_top_right") {
            Some(ZoneSide::TopRight)
        } else if action.starts_with("snap_bottom_left") {
            Some(ZoneSide::BottomLeft)
        } else if action.starts_with("snap_bottom_right") {
            Some(ZoneSide::BottomRight)
        } else if action.starts_with("snap_left") {
            Some(ZoneSide::Left)
        } else if action.starts_with("snap_right") {
            Some(ZoneSide::Right)
        } else if action == "almost_maximize" || action == "full_maximize" {
            Some(ZoneSide::Maximize)
        } else {
            None
        }
    }

    /// Returns the child zones that this zone fully covers.
    /// Left covers TopLeft + BottomLeft, Right covers TopRight + BottomRight,
    /// Maximize covers everything.
    fn covered_children(&self) -> &'static [ZoneSide] {
        match self {
            ZoneSide::Left => &[ZoneSide::TopLeft, ZoneSide::BottomLeft],
            ZoneSide::Right => &[ZoneSide::TopRight, ZoneSide::BottomRight],
            ZoneSide::Maximize => &[
                ZoneSide::Left, ZoneSide::Right,
                ZoneSide::TopLeft, ZoneSide::TopRight,
                ZoneSide::BottomLeft, ZoneSide::BottomRight,
            ],
            _ => &[],
        }
    }

    /// Returns the parent zones that fully cover this zone.
    /// TopLeft/BottomLeft are covered by Left and Maximize.
    /// Left/Right are covered by Maximize.
    fn covering_parents(&self) -> &'static [ZoneSide] {
        match self {
            ZoneSide::TopLeft | ZoneSide::BottomLeft => &[ZoneSide::Left, ZoneSide::Maximize],
            ZoneSide::TopRight | ZoneSide::BottomRight => &[ZoneSide::Right, ZoneSide::Maximize],
            ZoneSide::Left | ZoneSide::Right => &[ZoneSide::Maximize],
            _ => &[],
        }
    }

    /// Returns the immediate parent zone (not Maximize).
    fn immediate_parent(&self) -> Option<ZoneSide> {
        match self {
            ZoneSide::TopLeft | ZoneSide::BottomLeft => Some(ZoneSide::Left),
            ZoneSide::TopRight | ZoneSide::BottomRight => Some(ZoneSide::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ZoneId {
    monitor_idx: usize,
    side: ZoneSide,
}

#[derive(Debug, Clone)]
struct ZoneEntry {
    window_id: String,
    snap_action: String,
    geometry: Rect,
}

struct ZoneTracker {
    zones: HashMap<ZoneId, Vec<ZoneEntry>>,
}

impl ZoneTracker {
    fn new() -> Self {
        Self {
            zones: HashMap::new(),
        }
    }

    /// Place a window in a zone. Removes it from any prior zone first.
    /// Returns the IDs of all displaced windows to minimize (same zone + overlapping zones).
    /// Windows in overlapping zones are kept in the tracker so they can be surfaced later.
    fn place_window(
        &mut self,
        zone_id: ZoneId,
        window_id: &str,
        snap_action: &str,
        geometry: Rect,
    ) -> Vec<String> {
        // Remove from any prior zone
        self.remove_window(window_id);

        let mut displaced = Vec::new();

        // Displace the active window in the same zone
        if let Some(entries) = self.zones.get(&zone_id) {
            if let Some(entry) = entries.last() {
                if entry.window_id != window_id {
                    displaced.push(entry.window_id.clone());
                }
            }
        }

        // Collect (but don't remove) windows in covered child zones
        // e.g. snapping to Right minimizes windows in TopRight + BottomRight
        for child_side in zone_id.side.covered_children() {
            let child_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                side: child_side.clone(),
            };
            if let Some(entries) = self.zones.get(&child_zone) {
                for entry in entries {
                    if entry.window_id != window_id && !displaced.contains(&entry.window_id) {
                        displaced.push(entry.window_id.clone());
                    }
                }
            }
        }

        // Collect (but don't remove) windows in parent zones that fully cover this zone
        // e.g. snapping to TopRight minimizes a window in the Right zone
        for parent_side in zone_id.side.covering_parents() {
            let parent_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                side: parent_side.clone(),
            };
            if let Some(entries) = self.zones.get(&parent_zone) {
                for entry in entries {
                    if entry.window_id != window_id && !displaced.contains(&entry.window_id) {
                        displaced.push(entry.window_id.clone());
                    }
                }
            }
        }

        let entries = self.zones.entry(zone_id).or_default();
        entries.push(ZoneEntry {
            window_id: window_id.to_string(),
            snap_action: snap_action.to_string(),
            geometry,
        });

        displaced
    }

    /// Place a window in a zone without displacing — used for startup scan.
    /// Just appends to the zone entries without triggering minimizes.
    fn place_window_silent(
        &mut self,
        zone_id: ZoneId,
        window_id: &str,
        snap_action: &str,
        geometry: Rect,
    ) {
        // Remove from any prior zone first
        self.remove_window(window_id);

        let entries = self.zones.entry(zone_id).or_default();
        entries.push(ZoneEntry {
            window_id: window_id.to_string(),
            snap_action: snap_action.to_string(),
            geometry,
        });
    }

    /// Remove a window from whatever zone it's in. Cleans up empty zones.
    fn remove_window(&mut self, window_id: &str) {
        let mut empty_zones = Vec::new();
        for (zone_id, entries) in self.zones.iter_mut() {
            entries.retain(|e| e.window_id != window_id);
            if entries.is_empty() {
                empty_zones.push(zone_id.clone());
            }
        }
        for zone_id in empty_zones {
            self.zones.remove(&zone_id);
        }
    }

    /// Find which zone contains a window
    fn find_zone(&self, window_id: &str) -> Option<&ZoneId> {
        for (zone_id, entries) in &self.zones {
            if entries.iter().any(|e| e.window_id == window_id) {
                return Some(zone_id);
            }
        }
        None
    }

    /// Cycle tab groups. Returns (entries to show, window IDs to hide).
    ///
    /// Group cycling logic:
    /// - If current window is in a parent zone (e.g. Right) and children exist (TopRight + BottomRight):
    ///   show all children, hide parent
    /// - If current window is in a child zone (e.g. TopRight) and parent exists (Right):
    ///   show parent, hide all children (siblings too)
    /// - If same-zone stacking (multiple windows in exact same zone): cycle within zone
    fn cycle_next(&self, current_window_id: &str) -> Option<(Vec<ZoneEntry>, Vec<String>)> {
        let zone_id = self.find_zone(current_window_id)?;

        // Check for parent-child group cycling first
        // Case 1: Current is in a parent zone, children exist → show children, hide parent
        let children = zone_id.side.covered_children();
        if !children.is_empty() {
            let mut child_entries = Vec::new();
            for child_side in children {
                let child_zone = ZoneId {
                    monitor_idx: zone_id.monitor_idx,
                    side: child_side.clone(),
                };
                if let Some(entries) = self.zones.get(&child_zone) {
                    if let Some(entry) = entries.last() {
                        child_entries.push(entry.clone());
                    }
                }
            }
            if !child_entries.is_empty() {
                return Some((child_entries, vec![current_window_id.to_string()]));
            }
        }

        // Case 2: Current is in a child zone, parent exists → show parent, hide all siblings
        if let Some(parent_side) = zone_id.side.immediate_parent() {
            let parent_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                side: parent_side.clone(),
            };
            if let Some(parent_entries) = self.zones.get(&parent_zone) {
                if let Some(parent_entry) = parent_entries.last() {
                    // Collect all sibling child window IDs to hide
                    let mut to_hide = Vec::new();
                    for child_side in parent_side.covered_children() {
                        let child_zone = ZoneId {
                            monitor_idx: zone_id.monitor_idx,
                            side: child_side.clone(),
                        };
                        if let Some(entries) = self.zones.get(&child_zone) {
                            for entry in entries {
                                if !to_hide.contains(&entry.window_id) {
                                    to_hide.push(entry.window_id.clone());
                                }
                            }
                        }
                    }
                    return Some((vec![parent_entry.clone()], to_hide));
                }
            }
        }

        // Case 3: Same-zone stacking (multiple windows in exact same zone)
        let entries = self.zones.get(zone_id)?;
        if entries.len() < 2 {
            return None;
        }

        let current_idx = entries.iter().position(|e| e.window_id == current_window_id)?;
        let next_idx = (current_idx + 1) % entries.len();
        let next = &entries[next_idx];

        Some((
            vec![next.clone()],
            vec![current_window_id.to_string()],
        ))
    }

    /// Remove stale entries for windows that no longer exist
    fn cleanup_stale_windows(&mut self, existing_ids: &[String]) {
        let mut empty_zones = Vec::new();
        for (zone_id, entries) in self.zones.iter_mut() {
            entries.retain(|e| existing_ids.contains(&e.window_id));
            if entries.is_empty() {
                empty_zones.push(zone_id.clone());
            }
        }
        for zone_id in empty_zones {
            self.zones.remove(&zone_id);
        }
    }

    /// Collect all entries that should be surfaced when a zone is vacated.
    /// Includes same-zone entries AND entries in child zones that were hidden.
    fn collect_surface_entries(&self, zone_id: &ZoneId) -> Vec<ZoneEntry> {
        let mut entries = Vec::new();

        // Surface from the same zone
        if let Some(zone_entries) = self.zones.get(zone_id) {
            if let Some(entry) = zone_entries.last() {
                entries.push(entry.clone());
            }
        }

        // Surface from child zones (e.g. Right vacated → surface TopRight + BottomRight)
        for child_side in zone_id.side.covered_children() {
            let child_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                side: child_side.clone(),
            };
            if let Some(zone_entries) = self.zones.get(&child_zone) {
                if let Some(entry) = zone_entries.last() {
                    entries.push(entry.clone());
                }
            }
        }

        entries
    }

    /// Find any zone entry whose window is minimized (for fallback recovery).
    /// Returns the last entry from the first zone where all windows are minimized.
    fn find_minimized_entry(&self, windows: &[WindowInfo]) -> Option<ZoneEntry> {
        let minimized_ids: Vec<&str> = windows
            .iter()
            .filter(|w| w.minimized)
            .map(|w| w.id.as_str())
            .collect();

        // Find zones where at least one entry is minimized
        for entries in self.zones.values() {
            if let Some(entry) = entries.iter().rev().find(|e| minimized_ids.contains(&e.window_id.as_str())) {
                return Some(entry.clone());
            }
        }
        None
    }

    /// Remove a window and return (zone_id, entries to surface)
    fn find_and_remove(&mut self, window_id: &str) -> Option<(ZoneId, Vec<ZoneEntry>)> {
        let zone_id = self.find_zone(window_id)?.clone();
        self.remove_window(window_id);
        let surface = self.collect_surface_entries(&zone_id);
        Some((zone_id, surface))
    }
}

/// Tracks state for repeat-press detection and snap positions
pub struct TilingState {
    /// Last action per window: (action_name, monitor_index, timestamp)
    last_action: Mutex<HashMap<String, (String, usize, Instant)>>,
    /// Original geometry before first snap/maximize/grow (for restore)
    pre_snap_geometry: Mutex<HashMap<String, Rect>>,
    /// Zone tracker for tab zone system
    zone_tracker: Mutex<ZoneTracker>,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Direction {
    Left,
    Right,
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
            zone_tracker: Mutex::new(ZoneTracker::new()),
        }
    }

    /// Get the last action for a window.
    /// Persists until explicitly cleared or overwritten by a new snap action.
    fn get_last_action(&self, window_id: &str) -> Option<(String, usize)> {
        let actions = self.last_action.lock().unwrap();
        if let Some((action, monitor_idx, _time)) = actions.get(window_id) {
            return Some((action.clone(), *monitor_idx));
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

    /// Place a window in a zone. Returns (displaced window IDs to minimize, entries to surface from vacated zone).
    fn zone_place(
        &self,
        monitor_idx: usize,
        action: &str,
        window_id: &str,
        geometry: Rect,
    ) -> (Vec<String>, Vec<ZoneEntry>) {
        let side = match ZoneSide::from_action(action) {
            Some(s) => s,
            None => return (Vec::new(), Vec::new()),
        };
        let zone_id = ZoneId { monitor_idx, side };

        // Find old zone before move (for auto-surface)
        let old_zone = {
            let tracker = self.zone_tracker.lock().unwrap();
            tracker.find_zone(window_id).cloned()
        };

        let displaced = {
            let mut tracker = self.zone_tracker.lock().unwrap();
            tracker.place_window(zone_id.clone(), window_id, action, geometry)
        };

        // If window moved to a different zone, collect entries to surface
        // (from old zone itself + any child zones it was covering)
        let surface = if let Some(old_zone_id) = old_zone {
            if old_zone_id != zone_id {
                let tracker = self.zone_tracker.lock().unwrap();
                let candidates = tracker.collect_surface_entries(&old_zone_id);
                // Filter out: the window being placed, and windows that were just displaced.
                // This prevents surfacing a window that the new zone covers, which would
                // trigger overlapping-parent checks that minimize the just-placed window.
                candidates
                    .into_iter()
                    .filter(|e| {
                        e.window_id != window_id && !displaced.contains(&e.window_id)
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        (displaced, surface)
    }

    /// Place a window in a zone silently (no displacement) — used for startup scan.
    fn zone_place_silent(&self, monitor_idx: usize, action: &str, window_id: &str, geometry: Rect) {
        let side = match ZoneSide::from_action(action) {
            Some(s) => s,
            None => return,
        };
        let zone_id = ZoneId { monitor_idx, side };
        let mut tracker = self.zone_tracker.lock().unwrap();
        tracker.place_window_silent(zone_id, window_id, action, geometry);
    }

    /// Remove a window from its zone.
    fn zone_remove(&self, window_id: &str) {
        let mut tracker = self.zone_tracker.lock().unwrap();
        tracker.remove_window(window_id);
    }

    /// Cycle tab groups. Returns (entries to show, window IDs to hide).
    fn zone_cycle(&self, current_window_id: &str) -> Option<(Vec<ZoneEntry>, Vec<String>)> {
        let tracker = self.zone_tracker.lock().unwrap();
        tracker.cycle_next(current_window_id)
    }

    /// Remove stale entries for closed windows.
    fn zone_cleanup(&self, existing_ids: &[String]) {
        let mut tracker = self.zone_tracker.lock().unwrap();
        tracker.cleanup_stale_windows(existing_ids);
    }

    /// Remove a window from its zone and return entries to surface (if any).
    fn zone_find_and_remove(&self, window_id: &str) -> Option<(ZoneId, Vec<ZoneEntry>)> {
        let mut tracker = self.zone_tracker.lock().unwrap();
        tracker.find_and_remove(window_id)
    }

    /// Find a minimized window in any zone (fallback for tab cycle recovery).
    fn zone_find_minimized(&self, windows: &[WindowInfo]) -> Option<ZoneEntry> {
        let tracker = self.zone_tracker.lock().unwrap();
        tracker.find_minimized_entry(windows)
    }

    /// Get window IDs from zones that cover this window's zone (parents + children).
    /// Used to minimize overlapping windows when a window is surfaced.
    fn zone_get_overlapping(&self, window_id: &str) -> Vec<String> {
        let tracker = self.zone_tracker.lock().unwrap();
        let zone_id = match tracker.find_zone(window_id) {
            Some(z) => z.clone(),
            None => return Vec::new(),
        };

        let mut to_minimize = Vec::new();

        // Check parent zones (e.g. surfacing TopRight should minimize Right)
        for parent_side in zone_id.side.covering_parents() {
            let parent_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                side: parent_side.clone(),
            };
            if let Some(entries) = tracker.zones.get(&parent_zone) {
                for entry in entries {
                    if entry.window_id != window_id && !to_minimize.contains(&entry.window_id) {
                        to_minimize.push(entry.window_id.clone());
                    }
                }
            }
        }

        to_minimize
    }

    /// Get the side context ("left" or "right") for a window from the zone tracker.
    /// Used as a fallback when last_action has expired.
    fn zone_side_context(&self, window_id: &str) -> Option<&'static str> {
        let tracker = self.zone_tracker.lock().unwrap();
        let zone_id = tracker.find_zone(window_id)?;
        match &zone_id.side {
            ZoneSide::Left | ZoneSide::TopLeft | ZoneSide::BottomLeft => Some("left"),
            ZoneSide::Right | ZoneSide::TopRight | ZoneSide::BottomRight => Some("right"),
            ZoneSide::Maximize => None,
        }
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

/// Find the monitor to the left/right/above/below based on physical layout.
/// Uses center-point proximity — finds the nearest monitor whose center
/// is in the requested direction from the current monitor's center.
fn find_monitor_in_direction(
    current_idx: usize,
    monitors: &[MonitorInfo],
    excluded: &[String],
    direction: Direction,
) -> Option<usize> {
    let cur = &monitors[current_idx];
    let cx = cur.x + cur.width / 2;
    let cy = cur.y + cur.height / 2;

    monitors
        .iter()
        .enumerate()
        .filter(|(i, m)| *i != current_idx && !excluded.contains(&m.name))
        .filter(|(_, m)| {
            let mx = m.x + m.width / 2;
            let my = m.y + m.height / 2;
            let dx = (mx - cx).abs();
            let dy = (my - cy).abs();
            // Require dominant axis: Left/Right needs dx >= dy, Up/Down needs dy >= dx.
            // This prevents stacked monitors being found via Left/Right (and vice versa).
            match direction {
                Direction::Left => mx < cx && dx >= dy,
                Direction::Right => mx > cx && dx >= dy,
                Direction::Up => my < cy && dy >= dx,
                Direction::Down => my > cy && dy >= dx,
            }
        })
        .min_by_key(|(_, m)| {
            let mx = m.x + m.width / 2;
            let my = m.y + m.height / 2;
            (cx - mx).abs() + (cy - my).abs()
        })
        .map(|(i, _)| i)
}

/// Scan all windows on startup and populate the zone tracker + last_action
/// based on geometry matching. This lets Pave "know" about existing tiled windows
/// without requiring them to be re-snapped.
pub async fn scan_existing_windows(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
) -> Result<(), String> {
    let windows = wm.get_windows().await?;
    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Ok(());
    }

    let gap = config.gap_size;
    let mut matched = 0;

    for window in &windows {
        let mon_idx = find_window_monitor(window, &monitors);
        let monitor = &monitors[mon_idx];
        let wrect = window_to_rect(window);

        // Build all candidate zones and their geometries for this monitor
        let candidates: Vec<(&str, Rect)> = vec![
            ("almost_maximize", almost_maximize_rect(monitor, gap)),
            ("full_maximize", almost_maximize_rect(monitor, 1)),
            ("snap_left", snap_left_rect(monitor, gap)),
            ("snap_right", snap_right_rect(monitor, gap)),
            ("snap_left_two_thirds", snap_left_two_thirds_rect(monitor, gap)),
            ("snap_left_one_third", snap_left_one_third_rect(monitor, gap)),
            ("snap_right_two_thirds", snap_right_two_thirds_rect(monitor, gap)),
            ("snap_right_one_third", snap_right_one_third_rect(monitor, gap)),
            ("snap_top_left", snap_top_left_rect(monitor, gap)),
            ("snap_top_right", snap_top_right_rect(monitor, gap)),
            ("snap_bottom_left", snap_bottom_left_rect(monitor, gap)),
            ("snap_bottom_right", snap_bottom_right_rect(monitor, gap)),
        ];

        // Also check width-preserving quarters (current width + half height)
        let half_h = monitor.height / 2;
        let g = gap as i32;
        let top_preserve = Rect {
            x: wrect.x,
            y: monitor.y + g,
            width: wrect.width,
            height: half_h - g - g / 2,
        };
        let bottom_preserve = Rect {
            x: wrect.x,
            y: monitor.y + half_h + g / 2,
            width: wrect.width,
            height: monitor.height - half_h - g - g / 2,
        };

        // Try standard zone geometries first
        if let Some((action, rect)) = candidates.iter().find(|(_, r)| rects_approx_equal(&wrect, r)) {
            log::info!(
                "Startup scan: window '{}' ({}) matches zone {} on monitor {}",
                window.title, if window.minimized { "minimized" } else { "visible" },
                action, monitor.name
            );
            state.set_last_action(&window.id, action, mon_idx);
            state.zone_place_silent(mon_idx, action, &window.id, *rect);
            matched += 1;
        } else if rects_approx_equal(&wrect, &top_preserve) {
            // Width-preserving top quarter — determine side from x position
            let action = if wrect.x < monitor.x + monitor.width / 2 {
                "snap_top_left"
            } else {
                "snap_top_right"
            };
            log::info!(
                "Startup scan: window '{}' ({}) matches {} (width-preserving) on monitor {}",
                window.title, if window.minimized { "minimized" } else { "visible" },
                action, monitor.name
            );
            state.set_last_action(&window.id, action, mon_idx);
            state.zone_place_silent(mon_idx, action, &window.id, wrect);
            matched += 1;
        } else if rects_approx_equal(&wrect, &bottom_preserve) {
            let action = if wrect.x < monitor.x + monitor.width / 2 {
                "snap_bottom_left"
            } else {
                "snap_bottom_right"
            };
            log::info!(
                "Startup scan: window '{}' ({}) matches {} (width-preserving) on monitor {}",
                window.title, if window.minimized { "minimized" } else { "visible" },
                action, monitor.name
            );
            state.set_last_action(&window.id, action, mon_idx);
            state.zone_place_silent(mon_idx, action, &window.id, wrect);
            matched += 1;
        }
    }

    log::info!("Startup scan complete: {matched}/{} windows matched zones", windows.len());
    Ok(())
}

/// After a snap/maximize, register the window in its zone.
/// Immediately minimizes displaced windows and returns an entry to surface from a vacated zone.
async fn auto_tab_after_snap(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
    window_id: &str,
    action: &str,
    monitor_idx: usize,
    geometry: Rect,
) -> Result<(), String> {
    let (displaced, surface) = state.zone_place(monitor_idx, action, window_id, geometry);

    for displaced_id in &displaced {
        log::info!("Auto-tab: minimizing displaced window {displaced_id}");
        if let Err(e) = wm.minimize_window(displaced_id).await {
            log::error!("Failed to minimize displaced window: {e}");
        }
    }

    if config.auto_surface_tabs {
        for entry in &surface {
            surface_zone_entry(wm, state, entry).await?;
        }
    }

    Ok(())
}

/// Surface a window from a vacated zone (unminimize + move to stored geometry).
/// Also minimizes any windows in parent zones that would cover this window.
async fn surface_zone_entry(
    wm: &KWinBackend,
    state: &TilingState,
    entry: &ZoneEntry,
) -> Result<(), String> {
    log::info!("Auto-surface: restoring {} from vacated zone", entry.window_id);

    // Minimize any parent zone windows that cover this window's zone
    let overlapping = state.zone_get_overlapping(&entry.window_id);
    for id in &overlapping {
        log::info!("Auto-surface: minimizing overlapping parent window {id}");
        if let Err(e) = wm.minimize_window(id).await {
            log::error!("Failed to minimize overlapping window: {e}");
        }
    }

    wm.unminimize_window(&entry.window_id).await?;
    wm.move_window(
        &entry.window_id,
        entry.geometry.x,
        entry.geometry.y,
        entry.geometry.width,
        entry.geometry.height,
    )
    .await?;
    Ok(())
}

// --- Snap spectrum: bidirectional size cycling ---

/// Ordered snap spectrum: leftmost to rightmost
const SNAP_SPECTRUM: &[&str] = &[
    "snap_left_one_third",    // 0
    "snap_left",              // 1
    "snap_left_two_thirds",   // 2
    "almost_maximize",        // 3
    "snap_right_two_thirds",  // 4
    "snap_right",             // 5
    "snap_right_one_third",   // 6
];

fn spectrum_index(action: &str) -> Option<usize> {
    SNAP_SPECTRUM.iter().position(|&a| a == action)
}

fn spectrum_rect(index: usize, monitor: &MonitorInfo, gap: u32) -> Rect {
    match SNAP_SPECTRUM[index] {
        "snap_left_one_third" => snap_left_one_third_rect(monitor, gap),
        "snap_left" => snap_left_rect(monitor, gap),
        "snap_left_two_thirds" => snap_left_two_thirds_rect(monitor, gap),
        "almost_maximize" => almost_maximize_rect(monitor, gap),
        "snap_right_two_thirds" => snap_right_two_thirds_rect(monitor, gap),
        "snap_right" => snap_right_rect(monitor, gap),
        "snap_right_one_third" => snap_right_one_third_rect(monitor, gap),
        _ => unreachable!(),
    }
}

/// Determine current spectrum index from geometry match, then last_action fallback.
fn find_current_spectrum_index(
    current_rect: &Rect,
    last_action_name: &Option<String>,
    mon_idx: usize,
    state: &TilingState,
    window: &WindowInfo,
    monitor: &MonitorInfo,
    gap: u32,
) -> Option<usize> {
    // Try geometry match first
    for (i, _) in SNAP_SPECTRUM.iter().enumerate() {
        let rect = spectrum_rect(i, monitor, gap);
        if rects_approx_equal(current_rect, &rect) {
            return Some(i);
        }
    }

    // Fall back to last_action (handles smart-space positions that don't match standard geometry)
    let on_same_monitor = state
        .get_last_action(&window.id)
        .map(|(_, m)| m == mon_idx)
        .unwrap_or(false);
    if on_same_monitor {
        if let Some(action) = last_action_name.as_deref() {
            return spectrum_index(action);
        }
    }

    None
}

/// Determine spectrum index for a quarter by matching x and width only (ignoring y/height).
fn find_quarter_spectrum_index(
    current_rect: &Rect,
    monitor: &MonitorInfo,
    gap: u32,
) -> Option<usize> {
    for i in 0..SNAP_SPECTRUM.len() {
        let rect = spectrum_rect(i, monitor, gap);
        if (current_rect.x - rect.x).abs() <= 15
            && (current_rect.width - rect.width).abs() <= 15
        {
            return Some(i);
        }
    }
    None
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
        auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
    } else if was_full_maximized || rects_approx_equal(&current_rect, &full_rect) {
        // Full-size -> almost-maximize
        log::info!("Action: full-size to almost-maximize");
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
    } else if was_almost_maximized || rects_approx_equal(&current_rect, &almost_rect) {
        // Almost-maximized -> full-size (just geometry, no KWin maximize)
        log::info!("Action: almost-maximize to full-size");
        wm.move_window(&window.id, full_rect.x, full_rect.y, full_rect.width, full_rect.height)
            .await?;
        state.set_last_action(&window.id, "full_maximize", mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, "full_maximize", mon_idx, full_rect).await?;
    } else {
        // Neither -> almost-maximize
        log::info!("Action: almost-maximize");
        state.save_pre_snap_geometry(&window.id, current_rect);
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
    }

    Ok(())
}

/// Handle snap left/right action using the bidirectional snap spectrum.
/// Left = step leftward through spectrum, Right = step rightward.
/// At spectrum edges, crosses to adjacent monitor if one exists in that direction.
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
    let current_rect = window_to_rect(&window);
    let gap = config.gap_size;

    // If currently in a quarter, left/right escapes to full half snap on that side
    let last_action_name = state
        .get_last_action(&window.id)
        .map(|(a, _)| a);
    let is_quarter = matches!(
        last_action_name.as_deref(),
        Some("snap_top_left") | Some("snap_top_right")
        | Some("snap_bottom_left") | Some("snap_bottom_right")
    );

    if is_quarter {
        let is_top = matches!(
            last_action_name.as_deref(),
            Some("snap_top_left") | Some("snap_top_right")
        );

        // Try to find current width in the spectrum
        let quarter_idx = find_quarter_spectrum_index(&current_rect, monitor, gap)
            .or_else(|| {
                // Fallback: if geometry doesn't match, infer from action name side
                let is_left_quarter = matches!(
                    last_action_name.as_deref(),
                    Some("snap_top_left") | Some("snap_bottom_left")
                );
                Some(if is_left_quarter { 1 } else { 5 }) // default to 1/2 width
            });

        if let Some(idx) = quarter_idx {
            let new_idx_i32 = match side {
                SnapSide::Right => idx as i32 + 1,
                SnapSide::Left => idx as i32 - 1,
            };

            if new_idx_i32 < 0 || new_idx_i32 >= SNAP_SPECTRUM.len() as i32 {
                // Edge of spectrum — try monitor crossing (as quarter)
                let direction = match side {
                    SnapSide::Right => Direction::Right,
                    SnapSide::Left => Direction::Left,
                };
                if let Some(next_mon_idx) = find_monitor_in_direction(
                    mon_idx, &monitors, &config.excluded_monitors, direction,
                ) {
                    let entry_idx = match side {
                        SnapSide::Right => 0,
                        SnapSide::Left => SNAP_SPECTRUM.len() - 1,
                    };
                    let next_monitor = &monitors[next_mon_idx];
                    let spectrum_r = spectrum_rect(entry_idx, next_monitor, gap);
                    let g = gap as i32;
                    let half_h = next_monitor.height / 2;
                    let quarter_rect = if is_top {
                        Rect { x: spectrum_r.x, y: next_monitor.y + g, width: spectrum_r.width, height: half_h - g - g / 2 }
                    } else {
                        Rect { x: spectrum_r.x, y: next_monitor.y + half_h + g / 2, width: spectrum_r.width, height: next_monitor.height - half_h - g - g / 2 }
                    };
                    let new_is_left = entry_idx < 3;
                    let action = match (is_top, new_is_left) {
                        (true, true) => "snap_top_left",
                        (true, false) => "snap_top_right",
                        (false, true) => "snap_bottom_left",
                        (false, false) => "snap_bottom_right",
                    };
                    wm.move_window(&window.id, quarter_rect.x, quarter_rect.y, quarter_rect.width, quarter_rect.height).await?;
                    state.set_last_action(&window.id, action, next_mon_idx);
                    auto_tab_after_snap(wm, config, state, &window.id, action, next_mon_idx, quarter_rect).await?;
                }
                // else: no monitor in that direction, no-op
            } else {
                // Stay in quarter mode, adjust width
                let new_idx = new_idx_i32 as usize;
                let spectrum_r = spectrum_rect(new_idx, monitor, gap);
                let g = gap as i32;
                let half_h = monitor.height / 2;
                let quarter_rect = if is_top {
                    Rect { x: spectrum_r.x, y: monitor.y + g, width: spectrum_r.width, height: half_h - g - g / 2 }
                } else {
                    Rect { x: spectrum_r.x, y: monitor.y + half_h + g / 2, width: spectrum_r.width, height: monitor.height - half_h - g - g / 2 }
                };
                let new_is_left = new_idx < 3;
                let action = match (is_top, new_is_left) {
                    (true, true) => "snap_top_left",
                    (true, false) => "snap_top_right",
                    (false, true) => "snap_bottom_left",
                    (false, false) => "snap_bottom_right",
                };
                wm.move_window(&window.id, quarter_rect.x, quarter_rect.y, quarter_rect.width, quarter_rect.height).await?;
                state.set_last_action(&window.id, action, mon_idx);
                auto_tab_after_snap(wm, config, state, &window.id, action, mon_idx, quarter_rect).await?;
            }
        }

        return Ok(());
    }

    // Try to find current position in spectrum
    let current_index = find_current_spectrum_index(
        &current_rect, &last_action_name, mon_idx, state, &window, monitor, gap,
    );

    match current_index {
        Some(idx) => {
            // Step in the pressed direction
            let new_idx = match side {
                SnapSide::Right => idx as i32 + 1,
                SnapSide::Left => idx as i32 - 1,
            };

            if new_idx >= 0 && new_idx < SNAP_SPECTRUM.len() as i32 {
                // Move within current monitor
                let new_idx = new_idx as usize;
                let action = SNAP_SPECTRUM[new_idx];
                let rect = spectrum_rect(new_idx, monitor, gap);
                wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height).await?;
                state.set_last_action(&window.id, action, mon_idx);
                auto_tab_after_snap(wm, config, state, &window.id, action, mon_idx, rect).await?;
            } else {
                // Edge of spectrum — try crossing to adjacent monitor
                let direction = match side {
                    SnapSide::Right => Direction::Right,
                    SnapSide::Left => Direction::Left,
                };
                if let Some(next_mon_idx) = find_monitor_in_direction(
                    mon_idx, &monitors, &config.excluded_monitors, direction,
                ) {
                    // Enter the opposite end of the spectrum on the new monitor
                    let entry_idx = match side {
                        SnapSide::Right => 0,  // Enter at L1/3
                        SnapSide::Left => SNAP_SPECTRUM.len() - 1,  // Enter at R1/3
                    };
                    let next_monitor = &monitors[next_mon_idx];
                    let action = SNAP_SPECTRUM[entry_idx];
                    let rect = spectrum_rect(entry_idx, next_monitor, gap);
                    wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height).await?;
                    state.set_last_action(&window.id, action, next_mon_idx);
                    auto_tab_after_snap(wm, config, state, &window.id, action, next_mon_idx, rect).await?;
                }
                // else: no monitor in that direction, no-op
            }
        }
        None => {
            // Fresh snap — not currently in any spectrum position
            state.save_pre_snap_geometry(&window.id, current_rect);

            let action_name = match side {
                SnapSide::Left => "snap_left",
                SnapSide::Right => "snap_right",
            };

            // Try smart space first, fall back to standard half
            let final_rect = if let Some(smart_rect) = find_empty_space(
                side, monitor, &windows, &window.id, gap,
            ) {
                wm.move_window(&window.id, smart_rect.x, smart_rect.y, smart_rect.width, smart_rect.height).await?;
                smart_rect
            } else {
                let rect = match side {
                    SnapSide::Left => snap_left_rect(monitor, gap),
                    SnapSide::Right => snap_right_rect(monitor, gap),
                };
                wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height).await?;
                rect
            };

            state.set_last_action(&window.id, action_name, mon_idx);
            auto_tab_after_snap(wm, config, state, &window.id, action_name, mon_idx, final_rect).await?;
        }
    }

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

    // Determine side context from last action, falling back to zone tracker
    let side_context = last_action.as_deref().and_then(|a| {
        if a.starts_with("snap_left") || a.starts_with("snap_top_left") || a.starts_with("snap_bottom_left") {
            Some("left")
        } else if a.starts_with("snap_right") || a.starts_with("snap_top_right") || a.starts_with("snap_bottom_right") {
            Some("right")
        } else {
            None
        }
    }).or_else(|| state.zone_side_context(&window.id));

    let side_context = match side_context {
        Some(s) => s,
        None => {
            log::debug!("Snap vertical: no snap context, ignoring");
            return Ok(());
        }
    };

    // Check if already in the target quarter — repeat press crosses monitors
    let is_already_top = matches!(
        last_action.as_deref(),
        Some("snap_top_left") | Some("snap_top_right")
    );
    let is_already_bottom = matches!(
        last_action.as_deref(),
        Some("snap_bottom_left") | Some("snap_bottom_right")
    );

    if (direction == SnapVertical::Up && is_already_top)
        || (direction == SnapVertical::Down && is_already_bottom)
    {
        let dir = match direction {
            SnapVertical::Up => Direction::Up,
            SnapVertical::Down => Direction::Down,
        };
        if let Some(next_mon_idx) = find_monitor_in_direction(
            mon_idx, &monitors, &config.excluded_monitors, dir,
        ) {
            // Snap to the opposite vertical position on the new monitor
            let next_monitor = &monitors[next_mon_idx];
            let target_action = match (side_context, direction) {
                ("left", SnapVertical::Up) => "snap_bottom_left",
                ("left", SnapVertical::Down) => "snap_top_left",
                ("right", SnapVertical::Up) => "snap_bottom_right",
                ("right", SnapVertical::Down) => "snap_top_right",
                _ => unreachable!(),
            };
            let target_rect = match target_action {
                "snap_top_left" => snap_top_left_rect(next_monitor, config.gap_size),
                "snap_top_right" => snap_top_right_rect(next_monitor, config.gap_size),
                "snap_bottom_left" => snap_bottom_left_rect(next_monitor, config.gap_size),
                "snap_bottom_right" => snap_bottom_right_rect(next_monitor, config.gap_size),
                _ => unreachable!(),
            };
            wm.move_window(&window.id, target_rect.x, target_rect.y, target_rect.width, target_rect.height).await?;
            state.set_last_action(&window.id, target_action, next_mon_idx);
            auto_tab_after_snap(wm, config, state, &window.id, target_action, next_mon_idx, target_rect).await?;
            return Ok(());
        }
        // No monitor in that direction — no-op
        return Ok(());
    }

    let gap = config.gap_size;
    let g = gap as i32;
    let current_rect = window_to_rect(&window);
    let half_h = monitor.height / 2;

    // Quarter pressing opposite direction → go to full height first, preserving width
    if (direction == SnapVertical::Down && is_already_top)
        || (direction == SnapVertical::Up && is_already_bottom)
    {
        // Determine full-height action from horizontal spectrum position
        let spectrum_idx = find_quarter_spectrum_index(&current_rect, monitor, gap);
        let (target_action, target_rect) = if let Some(idx) = spectrum_idx {
            (SNAP_SPECTRUM[idx], spectrum_rect(idx, monitor, gap))
        } else {
            // Fallback: standard half-width for the side
            match side_context {
                "left" => ("snap_left", snap_left_rect(monitor, gap)),
                _ => ("snap_right", snap_right_rect(monitor, gap)),
            }
        };

        log::info!(
            "Snap vertical: {} -> {} (quarter to full height) at ({},{} {}x{})",
            last_action.as_deref().unwrap_or("none"),
            target_action,
            target_rect.x, target_rect.y, target_rect.width, target_rect.height
        );

        wm.move_window(&window.id, target_rect.x, target_rect.y, target_rect.width, target_rect.height).await?;
        state.set_last_action(&window.id, target_action, mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, target_action, mon_idx, target_rect).await?;
        return Ok(());
    }

    // Full height → quarter (preserve width and x)
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
    auto_tab_after_snap(wm, config, state, &window.id, target_action, mon_idx, target_rect).await?;
    Ok(())
}

/// Restore a window to its pre-snap geometry
pub async fn handle_restore(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    // Remove from zone and surface next tabbed window if any
    let zone_result = state.zone_find_and_remove(&window.id);

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

            // Surface tabbed windows in the zone (and child zones)
            if let Some((_zone_id, entries)) = zone_result {
                if config.auto_surface_tabs {
                    for entry in &entries {
                        log::info!("Restore: surfacing tabbed window {}", entry.window_id);
                        surface_zone_entry(wm, state, entry).await?;
                    }
                }
            }

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

    // Remove from zone — grow/shrink means the window is no longer in a snap position
    state.zone_remove(&window.id);

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

/// Cycle to the next tabbed window group in the current zone.
///
/// Group cycling:
/// - Full-side window (Right) ↔ both quarter windows (TopRight + BottomRight)
/// - Same-zone stacking: cycles between individual windows
pub async fn handle_tab_cycle(
    wm: &KWinBackend,
    _config: &PaveConfig,
    state: &TilingState,
) -> Result<(), String> {
    let window = wm
        .get_active_window()
        .await?
        .ok_or("No active window")?;

    // Cleanup stale zone entries
    let windows = wm.get_windows().await?;
    let existing_ids: Vec<String> = windows.iter().map(|w| w.id.clone()).collect();
    state.zone_cleanup(&existing_ids);

    if let Some((to_show, to_hide)) = state.zone_cycle(&window.id) {
        log::info!(
            "Tab cycle: showing {} window(s), hiding {} window(s)",
            to_show.len(), to_hide.len()
        );

        // Hide first, then show
        for id in &to_hide {
            if let Err(e) = wm.minimize_window(id).await {
                log::error!("Tab cycle: failed to minimize {id}: {e}");
            }
        }

        let monitors = wm.get_monitors().await?;
        let mut last_shown_id = None;

        for entry in &to_show {
            wm.unminimize_window(&entry.window_id).await?;
            wm.move_window(
                &entry.window_id,
                entry.geometry.x,
                entry.geometry.y,
                entry.geometry.width,
                entry.geometry.height,
            )
            .await?;

            // Set last_action for each shown window
            let mon_idx = mon_idx_from_geometry(&entry.geometry, &monitors);
            state.set_last_action(&entry.window_id, &entry.snap_action, mon_idx);
            last_shown_id = Some(entry.window_id.clone());
        }

        // Activate the last shown window
        if let Some(id) = last_shown_id {
            wm.activate_window(&id).await?;
        }
    } else {
        // Fallback: surface any minimized window from any zone
        if let Some(entry) = state.zone_find_minimized(&windows) {
            log::info!("Tab cycle fallback: surfacing minimized window {}", entry.window_id);

            // Minimize parent zone windows that cover this window
            let overlapping = state.zone_get_overlapping(&entry.window_id);
            for id in &overlapping {
                log::info!("Tab cycle: minimizing overlapping parent window {id}");
                if let Err(e) = wm.minimize_window(id).await {
                    log::error!("Failed to minimize overlapping window: {e}");
                }
            }

            wm.unminimize_window(&entry.window_id).await?;
            wm.move_window(
                &entry.window_id,
                entry.geometry.x,
                entry.geometry.y,
                entry.geometry.width,
                entry.geometry.height,
            )
            .await?;
            wm.activate_window(&entry.window_id).await?;

            let monitors = wm.get_monitors().await?;
            let mon_idx = mon_idx_from_geometry(&entry.geometry, &monitors);
            state.set_last_action(&entry.window_id, &entry.snap_action, mon_idx);
        } else {
            log::debug!("Tab cycle: no windows to cycle or surface");
        }
    }

    Ok(())
}

/// Helper to find monitor index from a geometry rect
fn mon_idx_from_geometry(geometry: &Rect, monitors: &[MonitorInfo]) -> usize {
    let fake_win = WindowInfo {
        id: String::new(),
        title: String::new(),
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        maximized: false,
        minimized: false,
        resource_class: String::new(),
        active: false,
        desktop: -1,
        screen: String::new(),
    };
    find_window_monitor(&fake_win, monitors)
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
