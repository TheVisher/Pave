# Pave Roadmap

## Vision

Pave is a zone-based window tiling manager that goes beyond snapping windows to halves and thirds. The end goal is a system where monitors have persistent named zones, apps are assigned to zones, tabbed stacking lets multiple apps share a zone, and the gap between windows becomes interactive UI for navigating it all — no keybinds required.

---

## v1.0 — Solid Tiler (Current)

**Status:** Feature-complete, needs polish for public release.

### Features
- Bidirectional snap spectrum: Left/Right arrows step through 7 width positions (L1/3 → L1/2 → L2/3 → Full → R2/3 → R1/2 → R1/3)
- Quarter tiling with width cycling: Up/Down splits into quarters, Left/Right cycles quarter widths
- Vertical spectrum: Top quarter ↔ Full height ↔ Bottom quarter
- Monitor-aware navigation: arrows cross to adjacent monitors based on physical layout
- Smart space detection: first snap fills the largest empty area
- Cooperative resize: resizing one window adjusts its neighbor
- Presets: save and restore full window layouts (tray menu + CiderDeck + D-Bus)
- Session restore: save window state on quit, restore on startup
- Auto-surface: hidden windows reappear when the covering window moves away
- Tab zone cycling: Ctrl+Alt+Tab cycles through stacked windows in a zone
- Gap size and corner radius settings (ShapeCorners integration)
- System tray with preset menu and settings
- Stale process cleanup on startup

### TODO for v1.0 Release
- [x] README (structure done, needs screenshots/GIFs)
- [ ] Screenshots/GIFs for README showing key features
- [ ] Clean up compiler warnings (dead code)
- [x] AUR package (`pave-git`)
- [x] License file (MIT)
- [x] Settings UI polish (current layout names, active preset indicator)
- [ ] First-run onboarding (brief overlay showing shortcuts)

---

## v1.5 — Gap Popover

**The killer feature.** The padding gap between tiled windows becomes interactive UI.

### Features
- Hover over the gap between two zones → a split popover appears after ~300ms
- Left half shows tabs for the left zone, right half shows tabs for the right zone
- Click a tab to switch the active app in that zone
- Popover follows cursor position along the gap
- Popover closes on mouse leave or after clicking a tab
- Draggable anchor point — reposition the popover along the gap, saved per-gap
- Keyboard navigation: arrow keys + Enter once popover is open

### Implementation Notes
- Popover is a transparent Tauri overlay window, positioned dynamically
- Needs cursor position tracking (polling or KWin compositor events)
- Zone tracker already knows which windows are in each zone
- Gap geometry is computable from zone positions + gap size

### Stretch Goals
- Intersection popovers: where 3+ zones meet, show a multi-section popover
- Tab drag between zones via the popover
- App icons in tab entries (fetch from .desktop files)

---

## v2.0 — Zone Manager

**Persistent zones become first-class citizens.** Apps get assigned to zones and open there automatically.

### Features
- Named persistent zones per monitor (survive across sessions)
- Snap-to-assign: snapping a window into a zone assigns that app class to it
- Right-click context menu on tabs: Add to Zone, Pin to Zone, Unassign
- Pinned apps can't be dragged to other zones
- Auto-placement: assigned apps open directly in their zone on launch
- True window hiding: hidden windows removed from taskbar (off-screen or hidden virtual desktop)
- Layout editor in settings: visual zone editor instead of only capturing from live windows
- Fullscreen mode: Ctrl+Alt+Enter fullscreens over all zones, unified tab strip appears

### Implementation Notes
- Zone definitions stored in config (monitor name → zone layout)
- App-to-zone mapping stored by window class
- KWin window rules written dynamically for auto-placement
- Layout editor is a canvas component in the settings webview

---

## v2.5 — Multi-DE Support

**Expand beyond KDE.** Abstract the platform layer so Pave works on other compositors.

### Priority Order
1. **Hyprland** — Clean IPC, huge ricing community, high demand
2. **Sway / wlroots** — Similar IPC model, overlapping audience
3. **COSMIC** — System76's new DE, growing user base
4. **Gnome (Mutter)** — Large user base but extension system is restrictive

### Architecture
- `KWinBackend` is already isolated — add `HyprlandBackend`, `SwayBackend`, etc.
- Each backend implements: get_monitors, get_windows, move_window, minimize, shortcuts
- Shortcut registration differs per compositor (KWin scripts vs Hyprland binds vs Sway bindsym)
- Gap popover overlay window may need compositor-specific transparency/layering

### Per-Backend Effort Estimate
| Backend | Window Mgmt | Shortcuts | Overlay Windows | Effort |
|---------|------------|-----------|----------------|--------|
| KWin (done) | D-Bus | KWin scripts | Tauri window | ✅ Done |
| Hyprland | IPC socket | hyprctl binds | wlr-layer-shell | Medium |
| Sway | IPC socket | bindsym | wlr-layer-shell | Medium |
| COSMIC | Unknown (new) | TBD | TBD | Unknown |
| Mutter | D-Bus (limited) | GJS extension | GTK overlay | Hard |

---

## v3.0 — Cross-Platform

**macOS and Windows support.** Lower priority — these platforms have more competition and less community demand.

### macOS
- Accessibility API for window management (like Rectangle/Amethyst)
- Requires user permission grants
- No compositor-level hooks — event-driven approach needed
- Overlay windows work well on macOS (transparent NSWindow)

### Windows
- Win32 API for window management
- Competes directly with Windows 11 Snap Layouts and FancyZones
- Overlay windows via transparent WinAPI or Tauri window

---

## Future Ideas (Unscheduled)

These are ideas that could land in any version if they make sense:

| Feature | Description |
|---------|-------------|
| **Focused Presets** | Activating a preset minimizes all windows not in the preset — full context switch |
| **Companion Lock** | Pair two windows so they move together (editor + terminal) |
| **Ambient Zones** | Invisible grid, drag a window near a zone center to snap it |
| **Window Breadcrumbs** | Ctrl+Z for window placement — undo last 3 moves |
| **Quiet Hours** | Suppress notifications from non-preset apps via KDE Do Not Disturb |
| **Cursor-Aware Tab Cycling** | Ctrl+Alt+Tab cycles the zone under the cursor, not the focused window — spatial tab switching |
| **Stage Manager** | Sidebar with live thumbnail previews of background app groups |

---

## Community & Distribution

### Launch Plan
- Post to r/kde, r/unixporn, r/linux with GIFs
- ~~AUR package for Arch/CachyOS users~~ ✅ `pave-git` on AUR
- GitHub Releases with .deb and binary
- Ko-fi / GitHub Sponsors link in README and settings UI

### When to Expand
- Hyprland support after v1.5 has traction (gap popover is the hook)
- macOS/Windows only if there's clear demand and sponsorship to justify the effort
