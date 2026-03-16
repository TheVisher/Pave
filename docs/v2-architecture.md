# Pave v2 — Zone-First Architecture

## Overview

Pave v2 is a ground-up rewrite. The core idea: **zones are the only primitive.** Windows don't have geometry — they inherit it from the zone they belong to. Resizing doesn't operate on windows — it moves split points, and every window follows.

Built as a native KDE application (C++20 / Qt6 / KDE Frameworks 6) instead of Rust/Tauri. Runs as a systemd daemon with KCM settings integration.

---

## Core Model

### Zone Layout

A monitor's layout is defined by **split points**, not window geometry.

```
Vertical split:          Vertical + horizontal split:

┌──────────┬──────────┐  ┌──────────┬──────────┐
│          │          │  │          │   Zone   │
│  Zone 1  │  Zone 2  │  │  Zone 1  │    2     │
│          │          │  │          ●──────────│
│          │          │  │          │   Zone   │
│          │          │  │          │    3     │
└──────────┴──────────┘  └──────────┴──────────┘
     one split point          one convergence point
```

**Constraints:**
- One vertical split per monitor (divides into left/right columns)
- One horizontal split per column (divides a column into top/bottom)
- Maximum 4 zones per monitor (L, R, L.T+L.B, R.T+R.B, or both split)

**Vertical split ratios:** 1/4, 1/3, 1/2, 2/3, 3/4
**Horizontal split ratios:** 1/3, 1/2, 2/3

Fine-grained control beyond these steps is done by **mouse-dragging the convergence point**.

### Convergence Point

The convergence point is where split lines meet. It serves three roles:

1. **Drag handle** — grab with mouse to fine-tune zone ratios beyond keyboard steps
2. **App switcher anchor** — press a shortcut to show zone assignments radiating outward from it
3. **Layout control center** — keyboard shortcuts move it

For a simple vertical split, the convergence point sits at the midpoint of the vertical divider. When a horizontal split is added, it moves to the intersection.

### Window Assignment

Windows are assigned to zones, not positioned by geometry.

```
Data model:

Monitor → ZoneLayout (split ratios)
Zone → [Window] (assigned app classes)
Window → Zone (current assignment, if any)
```

**Assignment rules:**
- New windows float (unmanaged) until explicitly assigned
- Dragging a window into a zone assigns it
- Snapping via keyboard assigns it
- Assignments persist by app class (e.g., "zen-browser" → Zone 1)

**Adaptive layout based on running apps:**
- 0 assigned apps running → no zones, just desktop
- 1 assigned app running → almost maximize (full monitor with gaps)
- 2+ assigned apps running → split layout activates with assigned ratios
- Closing an app → remaining zones expand to fill (configurable: expand vs. keep empty)
- Reopening an app → layout splits back, zone reclaims its ratio

This means the layout *emerges* from what's running. You don't manage empty zones.

### Window Stacking

A zone can hold multiple assigned apps. Only the most recently focused one is visible — all others in that zone are **minimized** (important for transparency — no windows bleeding through behind the active one).

Switching between stacked windows uses normal **Alt+Tab**. When a window in a zone gains focus:
1. The previously visible window in that zone is minimized
2. The newly focused window is unminimized and sized to the zone rect

No dedicated tab-cycle shortcut needed. The convergence point popup (Phase 2) provides a visual way to see and select all apps in each zone.

### Per-Desktop Scoping

Every virtual desktop has its own independent:
- Zone layout (split ratios)
- Window-to-zone assignments
- Managed/unmanaged state

A desktop can be **managed** (Pave controls zones) or **unmanaged** (Pave ignores it entirely). Unmanaged desktops are the solution for fullscreen apps and games — just put them on another desktop.

---

## Keyboard Shortcuts

### Zone Layout Control (no focus required)

| Shortcut | Action |
|----------|--------|
| Meta+Alt+Left | Move convergence point left (decrease vertical ratio one step) |
| Meta+Alt+Right | Move convergence point right (increase vertical ratio one step) |
| Meta+Alt+Up | Move horizontal split up / create horizontal split at 1/2 |
| Meta+Alt+Down | Move horizontal split down / create horizontal split at 1/2 |

These operate on the **layout**, not on any window. Every window in every affected zone resizes simultaneously.

When moving a horizontal split: if multiple columns have horizontal splits, the split in the focused window's column is affected.

### Window Actions (focused window)

| Shortcut | Action |
|----------|--------|
| Meta+Alt+Return | Almost maximize cycle: tiled → almost max → full → restore to tiled |
| Shift+Meta+Alt+Left/Right/Up/Down | Move focused window to adjacent zone |
| Meta+Alt+Z | Unassign window from zone, restore to floating |

### Convergence Point UI

| Shortcut | Action |
|----------|--------|
| Meta+Alt+Space (TBD) | Show zone overview popup at convergence point |

The popup shows all zones with their assigned apps. Click to focus, drag to reassign.

---

## Almost Maximize Cycle

Preserved from v1. The three-state cycle:

1. **Tiled** → press Return → **Almost maximized** (full monitor, with gaps)
2. **Almost maximized** → press Return → **Full maximized** (1px gap, near-fullscreen)
3. **Full maximized** → press Return → **Restore to tiled** (back to zone assignment)

When almost-maximized, other zone windows can be hidden or shown behind. On restore, the full zone layout reactivates.

---

## Gaps & Padding

- Configurable gap size between zones and monitor edges (default: 15px)
- Gaps are visual only — they're computed when zone rects are calculated
- The gap between zones is where the convergence point drag handle lives
- Future: the gap becomes interactive UI (hover for tab popover, drag handle highlight)

---

## Drag-to-Zone (Phase 2)

Using LayerShellQt for Wayland overlays:

- Hold Alt + begin dragging a window → zone overlay appears
- Translucent highlight shows which zone the window will snap to
- Release to assign window to that zone
- Convergence point visible during drag as a grab handle

Deferred to Phase 2 but the zone model supports it from day one.

---

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | C++20 |
| UI Framework | Qt 6.6+ |
| KDE Integration | KDE Frameworks 6 |
| Wayland Overlays | LayerShellQt |
| Window Management | D-Bus (KWin) |
| Settings | KCM (KDE Control Module) |
| Service | Systemd user daemon |
| Build | CMake 3.16+ |

### Why Native KDE

- Direct D-Bus integration without KWin script indirection
- LayerShellQt for proper Wayland overlays (drag-to-snap, convergence point UI)
- KCM settings appear in System Settings natively
- Systemd service management (start/stop/enable)
- No web runtime overhead (Tauri webview)

---

## State Management

v1 had 7 interacting state stores. v2 has 2 core + 1 convenience:

1. **ZoneLayout** per monitor per desktop — the split ratios (source of truth)
2. **ZoneAssignment** — maps app class → zone ID (persisted to config)
3. **PreSnapGeometry** per window — original floating geometry for restore

Zone rects are **always computed fresh** from the layout ratios + monitor geometry + gap size. Never cached, never stale. When a ratio changes, every zone rect is recomputed and every window is moved to match.

---

## Configuration

Stored in `~/.config/pave/config.toml` (or KDE's config system):

```toml
[general]
gap_size = 15

[desktop.1]
managed = true

[desktop.1.monitor."DP-1"]
vertical_ratio = 0.5
left_horizontal_ratio = 0.0    # 0.0 = no split
right_horizontal_ratio = 0.5   # split right column at 1/2

[desktop.2]
managed = false                 # gaming desktop, Pave ignores

[assignments]
"zen-browser" = { desktop = 1, monitor = "DP-1", zone = "L" }
"dev.zed.Zed" = { desktop = 1, monitor = "DP-1", zone = "R.T" }
"Alacritty" = { desktop = 1, monitor = "DP-1", zone = "R.B" }
```

---

## Migration from v1

v2 is a full rewrite — no code migration. Feature parity with v1 is achieved when:
- [x] Snap left/right with ratio stepping
- [x] Snap up/down (horizontal splits)
- [x] Almost maximize cycle
- [x] Cooperative resize (now implicit — moving convergence point resizes all)
- [x] Gaps/padding
- [x] Per-monitor layouts
- [ ] Presets / session restore (deferred)
- [x] Window stacking per zone (via Alt+Tab, minimize inactive)
- [ ] Drag-to-zone overlays
- [ ] Convergence point UI popup
- [ ] KCM settings panel

---

## Design Decisions

1. **Snap past the edge** — stepping the ratio past the last step (e.g., past 3/4) collapses the shrinking zone. The window in that zone is hidden (minimized). Stepping back restores the zone and resurfaces the window. No destructive action — the assignment is preserved, the zone is just dormant.

2. **Monitor targeting** — convergence point movement affects whichever monitor the mouse cursor is on. No need for the keyboard-focused window to be on that monitor.

3. **Expand-to-fill** — enabled by default. When a zone empties (app closed or zone collapsed), neighboring zones expand to fill the space. When the zone reactivates, neighbors shrink back.

4. **Zone limit** — 4 zones per monitor (2 columns × 2 rows max). Sufficient for ultrawides and standard monitors alike. Not planning for 3+ columns.
