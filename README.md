# Pave

Pave is a small KDE Wayland window snapping setup modeled after the parts of
Rectangle that are useful on an ultrawide monitor. It is intentionally minimal:
padded left/right splits, directional split navigation, almost maximize, and
synchronized paired resizing.

## Current Behavior

- `Ctrl+Alt+Left`: snap focused window to the left side.
- `Ctrl+Alt+Right`: snap focused window to the right side.
- `Ctrl+Alt+Enter`: toggle the focused window between almost maximize and its
  previous geometry.
- From a floating window, a direction snaps to that side at `1/2`. Repeated
  presses toward the same edge shrink through `1/3 -> 1/4`, then stop.
- Pressing the opposite direction expands through the ordered split sizes
  `1/4 -> 1/3 -> 1/2 -> 2/3 -> 3/4`, then crosses to `1/4` on the other side.
- After a manual split resize, the arrows resume from the next smaller or larger
  preset on the window's current side instead of treating it as floating.
- Almost maximize remains separate from the directional split cycle. Pave
  remembers each window's starting geometry, so a second press restores its
  previous split or floating position.
- Pave recognizes a pair only when another window occupies the exact,
  non-overlapping complement on the same screen and desktop. It never selects a
  background window based only on overlap.
- Pair recognition comes from current geometry, so it works after login or a
  script reload without needing temporary state from an earlier shortcut.
- Once paired, navigating or horizontally resizing either window adjusts the
  verified partner.
- Moving a window manually, changing its side, almost-maximizing it, closing it,
  or moving it to another screen/desktop breaks the pair symmetrically.
- When a paired split window is resized manually, the opposite window follows
  during the drag and is reconciled once more when the drag ends.
- Default padding is `10 px`.

## Installed Files

The KWin script source lives in this project:

```text
~/Projects/pave/kwin/metadata.json
~/Projects/pave/kwin/contents/code/main.js
```

The settings launcher copies that source into KDE's script directory:

```text
~/.local/share/kwin/scripts/pave/metadata.json
~/.local/share/kwin/scripts/pave/contents/code/main.js
```

The settings app lives here:

```text
~/Projects/pave/pave-settings.py
~/.local/bin/pave-settings
~/.local/share/applications/pave.desktop
~/.local/share/applications/pave-autostart.desktop
~/.config/autostart/pave.desktop
```

`~/.local/bin/pave-settings` is just a launcher for the Python settings app.
The desktop entry makes Pave appear in the KDE launcher. The autostart entry
runs `pave-settings --apply` so login silently enables the script and refreshes
shortcuts without opening the settings window.

## Settings App

Open it from the KDE launcher as `Pave`, or run:

```bash
pave-settings
```

Useful actions:

- `Apply`: saves padding, writes the autostart toggle, reloads the KWin script,
  and refreshes shortcuts.
- `Reload Script`: reloads the KWin script after edits.
- `Fix Shortcuts`: rewrites the KDE shortcut config and calls KGlobalAccel over
  DBus so the shortcuts work immediately in the current session.

Silent apply mode:

```bash
pave-settings --apply
```

## KDE Config

KWin enablement and padding are stored in `~/.config/kwinrc`:

```ini
[Plugins]
paveEnabled=true

[Script-pave]
gap=10
```

Global shortcuts are stored in `~/.config/kglobalshortcutsrc` under the `kwin`
group:

```ini
PaveSnapLeft=Ctrl+Alt+Left,none,Pave: Snap Left
PaveSnapRight=Ctrl+Alt+Right,none,Pave: Snap Right
PaveAlmostMaximize=Ctrl+Alt+Return,none,Pave: Almost Maximize
PaveSnapTop=none,none,Pave: Snap Top
PaveSnapBottom=none,none,Pave: Snap Bottom
```

The live shortcut repair uses `org.kde.kglobalaccel` with these keycodes:

- `Ctrl+Alt+Left`: `218103826`
- `Ctrl+Alt+Right`: `218103828`
- `Ctrl+Alt+Return`: `218103812`
- `Ctrl+Alt+Enter`: `218103813`

## Development Notes

KWin JavaScript uses `workspace.activeWindow`, `workspace.stackingOrder`,
`client.frameGeometry`, and `registerShortcut`. Pave unmaximizes windows before
setting geometry, otherwise KDE may keep square maximized corners or ignore some
geometry changes.

The pairing logic is intentionally conservative:

1. Read the focused window's current geometry to determine its navigation
   state; no remembered cycle index is required.
2. Before moving it, look only for an exact complementary window on the same
   screen and desktop.
3. Move that verified partner to the new complement as the focused window
   navigates or resizes.
4. Break both sides of a pair whenever either window leaves that model.

Known limitations:

- Only left/right splits are active. Top/bottom shortcuts are deliberately
  disabled because they are not useful for the current ultrawide workflow.
- Some client-side decorated apps may draw square corners. KDE/Breeze decorated
  apps should follow the active decoration settings.

## Tests

The Node test harness mocks the relevant KWin API and verifies geometry and
pair lifecycle behavior:

```bash
node --test tests/pave.test.js
```

## Quick Checks

```bash
qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.isScriptLoaded pave
kreadconfig6 --file ~/.config/kwinrc --group Script-pave --key gap
kreadconfig6 --file ~/.config/kglobalshortcutsrc --group kwin --key PaveSnapLeft
```

If shortcuts stop responding:

```bash
pave-settings --apply
```
