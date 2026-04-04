use crate::config::PaveConfig;
use crate::platform::kwin::KWinBackend;
use crate::platform::{MonitorInfo, WindowInfo};
use crate::zone_assignments::ZoneAssignments;
use crate::zone_layout::{AdjacentDirection, ZoneLayout, ZoneLeafId};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZoneId {
    monitor_idx: usize,
    leaf_id: ZoneLeafId,
}

#[derive(Debug, Clone)]
pub struct ZoneEntry {
    pub window_id: String,
    pub snap_action: String,
    pub geometry: Rect,
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

        // Collect windows in overlapping zones (ancestors + descendants) on same monitor
        let overlapping_ids: Vec<ZoneId> = self
            .zones
            .keys()
            .filter(|k| {
                k.monitor_idx == zone_id.monitor_idx
                    && k.leaf_id != zone_id.leaf_id
                    && (zone_id.leaf_id.is_ancestor_of(&k.leaf_id)
                        || k.leaf_id.is_ancestor_of(&zone_id.leaf_id))
            })
            .cloned()
            .collect();

        for overlapping_zone in &overlapping_ids {
            if let Some(entries) = self.zones.get(overlapping_zone) {
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
    /// - If current window is in a parent zone and children exist:
    ///   show all children, hide parent
    /// - If current window is in a child zone and parent exists:
    ///   show parent, hide all children (siblings too)
    /// - If same-zone stacking (multiple windows in exact same zone): cycle within zone
    fn cycle_next(&self, current_window_id: &str) -> Option<(Vec<ZoneEntry>, Vec<String>)> {
        let zone_id = self.find_zone(current_window_id)?;

        // Case 1: Current is in a parent zone, children exist → show children, hide parent
        let child_zones: Vec<ZoneId> = self
            .zones
            .keys()
            .filter(|k| {
                k.monitor_idx == zone_id.monitor_idx
                    && zone_id.leaf_id.is_ancestor_of(&k.leaf_id)
            })
            .cloned()
            .collect();

        if !child_zones.is_empty() {
            let mut child_entries = Vec::new();
            for child_zone in &child_zones {
                if let Some(entries) = self.zones.get(child_zone) {
                    if let Some(entry) = entries.last() {
                        child_entries.push(entry.clone());
                    }
                }
            }
            if !child_entries.is_empty() {
                return Some((child_entries, vec![current_window_id.to_string()]));
            }
        }

        // Case 2: Current is in a child zone, immediate parent exists → show parent, hide all siblings
        if let Some(parent_leaf_id) = zone_id.leaf_id.immediate_parent() {
            let parent_zone = ZoneId {
                monitor_idx: zone_id.monitor_idx,
                leaf_id: parent_leaf_id.clone(),
            };
            if let Some(parent_entries) = self.zones.get(&parent_zone) {
                if let Some(parent_entry) = parent_entries.last() {
                    // Collect all descendant window IDs to hide
                    let mut to_hide = Vec::new();
                    for (k, entries) in &self.zones {
                        if k.monitor_idx == zone_id.monitor_idx
                            && parent_leaf_id.is_ancestor_of(&k.leaf_id)
                        {
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
    /// Includes same-zone entries, entries in descendant zones, AND entries in
    /// parent zones (if no sibling descendant still occupies the parent).
    fn collect_surface_entries(&self, zone_id: &ZoneId) -> Vec<ZoneEntry> {
        let mut entries = Vec::new();

        // Surface from the same zone
        if let Some(zone_entries) = self.zones.get(zone_id) {
            if let Some(entry) = zone_entries.last() {
                entries.push(entry.clone());
            }
        }

        // Surface from descendant zones
        for (k, zone_entries) in &self.zones {
            if k.monitor_idx == zone_id.monitor_idx
                && zone_id.leaf_id.is_ancestor_of(&k.leaf_id)
            {
                if let Some(entry) = zone_entries.last() {
                    entries.push(entry.clone());
                }
            }
        }

        // Surface from parent zone, but only if no sibling descendant still occupies it.
        if let Some(parent_leaf_id) = zone_id.leaf_id.immediate_parent() {
            let siblings_occupied = self.zones.iter().any(|(k, e)| {
                k.monitor_idx == zone_id.monitor_idx
                    && parent_leaf_id.is_ancestor_of(&k.leaf_id)
                    && k.leaf_id != zone_id.leaf_id
                    && !e.is_empty()
            });

            if !siblings_occupied {
                let parent_zone = ZoneId {
                    monitor_idx: zone_id.monitor_idx,
                    leaf_id: parent_leaf_id,
                };
                if let Some(zone_entries) = self.zones.get(&parent_zone) {
                    if let Some(entry) = zone_entries.last() {
                        entries.push(entry.clone());
                    }
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

    /// Get the top (most recent) entry from every occupied zone.
    fn get_all_zone_tops(&self) -> Vec<ZoneEntry> {
        self.zones.values()
            .filter_map(|entries| entries.last().cloned())
            .collect()
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
    /// Last known geometry for a zone, saved when a zone becomes empty via zone snap.
    /// Allows windows to return to the correct size when zone-snapping back.
    zone_last_geometry: Mutex<HashMap<ZoneId, Rect>>,
    /// Tiled geometry and action saved when entering the maximize cycle (for returning to tile)
    pre_maximize_geometry: Mutex<HashMap<String, (Rect, String)>>,
    /// Zone tracker for tab zone system
    zone_tracker: Mutex<ZoneTracker>,
    /// Zone layouts per monitor (keyed by monitor name). Default: 50/50 two-column.
    zone_layouts: Mutex<HashMap<String, ZoneLayout>>,
    /// Saved layout before maximize (for tile restore).
    pre_maximize_layout: Mutex<HashMap<String, ZoneLayout>>,
    /// Persistent window class → zone assignments for auto-placement.
    zone_assignments: Mutex<ZoneAssignments>,
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
pub enum Direction {
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
            pre_maximize_geometry: Mutex::new(HashMap::new()),
            zone_tracker: Mutex::new(ZoneTracker::new()),
            zone_last_geometry: Mutex::new(HashMap::new()),
            zone_layouts: Mutex::new(HashMap::new()),
            pre_maximize_layout: Mutex::new(HashMap::new()),
            zone_assignments: Mutex::new(ZoneAssignments::load()),
        }
    }

    /// Save the current layout before maximize, so we can restore it.
    fn save_pre_maximize_layout(&self, monitor_name: &str) {
        let layouts = self.zone_layouts.lock().unwrap();
        if let Some(layout) = layouts.get(monitor_name) {
            let mut saved = self.pre_maximize_layout.lock().unwrap();
            saved.insert(monitor_name.to_string(), layout.clone());
        }
    }

    /// Restore the layout saved before maximize.
    fn take_pre_maximize_layout(&self, monitor_name: &str) -> Option<ZoneLayout> {
        let mut saved = self.pre_maximize_layout.lock().unwrap();
        saved.remove(monitor_name)
    }

    /// Record that a window class was tiled into a zone.
    pub fn record_zone_assignment(&self, resource_class: &str, action: &str) {
        if let Some(leaf_id) = ZoneLeafId::from_action(action) {
            let mut assignments = self.zone_assignments.lock().unwrap();
            assignments.set(resource_class, &leaf_id);
        }
    }

    /// Look up the remembered zone for a window class.
    pub fn get_zone_assignment(&self, resource_class: &str) -> Option<ZoneLeafId> {
        let assignments = self.zone_assignments.lock().unwrap();
        assignments.get(resource_class)
    }

    /// Get the zone layout for a monitor, creating a default 50/50 two-column if missing.
    pub fn get_or_create_layout(&self, monitor_name: &str) -> ZoneLayout {
        let mut layouts = self.zone_layouts.lock().unwrap();
        layouts
            .entry(monitor_name.to_string())
            .or_insert_with(|| ZoneLayout::two_column(0.5))
            .clone()
    }

    /// Update the zone layout for a monitor.
    pub fn set_layout(&self, monitor_name: &str, layout: ZoneLayout) {
        let mut layouts = self.zone_layouts.lock().unwrap();
        layouts.insert(monitor_name.to_string(), layout);
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

    pub fn clear_last_action(&self, window_id: &str) {
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

    /// Save geometry and action at the moment the window enters the maximize cycle (for tile restore).
    fn save_pre_maximize_geometry(&self, window_id: &str, rect: Rect, action: &str) {
        let mut geo = self.pre_maximize_geometry.lock().unwrap();
        geo.insert(window_id.to_string(), (rect, action.to_string()));
    }

    /// Remove and return the pre-maximize geometry and action (tile restore on third press).
    fn take_pre_maximize_geometry(&self, window_id: &str) -> Option<(Rect, String)> {
        let mut geo = self.pre_maximize_geometry.lock().unwrap();
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
        let leaf_id = match ZoneLeafId::from_action(action) {
            Some(s) => s,
            None => return (Vec::new(), Vec::new()),
        };
        let zone_id = ZoneId { monitor_idx, leaf_id };

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
        let leaf_id = match ZoneLeafId::from_action(action) {
            Some(s) => s,
            None => return,
        };
        let zone_id = ZoneId { monitor_idx, leaf_id };
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
    pub fn zone_find_and_remove(&self, window_id: &str) -> Option<(ZoneId, Vec<ZoneEntry>)> {
        let mut tracker = self.zone_tracker.lock().unwrap();
        tracker.find_and_remove(window_id)
    }

    /// Find a minimized window in any zone (fallback for tab cycle recovery).
    fn zone_find_minimized(&self, windows: &[WindowInfo]) -> Option<ZoneEntry> {
        let tracker = self.zone_tracker.lock().unwrap();
        tracker.find_minimized_entry(windows)
    }

    /// Get window IDs from zones that cover this window's zone (ancestors).
    /// Used to minimize overlapping windows when a window is surfaced.
    fn zone_get_overlapping(&self, window_id: &str) -> Vec<String> {
        let tracker = self.zone_tracker.lock().unwrap();
        let zone_id = match tracker.find_zone(window_id) {
            Some(z) => z.clone(),
            None => return Vec::new(),
        };

        let mut to_minimize = Vec::new();

        // Check ancestor zones (e.g. surfacing R.T should minimize R)
        for (k, entries) in &tracker.zones {
            if k.monitor_idx == zone_id.monitor_idx
                && k.leaf_id.is_ancestor_of(&zone_id.leaf_id)
            {
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
        zone_id.leaf_id.side_context()
    }

    /// Get the top entry from every occupied zone (for resurface).
    pub fn zone_get_all_tops(&self) -> Vec<ZoneEntry> {
        let tracker = self.zone_tracker.lock().unwrap();
        tracker.get_all_zone_tops()
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

        // Build candidate zones from all ratio steps + quarters
        let mut candidates: Vec<(&str, Rect)> = vec![
            ("almost_maximize", almost_maximize_rect(monitor, gap)),
            ("full_maximize", almost_maximize_rect(monitor, 1)),
        ];
        // Add candidates for each ratio step
        for &ratio in RATIO_STEPS {
            let layout = ZoneLayout::two_column(ratio);
            let rects = layout.compute_rects(monitor, gap);
            let left_rect = rects[&ZoneLeafId("L".to_string())];
            let right_rect = rects[&ZoneLeafId("R".to_string())];

            candidates.push(("snap_left", left_rect));
            candidates.push(("snap_right", right_rect));

            // Quarter variants
            let g = gap as i32;
            let half_h = monitor.height / 2;
            candidates.push(("snap_top_left", Rect {
                x: left_rect.x, y: monitor.y + g,
                width: left_rect.width, height: half_h - g - g / 2,
            }));
            candidates.push(("snap_bottom_left", Rect {
                x: left_rect.x, y: monitor.y + half_h + g / 2,
                width: left_rect.width, height: monitor.height - half_h - g - g / 2,
            }));
            candidates.push(("snap_top_right", Rect {
                x: right_rect.x, y: monitor.y + g,
                width: right_rect.width, height: half_h - g - g / 2,
            }));
            candidates.push(("snap_bottom_right", Rect {
                x: right_rect.x, y: monitor.y + half_h + g / 2,
                width: right_rect.width, height: monitor.height - half_h - g - g / 2,
            }));
        }

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

    // Infer layout ratios from detected window positions per monitor
    for (mon_idx, monitor) in monitors.iter().enumerate() {
        // Check if any window matched a non-0.5 ratio on this monitor
        let actions: Vec<String> = windows
            .iter()
            .filter_map(|w| {
                let (action, idx) = state.get_last_action(&w.id)?;
                if idx == mon_idx {
                    Some(action)
                } else {
                    None
                }
            })
            .collect();

        let has_halves = actions.iter().any(|a| a.contains("left") || a.contains("right"));
        let has_maximize = actions.iter().any(|a| a == "almost_maximize" || a == "full_maximize");

        if has_halves {
            // Try to detect the ratio from left-side window width
            let left_windows: Vec<&WindowInfo> = windows
                .iter()
                .filter(|w| {
                    state
                        .get_last_action(&w.id)
                        .map(|(a, i)| i == mon_idx && a.contains("left"))
                        .unwrap_or(false)
                })
                .collect();
            if let Some(lw) = left_windows.first() {
                let boundary = (lw.x + lw.width - monitor.x) as f64 / monitor.width as f64;
                let ratio = RATIO_STEPS
                    .iter()
                    .min_by(|a, b| {
                        ((**a) - boundary)
                            .abs()
                            .partial_cmp(&((**b) - boundary).abs())
                            .unwrap()
                    })
                    .copied()
                    .unwrap_or(0.5);
                state.set_layout(&monitor.name, ZoneLayout::two_column(ratio));
                log::info!(
                    "Startup scan: inferred layout ratio {:.3} for monitor '{}'",
                    ratio, monitor.name
                );
            }
        } else if has_maximize {
            state.set_layout(&monitor.name, ZoneLayout::single());
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
            surface_zone_entry(wm, state, entry, config.gap_size).await?;
        }
    }

    Ok(())
}

/// Surface a window from a vacated zone (unminimize + move to stored geometry).
/// Also minimizes any windows in parent zones that would cover this window.
pub async fn surface_zone_entry(
    wm: &KWinBackend,
    state: &TilingState,
    entry: &ZoneEntry,
    gap: u32,
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

    // Compute the current zone rect from the layout tree instead of using stale geometry.
    let current_rect = if let Some(leaf_id) = ZoneLeafId::from_action(&entry.snap_action) {
        let mon_idx = state.get_last_action(&entry.window_id).map(|(_, i)| i).unwrap_or(0);
        let monitors = wm.get_monitors().await.unwrap_or_default();
        if let Some(monitor) = monitors.get(mon_idx) {
            let layout = state.get_or_create_layout(&monitor.name);
            let rects = layout.compute_rects(monitor, gap);
            if let Some(rect) = rects.get(&leaf_id) {
                Some(*rect)
            } else {
                // Zone has been split — compute bounding rect of descendant leaves
                let children: Vec<&Rect> = rects.iter()
                    .filter(|(k, _)| leaf_id.is_ancestor_of(k))
                    .map(|(_, v)| v)
                    .collect();
                if !children.is_empty() {
                    let mut x_min = i32::MAX;
                    let mut y_min = i32::MAX;
                    let mut x_max = i32::MIN;
                    let mut y_max = i32::MIN;
                    for r in &children {
                        x_min = x_min.min(r.x);
                        y_min = y_min.min(r.y);
                        x_max = x_max.max(r.x + r.width);
                        y_max = y_max.max(r.y + r.height);
                    }
                    Some(Rect { x: x_min, y: y_min, width: x_max - x_min, height: y_max - y_min })
                } else {
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let rect = current_rect.unwrap_or(entry.geometry);

    wm.unminimize_window(&entry.window_id).await?;
    wm.move_window(
        &entry.window_id,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
    )
    .await?;
    Ok(())
}

/// Resurface the top window in every occupied zone.
/// Unminimizes and repositions each zone's most recent window.
pub async fn resurface_all_zones(
    wm: &KWinBackend,
    state: &TilingState,
) -> Result<(), String> {
    let tops = state.zone_get_all_tops();
    log::info!("Resurfacing {} zone(s)", tops.len());
    for entry in &tops {
        if let Err(e) = wm.unminimize_window(&entry.window_id).await {
            log::error!("Resurface: failed to unminimize {}: {e}", entry.window_id);
            continue;
        }
        if let Err(e) = wm.move_window(
            &entry.window_id,
            entry.geometry.x,
            entry.geometry.y,
            entry.geometry.width,
            entry.geometry.height,
        ).await {
            log::error!("Resurface: failed to move {}: {e}", entry.window_id);
        }
    }
    Ok(())
}

// --- Layout-based ratio stepping ---

/// Preset ratios for the vertical split boundary.
/// Maps to old spectrum: [L1/3, L1/2, L2/3] = [R2/3, R1/2, R1/3].
const RATIO_STEPS: &[f64] = &[1.0 / 3.0, 0.5, 2.0 / 3.0];

/// Find the closest ratio step index for a given ratio.
fn find_ratio_index(ratio: f64) -> Option<usize> {
    RATIO_STEPS
        .iter()
        .position(|&r| (r - ratio).abs() < 0.05)
}

/// Step the ratio up (increase) or down (decrease) through RATIO_STEPS.
/// Returns None if at the boundary.
fn step_ratio(current: f64, up: bool) -> Option<f64> {
    let idx = find_ratio_index(current).unwrap_or(1);
    if up {
        RATIO_STEPS.get(idx + 1).copied()
    } else {
        idx.checked_sub(1).and_then(|i| RATIO_STEPS.get(i).copied())
    }
}

/// Get the root vertical split ratio from a layout, if it has one.
fn get_root_v_ratio(layout: &ZoneLayout) -> Option<f64> {
    match &layout.root {
        ZoneNode::Split { split, .. }
            if split.axis == crate::zone_layout::SplitAxis::Vertical =>
        {
            Some(split.ratio)
        }
        _ => None,
    }
}

use crate::zone_layout::ZoneNode;

/// Detect which side (if any) has a horizontal split in a two-column layout.
/// Returns ("left", h_ratio), ("right", h_ratio), or None.
fn get_horizontal_split_side(layout: &ZoneLayout) -> Option<(&'static str, f64)> {
    match &layout.root {
        ZoneNode::Split { split, first, second, .. }
            if split.axis == crate::zone_layout::SplitAxis::Vertical =>
        {
            if let ZoneNode::Split { split: h_split, .. } = first.as_ref() {
                if h_split.axis == crate::zone_layout::SplitAxis::Horizontal {
                    return Some(("left", h_split.ratio));
                }
            }
            if let ZoneNode::Split { split: h_split, .. } = second.as_ref() {
                if h_split.axis == crate::zone_layout::SplitAxis::Horizontal {
                    return Some(("right", h_split.ratio));
                }
            }
            None
        }
        _ => None,
    }
}

/// Build a layout with the given vertical ratio, preserving any existing horizontal split.
fn build_layout_preserving_splits(new_v_ratio: f64, old_layout: &ZoneLayout) -> ZoneLayout {
    match get_horizontal_split_side(old_layout) {
        Some(("left", h_ratio)) => ZoneLayout::left_split_and_right(new_v_ratio, h_ratio),
        Some(("right", h_ratio)) => ZoneLayout::left_and_right_split(new_v_ratio, h_ratio),
        _ => ZoneLayout::two_column(new_v_ratio),
    }
}

/// Reconcile the layout tree with actual zone occupancy.
/// If the layout has a horizontal split but no windows occupy the quarter zones,
/// merge the split back to a simple leaf. This keeps the layout in sync when
/// windows are zone-snapped or surfaced out of quarter positions.
/// Compute the current zone rect for a snap action using the layout tree.
/// Looks up the monitor from the action's last known monitor index, then
/// resolves the leaf from the layout. Returns None if it can't resolve.
fn compute_zone_rect_for_entry(
    state: &TilingState,
    snap_action: &str,
    monitors: &[MonitorInfo],
    gap: u32,
) -> Option<Rect> {
    let leaf_id = ZoneLeafId::from_action(snap_action)?;

    // Try each monitor's layout to find the leaf
    for monitor in monitors {
        let layout = state.get_or_create_layout(&monitor.name);
        let rects = layout.compute_rects(monitor, gap);
        if let Some(rect) = rects.get(&leaf_id) {
            return Some(*rect);
        }
        // If the zone has been split (e.g. "R" split to "R.T"/"R.B"),
        // compute the bounding rect of all descendant leaves.
        let children: Vec<&Rect> = rects.iter()
            .filter(|(k, _)| leaf_id.is_ancestor_of(k))
            .map(|(_, v)| v)
            .collect();
        if !children.is_empty() {
            let mut x_min = i32::MAX;
            let mut y_min = i32::MAX;
            let mut x_max = i32::MIN;
            let mut y_max = i32::MIN;
            for r in &children {
                x_min = x_min.min(r.x);
                y_min = y_min.min(r.y);
                x_max = x_max.max(r.x + r.width);
                y_max = y_max.max(r.y + r.height);
            }
            return Some(Rect {
                x: x_min,
                y: y_min,
                width: x_max - x_min,
                height: y_max - y_min,
            });
        }
    }

    // Fallback: use action_to_rect with first monitor
    monitors.first().map(|m| action_to_rect(snap_action, m, gap))
}

fn reconcile_layout(state: &TilingState, monitor_name: &str, mon_idx: usize) {
    let layout = state.get_or_create_layout(monitor_name);
    let h_split = get_horizontal_split_side(&layout);
    if h_split.is_none() {
        return; // No horizontal split to reconcile
    }
    let (split_side, _) = h_split.unwrap();

    // Check if any tracked window is in a quarter zone on that side
    let tracker = state.zone_tracker.lock().unwrap();
    let has_quarter_window = tracker.zones.iter().any(|(zone_id, entries)| {
        if zone_id.monitor_idx != mon_idx || entries.is_empty() {
            return false;
        }
        let leaf = &zone_id.leaf_id.0;
        match split_side {
            "left" => leaf == "L.T" || leaf == "L.B",
            "right" => leaf == "R.T" || leaf == "R.B",
            _ => false,
        }
    });
    drop(tracker);

    if !has_quarter_window {
        // Also check last_action for windows that might not be in the tracker yet
        let actions = state.last_action.lock().unwrap();
        let has_quarter_action = actions.values().any(|(action, idx, _)| {
            if *idx != mon_idx {
                return false;
            }
            match split_side {
                "left" => action == "snap_top_left" || action == "snap_bottom_left",
                "right" => action == "snap_top_right" || action == "snap_bottom_right",
                _ => false,
            }
        });

        if !has_quarter_action {
            let v_ratio = get_root_v_ratio(&layout).unwrap_or(0.5);
            log::info!(
                "Reconcile layout: removing stale {} horizontal split on monitor {}",
                split_side, monitor_name
            );
            state.set_layout(monitor_name, ZoneLayout::two_column(v_ratio));
        }
    }
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
        let prev_action = last_action.as_deref().unwrap_or("unknown");
        wm.unmaximize_window(&window.id).await?;
        state.save_pre_maximize_geometry(&window.id, current_rect, prev_action);
        state.save_pre_maximize_layout(&monitor.name);
        state.set_layout(&monitor.name, ZoneLayout::single());
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
    } else if was_full_maximized || rects_approx_equal(&current_rect, &full_rect) {
        // Full-size -> restore to tiled position (if saved), otherwise almost-maximize
        if let Some((tile_rect, tile_action)) = state.take_pre_maximize_geometry(&window.id) {
            log::info!("Action: full-size to tiled restore '{}' ({},{} {}x{})", tile_action, tile_rect.x, tile_rect.y, tile_rect.width, tile_rect.height);
            // Restore the saved layout
            if let Some(saved_layout) = state.take_pre_maximize_layout(&monitor.name) {
                state.set_layout(&monitor.name, saved_layout);
            }
            // Remove from the maximize zone before re-registering in the tile zone
            state.zone_find_and_remove(&window.id);
            wm.move_window(&window.id, tile_rect.x, tile_rect.y, tile_rect.width, tile_rect.height)
                .await?;
            // Restore the original tiled action and zone registration
            state.set_last_action(&window.id, &tile_action, mon_idx);
            auto_tab_after_snap(wm, config, state, &window.id, &tile_action, mon_idx, tile_rect).await?;
            // Resurface all other zone tops that were minimized when we maximized
            resurface_all_zones(wm, state).await?;
        } else {
            log::info!("Action: full-size to almost-maximize (no tile saved)");
            wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
                .await?;
            state.set_last_action(&window.id, "almost_maximize", mon_idx);
            auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
        }
    } else if was_almost_maximized || rects_approx_equal(&current_rect, &almost_rect) {
        // Almost-maximized -> full-size (just geometry, no KWin maximize)
        log::info!("Action: almost-maximize to full-size");
        wm.move_window(&window.id, full_rect.x, full_rect.y, full_rect.width, full_rect.height)
            .await?;
        state.set_last_action(&window.id, "full_maximize", mon_idx);
        auto_tab_after_snap(wm, config, state, &window.id, "full_maximize", mon_idx, full_rect).await?;
    } else {
        // Neither -> almost-maximize; save current position so we can return to it
        let prev_action = last_action.as_deref().unwrap_or("unknown");
        log::info!("Action: almost-maximize (saving tile action '{}')", prev_action);
        state.save_pre_snap_geometry(&window.id, current_rect);
        state.save_pre_maximize_geometry(&window.id, current_rect, prev_action);
        state.save_pre_maximize_layout(&monitor.name);
        state.set_layout(&monitor.name, ZoneLayout::single());
        wm.move_window(&window.id, almost_rect.x, almost_rect.y, almost_rect.width, almost_rect.height)
            .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
        state.record_zone_assignment(&window.resource_class, "almost_maximize");
        auto_tab_after_snap(wm, config, state, &window.id, "almost_maximize", mon_idx, almost_rect).await?;
    }

    Ok(())
}

/// Handle snap left/right using layout-based ratio stepping.
/// Pressing Right always moves the boundary rightward (increases ratio).
/// Pressing Left always moves the boundary leftward (decreases ratio).
/// At ratio extremes: growing collapses to single zone, shrinking crosses monitors.
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

    let last_action_name = state
        .get_last_action(&window.id)
        .map(|(a, _)| a);
    let current_leaf = last_action_name
        .as_deref()
        .and_then(ZoneLeafId::from_action);

    let is_left = current_leaf
        .as_ref()
        .map(|l| l.0.starts_with('L'))
        .unwrap_or(false);
    let is_right = current_leaf
        .as_ref()
        .map(|l| l.0.starts_with('R'))
        .unwrap_or(false);
    let is_root = current_leaf
        .as_ref()
        .map(|l| l.0 == "root")
        .unwrap_or(false);
    let is_quarter = current_leaf
        .as_ref()
        .map(|l| l.0.contains('.'))
        .unwrap_or(false);
    let in_spectrum = is_left || is_right || is_root;

    // Fresh snap — not in any known zone
    if !in_spectrum {
        state.save_pre_snap_geometry(&window.id, current_rect);

        let leaf_id = match side {
            SnapSide::Left => ZoneLeafId("L".to_string()),
            SnapSide::Right => ZoneLeafId("R".to_string()),
        };
        let action = leaf_id.to_action();

        // Try smart space first, fall back to standard half
        let final_rect = if let Some(smart_rect) =
            find_empty_space(side, monitor, &windows, &window.id, gap)
        {
            // Set layout ratio based on smart space
            let boundary = (smart_rect.x + smart_rect.width - monitor.x) as f64
                / monitor.width as f64;
            let ratio = match side {
                SnapSide::Left => boundary,
                SnapSide::Right => {
                    (smart_rect.x - monitor.x) as f64 / monitor.width as f64
                }
            };
            // Snap to nearest step
            let ratio = RATIO_STEPS
                .iter()
                .min_by(|a, b| {
                    ((**a) - ratio)
                        .abs()
                        .partial_cmp(&((**b) - ratio).abs())
                        .unwrap()
                })
                .copied()
                .unwrap_or(0.5);
            state.set_layout(&monitor.name, ZoneLayout::two_column(ratio));
            smart_rect
        } else {
            let layout = ZoneLayout::two_column(0.5);
            let rects = layout.compute_rects(monitor, gap);
            let rect = match rects.get(&leaf_id) {
                Some(r) => *r,
                None => {
                    log::warn!("Fresh snap: leaf {:?} not found in layout, falling back to action_to_rect", leaf_id.0);
                    action_to_rect(&leaf_id.to_action(), monitor, gap)
                }
            };
            state.set_layout(&monitor.name, layout);
            rect
        };

        wm.move_window(
            &window.id,
            final_rect.x,
            final_rect.y,
            final_rect.width,
            final_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, &action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &action);
        auto_tab_after_snap(wm, config, state, &window.id, &action, mon_idx, final_rect)
            .await?;
        cooperative_resize_from_layout(wm, state, &window.id, monitor, mon_idx, gap).await?;
        return Ok(());
    }

    // From single zone (almost maximize): break out into two-column
    if is_root {
        let (leaf_id, ratio) = match side {
            SnapSide::Right => (ZoneLeafId("R".to_string()), 1.0 / 3.0),
            SnapSide::Left => (ZoneLeafId("L".to_string()), 2.0 / 3.0),
        };
        let layout = ZoneLayout::two_column(ratio);
        let rects = layout.compute_rects(monitor, gap);
        let rect = rects.get(&leaf_id).copied().unwrap_or_else(|| action_to_rect(&leaf_id.to_action(), monitor, gap));
        state.set_layout(&monitor.name, layout);

        let action = leaf_id.to_action();
        wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height)
            .await?;
        state.set_last_action(&window.id, &action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &action);
        auto_tab_after_snap(wm, config, state, &window.id, &action, mon_idx, rect).await?;
        cooperative_resize_from_layout(wm, state, &window.id, monitor, mon_idx, gap).await?;
        return Ok(());
    }

    // In L or R zone: step the ratio
    let layout = state.get_or_create_layout(&monitor.name);
    // If the layout is single (no split), treat it as a 0.5 two-column for stepping purposes
    let layout = if get_root_v_ratio(&layout).is_none() {
        ZoneLayout::two_column(0.5)
    } else {
        layout
    };
    let current_ratio = get_root_v_ratio(&layout).unwrap_or(0.5);
    let step_up = side == SnapSide::Right;

    // Determine if this step grows or shrinks the window's zone
    let growing = (is_left && step_up) || (is_right && !step_up);

    if let Some(new_ratio) = step_ratio(current_ratio, step_up) {
        // Step to new ratio, preserving any existing horizontal splits
        let new_layout = build_layout_preserving_splits(new_ratio, &layout);
        state.set_layout(&monitor.name, new_layout.clone());

        let rects = new_layout.compute_rects(monitor, gap);
        let my_leaf = current_leaf.as_ref().unwrap();
        let rect = rects.get(my_leaf).copied()
            .unwrap_or_else(|| action_to_rect(&my_leaf.to_action(), monitor, gap));
        let action = my_leaf.to_action();

        wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height)
            .await?;
        state.set_last_action(&window.id, &action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &action);
        auto_tab_after_snap(wm, config, state, &window.id, &action, mon_idx, rect).await?;
        cooperative_resize_from_layout(wm, state, &window.id, monitor, mon_idx, gap).await?;
    } else if growing && !is_quarter {
        // At growth limit for a full-height zone → collapse to single zone (almost maximize)
        // Quarter zones should NOT collapse to maximize — they stay at their quarter size.
        state.set_layout(&monitor.name, ZoneLayout::single());
        let almost_rect = almost_maximize_rect(monitor, gap);
        wm.move_window(
            &window.id,
            almost_rect.x,
            almost_rect.y,
            almost_rect.width,
            almost_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, "almost_maximize", mon_idx);
        state.record_zone_assignment(&window.resource_class, "almost_maximize");
        auto_tab_after_snap(
            wm,
            config,
            state,
            &window.id,
            "almost_maximize",
            mon_idx,
            almost_rect,
        )
        .await?;
    } else {
        // At shrink limit → cross to adjacent monitor
        let direction = match side {
            SnapSide::Right => Direction::Right,
            SnapSide::Left => Direction::Left,
        };
        if let Some(next_mon_idx) = find_monitor_in_direction(
            mon_idx,
            &monitors,
            &config.excluded_monitors,
            direction,
        ) {
            let next_monitor = &monitors[next_mon_idx];
            // Enter opposite side on new monitor at 1/3 ratio
            let (leaf_id, ratio) = match side {
                SnapSide::Right => (ZoneLeafId("L".to_string()), 1.0 / 3.0),
                SnapSide::Left => (ZoneLeafId("R".to_string()), 2.0 / 3.0),
            };
            let new_layout = ZoneLayout::two_column(ratio);
            let rects = new_layout.compute_rects(next_monitor, gap);

            // If in a quarter, preserve the quarter on the new monitor
            let (rect, action) = if is_quarter {
                let is_top = current_leaf
                    .as_ref()
                    .map(|l| l.0.ends_with(".T"))
                    .unwrap_or(false);
                let new_leaf = match (is_top, &leaf_id.0 == "L") {
                    (true, true) => ZoneLeafId("L.T".to_string()),
                    (true, false) => ZoneLeafId("R.T".to_string()),
                    (false, true) => ZoneLeafId("L.B".to_string()),
                    (false, false) => ZoneLeafId("R.B".to_string()),
                };
                // Use the half-width rect but adjust height for quarter
                let half_rect = rects.get(&leaf_id).copied().unwrap_or_else(|| action_to_rect(&leaf_id.to_action(), next_monitor, gap));
                let g = gap as i32;
                let half_h = next_monitor.height / 2;
                let qrect = if is_top {
                    Rect {
                        x: half_rect.x,
                        y: next_monitor.y + g,
                        width: half_rect.width,
                        height: half_h - g - g / 2,
                    }
                } else {
                    Rect {
                        x: half_rect.x,
                        y: next_monitor.y + half_h + g / 2,
                        width: half_rect.width,
                        height: next_monitor.height - half_h - g - g / 2,
                    }
                };
                (qrect, new_leaf.to_action())
            } else {
                (rects.get(&leaf_id).copied().unwrap_or_else(|| action_to_rect(&leaf_id.to_action(), next_monitor, gap)), leaf_id.to_action())
            };

            state.set_layout(&next_monitor.name, new_layout);
            wm.move_window(&window.id, rect.x, rect.y, rect.width, rect.height)
                .await?;
            state.set_last_action(&window.id, &action, next_mon_idx);
            state.record_zone_assignment(&window.resource_class, &action);
            auto_tab_after_snap(wm, config, state, &window.id, &action, next_mon_idx, rect)
                .await?;
        }
        // else: no monitor in that direction, no-op
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
            // Compute quarter rect from the next monitor's layout
            let next_layout = state.get_or_create_layout(&next_monitor.name);
            let next_rects = next_layout.compute_rects(next_monitor, config.gap_size);
            let parent_leaf = if target_action.contains("left") {
                ZoneLeafId("L".to_string())
            } else {
                ZoneLeafId("R".to_string())
            };
            let half_rect = next_rects
                .get(&parent_leaf)
                .copied()
                .unwrap_or_else(|| action_to_rect(target_action, next_monitor, config.gap_size));
            let g2 = config.gap_size as i32;
            let half_h2 = next_monitor.height / 2;
            let target_rect = if target_action.contains("top") {
                Rect {
                    x: half_rect.x,
                    y: next_monitor.y + g2,
                    width: half_rect.width,
                    height: half_h2 - g2 - g2 / 2,
                }
            } else {
                Rect {
                    x: half_rect.x,
                    y: next_monitor.y + half_h2 + g2 / 2,
                    width: half_rect.width,
                    height: next_monitor.height - half_h2 - g2 - g2 / 2,
                }
            };
            wm.move_window(&window.id, target_rect.x, target_rect.y, target_rect.width, target_rect.height).await?;
            state.set_last_action(&window.id, target_action, next_mon_idx);
            state.record_zone_assignment(&window.resource_class, target_action);
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
    let layout = state.get_or_create_layout(&monitor.name);

    // Quarter pressing opposite direction → go to full height (merge back to parent)
    if (direction == SnapVertical::Down && is_already_top)
        || (direction == SnapVertical::Up && is_already_bottom)
    {
        // Merge the horizontal split back to a single leaf in the layout
        let v_ratio = get_root_v_ratio(&layout).unwrap_or(0.5);
        let new_layout = ZoneLayout::two_column(v_ratio);
        state.set_layout(&monitor.name, new_layout.clone());

        let parent_leaf = match side_context {
            "left" => ZoneLeafId("L".to_string()),
            _ => ZoneLeafId("R".to_string()),
        };
        let rects = new_layout.compute_rects(monitor, gap);
        let target_rect = rects
            .get(&parent_leaf)
            .copied()
            .unwrap_or_else(|| match side_context {
                "left" => action_to_rect("snap_left", monitor, gap),
                _ => action_to_rect("snap_right", monitor, gap),
            });
        let target_action = parent_leaf.to_action();

        log::info!(
            "Snap vertical: {} -> {} (quarter to full height) at ({},{} {}x{})",
            last_action.as_deref().unwrap_or("none"),
            target_action,
            target_rect.x, target_rect.y, target_rect.width, target_rect.height
        );

        wm.move_window(&window.id, target_rect.x, target_rect.y, target_rect.width, target_rect.height).await?;
        state.set_last_action(&window.id, &target_action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &target_action);
        auto_tab_after_snap(wm, config, state, &window.id, &target_action, mon_idx, target_rect).await?;
        return Ok(());
    }

    // Full height → quarter: add horizontal split to the layout tree
    let v_ratio = get_root_v_ratio(&layout).unwrap_or(0.5);
    let new_layout = if side_context == "left" {
        ZoneLayout::left_split_and_right(v_ratio, 0.5)
    } else {
        ZoneLayout::left_and_right_split(v_ratio, 0.5)
    };
    state.set_layout(&monitor.name, new_layout.clone());

    let target_action = match (side_context, direction) {
        ("left", SnapVertical::Up) => "snap_top_left",
        ("left", SnapVertical::Down) => "snap_bottom_left",
        ("right", SnapVertical::Up) => "snap_top_right",
        ("right", SnapVertical::Down) => "snap_bottom_right",
        _ => unreachable!(),
    };

    let leaf_id = ZoneLeafId::from_action(target_action).unwrap();
    let rects = new_layout.compute_rects(monitor, gap);
    let target_rect = rects
        .get(&leaf_id)
        .copied()
        .unwrap_or_else(|| action_to_rect(target_action, monitor, gap));

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
    state.record_zone_assignment(&window.resource_class, target_action);
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
                        surface_zone_entry(wm, state, entry, config.gap_size).await?;
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
    config: &PaveConfig,
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
        let gap = config.gap_size;
        let mut last_shown_id = None;

        for entry in &to_show {
            // Compute current zone rect from the layout tree
            let rect = compute_zone_rect_for_entry(state, &entry.snap_action, &monitors, gap)
                .unwrap_or(entry.geometry);

            wm.unminimize_window(&entry.window_id).await?;
            wm.move_window(
                &entry.window_id,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            )
            .await?;

            // Set last_action for each shown window
            let mon_idx = mon_idx_from_geometry(&rect, &monitors);
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

            let monitors = wm.get_monitors().await?;
            let rect = compute_zone_rect_for_entry(state, &entry.snap_action, &monitors, config.gap_size)
                .unwrap_or(entry.geometry);

            wm.unminimize_window(&entry.window_id).await?;
            wm.move_window(
                &entry.window_id,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            )
            .await?;
            wm.activate_window(&entry.window_id).await?;

            let mon_idx = mon_idx_from_geometry(&rect, &monitors);
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
    state: &TilingState,
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

    // Infer the new vertical split ratio from the resized window and update the layout.
    // This ensures that subsequent snaps and cooperative resizes use the dragged boundary.
    if matches!(edge, ResizedEdge::Left | ResizedEdge::Right) {
        let gap = config.gap_size;
        let usable_width = monitor.width - gap as i32;
        // The right edge of the left zone determines the ratio
        let left_zone_right = if matches!(edge, ResizedEdge::Right) {
            // The resized window is on the left side — its new right edge
            event.new_geometry.x + event.new_geometry.width
        } else {
            // The resized window is on the right side — use the adjacent window's right edge,
            // or compute from the new left edge minus the gap
            event.new_geometry.x - gap as i32
        };
        let left_zone_width = left_zone_right - monitor.x - gap as i32;
        // The left zone width = usable_width * ratio - gap/2, so:
        // ratio = (left_zone_width + gap/2) / usable_width
        let half_gap = gap as f64 / 2.0;
        let ratio = (left_zone_width as f64 + half_gap) / usable_width as f64;
        let ratio = ratio.clamp(0.2, 0.8);

        let mut layouts = state.zone_layouts.lock().unwrap();
        let layout = layouts
            .entry(monitor.name.clone())
            .or_insert_with(|| ZoneLayout::two_column(0.5));

        // Only update if the layout is a simple two-column split
        if get_root_v_ratio(layout).is_some() {
            *layout = ZoneLayout::two_column(ratio);
            log::info!(
                "Resize event: updated layout ratio to {:.3} for monitor {}",
                ratio,
                monitor.name
            );
        }
    }

    Ok(())
}

/// After a layout ratio change, recompute all leaf rects from the layout and
/// move every tracked window on this monitor to its new position.
async fn cooperative_resize_from_layout(
    wm: &KWinBackend,
    state: &TilingState,
    snapped_window_id: &str,
    monitor: &MonitorInfo,
    mon_idx: usize,
    gap: u32,
) -> Result<(), String> {
    let layout = state.get_or_create_layout(&monitor.name);
    let rects = layout.compute_rects(monitor, gap);

    let windows = wm.get_windows().await?;

    for w in &windows {
        if w.id == snapped_window_id || w.minimized || !is_window_on_monitor(w, monitor) {
            continue;
        }

        let action = match state.get_last_action(&w.id).map(|(a, _)| a) {
            Some(a) => a,
            None => continue,
        };

        let leaf_id = match ZoneLeafId::from_action(&action) {
            Some(l) => l,
            None => continue,
        };

        // Look up the leaf rect directly from the layout tree
        let target_rect = if let Some(rect) = rects.get(&leaf_id) {
            *rect
        } else {
            // Quarter leaf not in layout (layout doesn't have horizontal split yet) — skip
            continue;
        };

        let wrect = window_to_rect(w);
        if !rects_approx_equal(&wrect, &target_rect) {
            log::info!(
                "Cooperative resize: '{}' -> ({},{} {}x{})",
                w.title, target_rect.x, target_rect.y, target_rect.width, target_rect.height
            );
            wm.move_window(
                &w.id,
                target_rect.x,
                target_rect.y,
                target_rect.width,
                target_rect.height,
            )
            .await?;

            // Update the zone tracker geometry
            let zone_id = ZoneId {
                monitor_idx: mon_idx,
                leaf_id: leaf_id.clone(),
            };
            let mut tracker = state.zone_tracker.lock().unwrap();
            if let Some(entries) = tracker.zones.get_mut(&zone_id) {
                if let Some(entry) = entries.iter_mut().find(|e| e.window_id == w.id) {
                    entry.geometry = target_rect;
                }
            }
        }
    }

    Ok(())
}

/// Legacy adjacency fallback for zone snap. Covers cases that the BSP tree
/// doesn't model (half-zone → quarter, quarter → quarter vertical).
/// This will be removed once handle_snap_vertical updates the layout tree.
fn legacy_adjacent(leaf_id: &ZoneLeafId, dir: Direction) -> Option<ZoneLeafId> {
    let id = leaf_id.0.as_str();
    match (id, dir) {
        // Half zones: vertical → quarter
        ("L", Direction::Down) => Some(ZoneLeafId("L.B".to_string())),
        ("L", Direction::Up) => Some(ZoneLeafId("L.T".to_string())),
        ("R", Direction::Down) => Some(ZoneLeafId("R.B".to_string())),
        ("R", Direction::Up) => Some(ZoneLeafId("R.T".to_string())),
        // Quarter zones: horizontal
        ("L.T", Direction::Right) => Some(ZoneLeafId("R.T".to_string())),
        ("R.T", Direction::Left) => Some(ZoneLeafId("L.T".to_string())),
        ("L.B", Direction::Right) => Some(ZoneLeafId("R.B".to_string())),
        ("R.B", Direction::Left) => Some(ZoneLeafId("L.B".to_string())),
        // Quarter zones: vertical
        ("L.T", Direction::Down) => Some(ZoneLeafId("L.B".to_string())),
        ("L.B", Direction::Up) => Some(ZoneLeafId("L.T".to_string())),
        ("R.T", Direction::Down) => Some(ZoneLeafId("R.B".to_string())),
        ("R.B", Direction::Up) => Some(ZoneLeafId("R.T".to_string())),
        _ => None,
    }
}

/// Map a snap action name to its standard geometry on a monitor.
/// Uses a default 50/50 two-column layout to compute zone rects.
fn action_to_rect(action: &str, monitor: &MonitorInfo, gap: u32) -> Rect {
    if action == "almost_maximize" || action == "full_maximize" {
        return almost_maximize_rect(monitor, gap);
    }

    let layout = ZoneLayout::two_column(0.5);
    let rects = layout.compute_rects(monitor, gap);

    if let Some(leaf_id) = ZoneLeafId::from_action(action) {
        // For quarter zones (e.g. snap_top_left), the layout only has L/R leaves.
        // Compute the quarter from the parent half.
        if let Some(rect) = rects.get(&leaf_id) {
            return *rect;
        }
        // Quarter zone: get parent half, then split vertically
        if let Some(parent) = leaf_id.immediate_parent() {
            if let Some(half_rect) = rects.get(&parent) {
                let g = gap as i32;
                let half_h = monitor.height / 2;
                let is_top = leaf_id.0.ends_with(".T");
                return if is_top {
                    Rect {
                        x: half_rect.x,
                        y: monitor.y + g,
                        width: half_rect.width,
                        height: half_h - g - g / 2,
                    }
                } else {
                    Rect {
                        x: half_rect.x,
                        y: monitor.y + half_h + g / 2,
                        width: half_rect.width,
                        height: half_h - g - g / 2,
                    }
                };
            }
        }
    }

    almost_maximize_rect(monitor, gap)
}

/// Move the active window into an adjacent zone. The displaced window in the
/// target zone gets minimized and joins the tab cycle stack.
pub async fn handle_zone_snap(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
    direction: Direction,
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
    let gap = config.gap_size;

    // Determine the window's current zone leaf
    let current_leaf_id = {
        let tracker = state.zone_tracker.lock().unwrap();
        tracker.find_zone(&window.id).map(|z| z.leaf_id.clone())
    };

    let current_leaf_id = match current_leaf_id {
        Some(s) => s,
        None => {
            // Try to infer from last_action
            let action = state.get_last_action(&window.id).map(|(a, _)| a);
            match action.as_deref().and_then(ZoneLeafId::from_action) {
                Some(s) => s,
                None => {
                    log::debug!("Zone snap: window not in any zone");
                    return Ok(());
                }
            }
        }
    };

    // Find adjacent zone using the monitor's zone layout, with legacy fallback
    let adj_dir = match direction {
        Direction::Left => AdjacentDirection::Left,
        Direction::Right => AdjacentDirection::Right,
        Direction::Up => AdjacentDirection::Up,
        Direction::Down => AdjacentDirection::Down,
    };
    let layout = state.get_or_create_layout(&monitor.name);
    let target_leaf_id = match layout
        .adjacent_leaf(&current_leaf_id, adj_dir)
        .or_else(|| legacy_adjacent(&current_leaf_id, direction))
    {
        Some(s) => s,
        None => {
            log::debug!("Zone snap: no adjacent zone in direction {:?}", direction);
            return Ok(());
        }
    };

    let target_action = target_leaf_id.to_action();
    let source_action = current_leaf_id.to_action();

    let source_zone_id = ZoneId {
        monitor_idx: mon_idx,
        leaf_id: current_leaf_id,
    };
    let target_zone_id = ZoneId {
        monitor_idx: mon_idx,
        leaf_id: target_leaf_id,
    };

    // Check if the target zone is occupied — get the top entry's geometry
    let target_top_entry = {
        let tracker = state.zone_tracker.lock().unwrap();
        tracker
            .zones
            .get(&target_zone_id)
            .and_then(|entries| entries.last().cloned())
            .filter(|e| e.window_id != window.id)
    };

    let target_zone_empty = target_top_entry.is_none();

    // Target geometry: use the existing zone occupant's geometry if present,
    // otherwise compute from the layout tree
    let target_rect = target_top_entry
        .as_ref()
        .map(|e| e.geometry)
        .unwrap_or_else(|| {
            let rects = layout.compute_rects(monitor, gap);
            rects.get(&target_zone_id.leaf_id)
                .copied()
                .unwrap_or_else(|| action_to_rect(&target_action, monitor, gap))
        });

    // Save the source zone's geometry before we remove the window,
    // so we remember the zone's size if it becomes empty.
    let source_rect = {
        let tracker = state.zone_tracker.lock().unwrap();
        tracker
            .zones
            .get(&source_zone_id)
            .and_then(|entries| entries.iter().find(|e| e.window_id == window.id))
            .map(|e| e.geometry)
            .unwrap_or_else(|| window_to_rect(&window))
    };

    // Remove the active window from its current zone
    {
        let mut tracker = state.zone_tracker.lock().unwrap();
        tracker.remove_window(&window.id);

        // If the source zone is now empty, remember its geometry
        let source_empty = tracker
            .zones
            .get(&source_zone_id)
            .map_or(true, |e| e.is_empty());
        if source_empty {
            state.zone_last_geometry.lock().unwrap()
                .insert(source_zone_id.clone(), source_rect);
        }
    }

    // If the target zone is empty, try to use remembered geometry
    let target_rect = if target_zone_empty {
        state.zone_last_geometry.lock().unwrap()
            .remove(&target_zone_id)
            .unwrap_or(target_rect)
    } else {
        target_rect
    };

    if target_zone_empty {
        // Target zone is empty — move window there visibly
        {
            let mut tracker = state.zone_tracker.lock().unwrap();
            tracker.place_window_silent(
                target_zone_id,
                &window.id,
                &target_action,
                target_rect,
            );
        }

        wm.move_window(
            &window.id,
            target_rect.x,
            target_rect.y,
            target_rect.width,
            target_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, &target_action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &target_action);

        log::info!(
            "Zone snap: moved '{}' from {} to empty zone {} ({},{} {}x{})",
            window.title,
            source_action,
            target_action,
            target_rect.x,
            target_rect.y,
            target_rect.width,
            target_rect.height,
        );
    } else {
        // Target zone is occupied — moved window becomes visible on top,
        // existing window gets pushed into the stack (minimized)
        let displaced = target_top_entry.unwrap();

        {
            let mut tracker = state.zone_tracker.lock().unwrap();
            tracker.place_window_silent(
                target_zone_id,
                &window.id,
                &target_action,
                target_rect,
            );
        }

        // Minimize the displaced window — it stays in the zone's tab stack
        wm.minimize_window(&displaced.window_id).await?;

        // Move the active window to the target zone (visible)
        wm.move_window(
            &window.id,
            target_rect.x,
            target_rect.y,
            target_rect.width,
            target_rect.height,
        )
        .await?;
        state.set_last_action(&window.id, &target_action, mon_idx);
        state.record_zone_assignment(&window.resource_class, &target_action);

        log::info!(
            "Zone snap: moved '{}' from {} onto {} (displaced {} into stack)",
            window.title,
            source_action,
            target_action,
            displaced.window_id,
        );
    }

    // Surface exactly one window in the vacated source zone (not descendants/parents).
    // This prevents cascading surface operations that cause overlapping windows.
    if config.auto_surface_tabs {
        let surf_entry = {
            let tracker = state.zone_tracker.lock().unwrap();
            tracker.zones.get(&source_zone_id)
                .and_then(|entries| entries.last().cloned())
                .filter(|e| e.window_id != window.id)
        };
        if let Some(entry) = surf_entry {
            surface_zone_entry(wm, state, &entry, gap).await?;
        }
    }

    // Reconcile the layout — if the window left a quarter zone and no other
    // quarter windows remain, merge the horizontal split back.
    reconcile_layout(state, &monitor.name, mon_idx);

    Ok(())
}

/// Payload from KWin's windowAdded D-Bus event.
#[derive(Debug, Deserialize)]
struct WindowAddedEvent {
    id: String,
    resource_class: String,
}

/// Handle a newly opened window: auto-place it into a remembered or largest zone.
pub async fn handle_auto_place(
    wm: &KWinBackend,
    config: &PaveConfig,
    state: &TilingState,
    payload: &str,
) -> Result<(), String> {
    let event: WindowAddedEvent =
        serde_json::from_str(payload).map_err(|e| format!("Failed to parse window added event: {e}"))?;

    if event.resource_class.is_empty() {
        return Ok(());
    }

    // Skip apps that should never be auto-placed (screenshot tools, system overlays, etc.)
    let skip_classes = [
        "org.kde.spectacle", "spectacle", "flameshot", "gnome-screenshot",
        "xdg-desktop-portal", "org.kde.polkit-kde-authentication-agent-1",
        "plasmashell", "krunner", "org.kde.krunner",
    ];
    let class_lower = event.resource_class.to_lowercase();
    if skip_classes.iter().any(|s| class_lower == *s) {
        return Ok(());
    }

    // Brief delay to let KWin settle the window geometry
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let windows = wm.get_windows().await?;
    let window = match windows.iter().find(|w| w.id == event.id) {
        Some(w) => w,
        None => return Ok(()), // Window already closed
    };

    // Skip if already maximized or minimized
    if window.maximized || window.minimized {
        return Ok(());
    }

    let monitors = wm.get_monitors().await?;
    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let mon_idx = find_window_monitor(window, &monitors);
    let monitor = &monitors[mon_idx];

    // Skip excluded monitors
    if config.excluded_monitors.iter().any(|m| m == &monitor.name) {
        return Ok(());
    }

    let gap = config.gap_size;

    // Look up remembered zone or find the largest available
    let (target_leaf, target_rect) = if let Some(leaf_id) = state.get_zone_assignment(&event.resource_class) {
        // Check if the remembered zone exists in the current layout
        let layout = state.get_or_create_layout(&monitor.name);
        let rects = layout.compute_rects(monitor, gap);
        if let Some(rect) = rects.get(&leaf_id) {
            (leaf_id, *rect)
        } else {
            // Remembered zone no longer exists — fall back to largest
            match find_largest_available_zone(state, &monitor.name, monitor, mon_idx, gap) {
                Some((leaf, rect)) => (leaf, rect),
                None => return Ok(()),
            }
        }
    } else {
        match find_largest_available_zone(state, &monitor.name, monitor, mon_idx, gap) {
            Some((leaf, rect)) => (leaf, rect),
            None => return Ok(()),
        }
    };

    let action = target_leaf.to_action();
    log::info!(
        "Auto-place: '{}' ({}) -> zone {} ({},{} {}x{})",
        event.resource_class, event.id, target_leaf, target_rect.x, target_rect.y, target_rect.width, target_rect.height
    );

    wm.move_window(&event.id, target_rect.x, target_rect.y, target_rect.width, target_rect.height)
        .await?;
    state.set_last_action(&event.id, &action, mon_idx);
    auto_tab_after_snap(wm, config, state, &event.id, &action, mon_idx, target_rect).await?;

    Ok(())
}

/// Find the largest available zone on a monitor, preferring unoccupied zones.
fn find_largest_available_zone(
    state: &TilingState,
    monitor_name: &str,
    monitor: &MonitorInfo,
    monitor_idx: usize,
    gap: u32,
) -> Option<(ZoneLeafId, Rect)> {
    let layout = state.get_or_create_layout(monitor_name);
    let rects = layout.compute_rects(monitor, gap);

    if rects.is_empty() {
        return None;
    }

    // Get occupied zone IDs for this monitor
    let occupied: Vec<ZoneLeafId> = {
        let tracker = state.zone_tracker.lock().unwrap();
        tracker.zones.iter()
            .filter(|(zone_id, entries)| zone_id.monitor_idx == monitor_idx && !entries.is_empty())
            .map(|(zone_id, _)| zone_id.leaf_id.clone())
            .collect()
    };

    // Try unoccupied zones first, sorted by area (largest first)
    let mut unoccupied: Vec<_> = rects.iter()
        .filter(|(leaf_id, _)| !occupied.contains(*leaf_id))
        .collect();
    unoccupied.sort_by(|(_, a), (_, b)| {
        let area_a = a.width as i64 * a.height as i64;
        let area_b = b.width as i64 * b.height as i64;
        area_b.cmp(&area_a)
    });

    if let Some((leaf_id, rect)) = unoccupied.first() {
        return Some(((*leaf_id).clone(), **rect));
    }

    // All zones occupied — pick the largest overall
    let mut all: Vec<_> = rects.iter().collect();
    all.sort_by(|(_, a), (_, b)| {
        let area_a = a.width as i64 * a.height as i64;
        let area_b = b.width as i64 * b.height as i64;
        area_b.cmp(&area_a)
    });

    all.first().map(|(leaf_id, rect)| ((*leaf_id).clone(), **rect))
}
