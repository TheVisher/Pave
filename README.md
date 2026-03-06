# Pave

A zone-based window tiling manager for KDE Plasma. Snap, resize, and organize windows with keyboard shortcuts — with smart space detection, cooperative resizing, and tabbed zone stacking.

## Features

- **Snap spectrum** — Left/Right arrows cycle through 7 width positions (L1/3 → L1/2 → L2/3 → Full → R2/3 → R1/2 → R1/3)
- **Quarter tiling** — Up/Down splits into quarters, Left/Right cycles quarter widths
- **Smart space detection** — first snap fills the largest empty area
- **Cooperative resize** — resizing one window adjusts its neighbor
- **Tab zone stacking** — multiple windows can share a zone, cycle through them with Ctrl+Alt+Tab
- **Auto-surface** — hidden windows reappear when the covering window moves away
- **Presets** — save and restore full window layouts from the tray menu
- **Session restore** — remembers window positions across restarts
- **Monitor-aware** — arrows cross to adjacent monitors based on physical layout
- **Configurable gaps and corner radius** — with ShapeCorners integration

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Alt+Return` | Almost Maximize |
| `Ctrl+Alt+Left` | Snap Left / Shrink |
| `Ctrl+Alt+Right` | Snap Right / Grow |
| `Ctrl+Alt+Up` | Snap Up (Quarter) |
| `Ctrl+Alt+Down` | Snap Down (Quarter) |
| `Ctrl+Alt+Z` | Restore Window |
| `Ctrl+Alt+=` | Grow Window by 10% |
| `Ctrl+Alt+-` | Shrink Window by 10% |
| `Ctrl+Alt+Tab` | Cycle Tabbed Windows in Zone |

## Install

### Arch Linux (AUR)

```bash
paru -S pave-git
```

### Build from source

Requires: Rust, Node.js, npm, webkit2gtk-4.1, gtk3, libayatana-appindicator

```bash
git clone https://github.com/TheVisher/Pave.git
cd Pave
npm ci
npx tauri build -b deb
```

The binary will be at `src-tauri/target/release/pave`.

## Configuration

Settings are accessible from the system tray icon. Config is stored in `~/.config/pave/config.toml`.

- **Gap size** — padding between tiled windows (0-30px)
- **Corner radius** — window corner rounding via ShapeCorners (0-20px)
- **Auto-surface** — automatically show hidden windows when a zone opens up
- **Session restore** — restore window positions on startup
- **Autostart** — launch Pave on login

## How It Works

Pave runs as a tray application and registers global shortcuts via a KWin script. It communicates with KWin over D-Bus to move and resize windows. The KWin script is embedded in the binary and self-installs to `~/.local/share/kwin/scripts/` at runtime.

## Requirements

- KDE Plasma 6 with KWin (Wayland or X11)
- webkit2gtk-4.1, gtk3, libayatana-appindicator, dbus

## License

[MIT](LICENSE)
