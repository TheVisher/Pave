# Pave — Zone-Based Window Tiler for KDE Plasma

## What is Pave

Pave is a zone-based window tiling manager for KDE Plasma 6. The core idea: **zones are the only primitive.** Windows don't have their own geometry — they inherit it from the zone they belong to. Resizing moves split points, and every window in every affected zone follows.

## Architecture (v2 — current branch: v2-kde-native)

**Tech stack:** C++20 / Qt 6.6+ / KDE Frameworks 6

**How it works:**
- A native C++ daemon runs as a systemd user service
- Global shortcuts registered via **KGlobalAccel** (appear in KDE System Settings)
- Window manipulation done via **KWin scripting over D-Bus** (only way to manage windows on Wayland)
- The daemon loads a KWin JS script (`data/kwin-script/main.js`) that handles moveWindow, minimize, unminimize, and forwards window events back to the daemon via D-Bus

**Data flow:** User shortcut → KGlobalAccel → PaveDaemon slot → recompute zone rects → send D-Bus commands → KWin script moves windows

### Project structure (v2)

```
src/
  main.cpp              Entry point, KDBusService single-instance
  daemon.h/cpp          Core daemon: shortcuts, zone state, window lifecycle
  zonelayout.h/cpp      Zone math: split ratios, rect computation, adjacency
  windowmanager.h/cpp   KWin D-Bus wrapper: load script, move/minimize windows
data/
  kwin-script/main.js   KWin helper script for window manipulation
  pave.service          Systemd user service file
docs/
  v2-architecture.md    Full design doc for v2 rewrite
  roadmap.md            Original v1 roadmap (historical reference)
CMakeLists.txt          Build config
```

### v1 code (branch: main, preserved in src-tauri/)

The original Rust/Tauri implementation lives in `src-tauri/`. It works but has architectural issues — see "What went wrong with v1" below.

## What went wrong with v1

v1 had **7 interacting state stores** that needed to stay in sync: last_action, pre_snap_geometry, pre_maximize_geometry, pre_maximize_layout, zone_tracker, zone_last_geometry, zone_layouts. When any one got stale, bugs appeared.

The root cause: **zones were derived from actions instead of being the source of truth.** The flow was: user presses shortcut → compute geometry → figure out what zone that is → update 4+ state stores. This made tiling.rs grow to 2,800 lines of edge-case handling.

**v2 fixes this with 2 core state stores:**
1. `ZoneLayout` per monitor per desktop — split ratios (source of truth)
2. `ZoneAssignment` — maps app class → zone ID (persisted)
3. `PreSnapGeometry` — original floating geometry for restore (convenience)

Zone rects are always computed fresh from ratios + monitor geometry + gap. Never cached, never stale.

## Key concepts

- **Convergence point** — where split lines meet. Keyboard shortcuts move it (no window focus needed). Also a mouse drag handle for fine control. Future: UI popup anchor for zone overview.
- **Snap past the edge** — stepping the ratio past the last step collapses the shrinking zone. The window hides. Step back and it reappears. Non-destructive.
- **Adaptive layout** — zones activate/deactivate based on which assigned apps are running. 1 app = almost maximize. 2+ apps = split. Empty zones expand to fill.
- **Window stacking** — multiple apps can be assigned to one zone. Only the most recently focused one is visible; others are minimized (for transparency support). Alt+Tab handles switching.
- **Per-virtual-desktop** — each desktop has its own zone layout and assignments. Desktops can be "unmanaged" (Pave ignores them) for fullscreen/games.

## Build (v2)

```bash
mkdir build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Debug
make
```

**Dependencies:** qt6-base, extra-cmake-modules, kf6-kglobalaccel, kf6-kdbusaddons, kf6-kconfig, kf6-kcoreaddons, kf6-kcrash

## Conventions

- Use `QStringLiteral()` for all string literals (avoids runtime allocation)
- Use `QLatin1String()` for comparisons
- Prefer `QHash` over `std::unordered_map`, `QVector` over `std::vector`
- Zone IDs are strings: "L", "R", "L.T", "L.B", "R.T", "R.B", "root"
- Monitor targeting uses cursor position, not focused window
- KWin script uses `workspace.stackingOrder` (KWin 6 API, not `clientList()`)
