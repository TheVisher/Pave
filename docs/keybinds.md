# Pave Keybinds

## Modifier

**Default:** `Super+Alt`

- Ergonomic: both keys hit with one thumb on most keyboards
- Conflict-free on Linux (KDE), macOS, and Windows defaults
- `Super` and `Alt` are adjacent on both PC and Apple layouts (order is swapped on Apple, but same two keys)

### History: Why Ctrl+Alt Originally

The original `Ctrl+Alt` modifier was chosen to match Rectangle (macOS window manager), which uses `Ctrl+Option` by default. Rectangle is fully remappable, so when Pave migrates to `Super+Alt`, Rectangle should be rebound to `Cmd+Option` to keep muscle memory consistent across Linux and Mac.

### Future: Hyper / Caps Lock Remap Option

For power users, support remapping Caps Lock to `Hyper` (a modifier nothing else uses) and using it as a single-key Pave modifier. This would be the cleanest ergonomic option but requires user setup:
- Linux: `setxkbmap` or KDE input settings
- macOS: System Settings > Keyboard > Modifier Keys
- Windows: Registry edit or PowerToys

This should be an opt-in setting, not the default. The app should allow users to configure which modifier(s) Pave uses.

---

## Current Shortcuts

All shortcuts currently use `Ctrl+Alt` — migration to `Super+Alt` is planned.

| Shortcut | Action | Description |
|----------|--------|-------------|
| `Ctrl+Alt+Return` | Maximize | Almost-maximize with gap (15px) |
| `Ctrl+Alt+Left` | Snap Left | Tile window to left zone |
| `Ctrl+Alt+Right` | Snap Right | Tile window to right zone |
| `Ctrl+Alt+Up` | Snap Up | Tile to top quarter |
| `Ctrl+Alt+Down` | Snap Down | Tile to bottom quarter |
| `Ctrl+Alt+Z` | Restore | Restore window to pre-snap size |
| `Ctrl+Alt+=` | Grow | Increase window size by 10% |
| `Ctrl+Alt+-` | Shrink | Decrease window size by 10% |
| `Ctrl+Alt+Tab` | Tab Cycle | Cycle through stacked windows in zone |

## Planned Shortcuts (Super+Alt)

### Resize — `Super+Alt+arrows`

Resizes zones cooperatively — neighbors adjust to fill remaining space. No overlap, ever.

| Shortcut | Action | Description |
|----------|--------|-------------|
| `Super+Alt+Left` | Resize Left | Shrink zone from right edge (neighbor grows) |
| `Super+Alt+Right` | Resize Right | Grow zone to the right (neighbor shrinks) |
| `Super+Alt+Up` | Split/Resize Up | Split zone vertically or resize height |
| `Super+Alt+Down` | Split/Resize Down | Split zone vertically or resize height |

Mouse drag on window edges is available for freeform resizing outside the shortcut system.

### Snap to Zone — `Super+Alt+Shift+arrows`

Instantly moves the focused window into the adjacent zone. The displaced window in that zone gets minimized and joins the zone's tab cycle stack.

| Shortcut | Action | Description |
|----------|--------|-------------|
| `Super+Alt+Shift+Left` | Snap to Left Zone | Jump window into the zone to the left |
| `Super+Alt+Shift+Right` | Snap to Right Zone | Jump window into the zone to the right |
| `Super+Alt+Shift+Up` | Snap to Upper Zone | Jump window into the zone above |
| `Super+Alt+Shift+Down` | Snap to Lower Zone | Jump window into the zone below |

### Other

| Shortcut | Action | Description |
|----------|--------|-------------|
| `Super+Alt+Return` | Maximize | Almost-maximize / maximize cycle |
| `Super+Alt+Z` | Restore | Restore window to pre-snap size |
| `Super+Alt+=` | Grow | Increase window size by 10% (cooperative) |
| `Super+Alt+-` | Shrink | Decrease window size by 10% (cooperative) |
| `Super+Alt+Tab` | Tab Cycle | Cycle stacked windows in current zone |
| `Super+Alt+1/2/3...` | Focus Zone | Jump directly to zone 1, 2, 3, etc. |
| `Super+Alt+O` | Overview | Open zone overview (show all zones + contents) |

---

## Zone Behavior

### Dynamic Zone Topology

Zones are not a static grid — they're defined by the current window arrangement. Splitting a 1/3 zone horizontally creates a third zone. Snapping back down collapses it. Pave tracks the live topology at all times.

### New Window Placement

When a new window opens:
1. If the app has a zone assignment, place it in that zone at the zone's **current** dimensions (not a saved default)
2. If no assignment, place it in the currently focused zone
3. Other zones are left untouched — empty zones stay empty until the user explicitly fills them

### Cooperative Resize Rules

- `Super+Alt+Left/Right` adjusts zone width — the neighboring zone grows or shrinks to compensate
- `Super+Alt+Up/Down` splits or resizes vertically — this can create new zones
- No window ever overlaps another. If a resize would cause overlap, the neighbor adjusts
- Mouse drag on edges is the escape hatch for freeform sizing outside the zone system

---

## Known Conflicts

### Linux (KDE Plasma)
| Shortcut | Conflict | Notes |
|----------|----------|-------|
| `Super+Alt+*` | **None known** | KDE doesn't bind Super+Alt combos by default |

### macOS
| Shortcut | Conflict | Notes |
|----------|----------|-------|
| `Cmd+Option+*` | **Minimal** | Some apps use Cmd+Option combos (e.g., Safari Cmd+Option+1 for bookmarks bar, Cmd+Option+Esc for force quit). Most arrow/number combos are free. |
| `Cmd+Option+Esc` | Force Quit dialog | Avoid binding Esc |

### Windows
| Shortcut | Conflict | Notes |
|----------|----------|-------|
| `Win+Alt+*` | **Some conflicts** | Win+Alt+R starts/stops Xbox Game Bar recording. Win+Alt+PrtSc takes a screenshot. Arrow and number combos appear free. |
| `Win+Alt+R` | Xbox Game Bar record | Avoid binding R |
| `Win+Alt+PrtSc` | Xbox Game Bar screenshot | Avoid binding PrtSc |

### Keys to Avoid (cross-platform)
- `Esc` — macOS Force Quit
- `R` — Windows Game Bar recording
- `PrtSc` — Windows Game Bar screenshot

---

## Design Principles

1. **One modifier to rule them all** — every Pave shortcut uses `Super+Alt` as the base modifier
2. **Shift = zone jump** — adding `Shift` upgrades a resize action into a zone-snap action
3. **Spatial consistency** — arrows always mean direction, numbers always mean zone index
4. **Two intents, two combos** — `Super+Alt+arrows` resizes cooperatively (neighbors adjust), `Super+Alt+Shift+arrows` snaps to zone (displaced window joins tab stack)
5. **No conflicts with common app shortcuts** — `Ctrl+C/V/Z/S` etc. stay untouched
6. **Remappable** — users should eventually be able to change the modifier in settings
