# Pave: Tab Zone Manager — Design Document

## Overview

This document captures the full design for a persistent zone-based window management system with tabbed window stacking, session restore, and a novel gap-based tab popover UI. This is intended as a major feature addition to Pave.

---

## Core Concept: Zones as First-Class Citizens

Traditional tiling in Pave (and most tiling tools) is **reactive** — you open an app, it lands wherever the WM puts it, then you manually snap or tile it into position using keybinds. This is friction every single time.

The new model is **declarative**. The monitor is divided into persistent named zones that always exist. Apps are assigned to zones. When you open an app, Pave places it in its assigned zone automatically. No keybinds needed for placement.

### Zone Layout

The user defines a zone layout for each monitor — for example:
- A **2/3 left zone** and a **1/3 right zone**
- A **1/2 left** and two **1/4 stacked right zones**
- Any split configuration Pave already supports

These zones are always present. They are the foundation everything else builds on.

---

## App-to-Zone Assignment

### Floating by Default

If an app has no zone assignment, it opens as a **floating window** — normal behavior, no forced placement. Users opt into zone assignment, it is never forced.

### Snap to Assign

When a user snaps a floating window into a zone using the existing Pave keybinds, that action doubles as **assignment**. The app is now associated with that zone and will always open there going forward.

### Right-Click Context Menu on Tabs

Each tab in the tab popover (see below) has a right-click context menu with:

- **Add to Zone** — assigns the app to the current zone. It will open here from now on.
- **Pin to Zone** — assigns AND locks the app to the zone. Pinned apps cannot be dragged to other zones. Useful for anchored utilities like a monitoring terminal or music player.
- **Unassign** — removes the zone assignment, app returns to floating behavior.

---

## Tabbed Window Stacking

### The Problem It Solves

With persistent zones, multiple apps will inevitably end up assigned to the same zone — or a user opens a new app into an occupied zone. The old behavior was apps stacking on top of each other with no clean way to navigate between them. Alt-tab and the dock exist but neither hides the buried app cleanly, and they have no spatial awareness of which zone an app belongs to.

### How Tabs Work

- Each zone maintains a **stack** of windows.
- When a zone has **one app**, no tab bar or popover indicator is shown — the zone looks completely normal.
- When a zone has **two or more apps**, a tab entry is created in the zone's tab stack.
- Only one app per zone is visible at a time. The others are **hidden** — not minimized to the taskbar, but truly hidden (moved off-screen or to a hidden virtual desktop) so they don't appear as clutter anywhere.
- Switching to a tab brings that app forward and hides the previously active one.

### Hiding vs. Minimizing

Standard minimize leaves apps in the taskbar as minimized entries, which creates noise. Pave's hiding mechanism will either:
- Move the window off-screen entirely, or
- Place it on a hidden virtual desktop managed by Pave

This keeps the taskbar clean. Pave is the only interface needed to access hidden windows.

### Dragging Tabs Between Zones

A tab can be dragged from one zone's tab popover to another zone's popover. When dropped:
- The app moves to the target zone, resizing to fill that zone.
- It is added to the target zone's stack.
- If the source zone now has only one app remaining, that app becomes visible again automatically (no tab bar needed for a single app).
- If the source zone is now empty, it sits as an empty zone until another app is assigned or snapped into it.

---

## The Gap Popover — Tab UI

### Design Philosophy

Placing a tab bar at the top of a zone risks conflicting with auto-hide top panels common in custom desktop rices. Placing it at the bottom risks conflicting with taskbars. Placing it as an overlay inside the window edge works but feels cramped.

The solution: **the padding gap between zones becomes interactive UI.**

Most tiling setups include padding (gaps) between windows. This vertical gap between two side-by-side zones is dead space. Pave gives it a purpose.

### How It Works

- When you hover over the **vertical gap** between two zones, a small **popover appears** after a ~300ms delay.
- The popover is **split down the middle** — the left half shows tabs for the left zone, the right half shows tabs for the right zone.
- The popover follows the cursor's vertical position along the gap, appearing near where you hovered.
- Clicking a tab switches the active app in that zone.
- The popover closes when the mouse leaves it, or immediately after clicking a tab.

### Popover for 3+ Zones

When three zones share an intersection point (e.g., one left zone and two stacked right zones), hovering over the intersection point shows a popover split into the corresponding sections:
- One section on the left for the left zone
- Two sections on the right (top-right zone, bottom-right zone)

For a 2x2 grid with a central intersection, the popover would have four quadrants. This is spatially intuitive — the popover mirrors the layout of the zones around it. This may be complex to implement initially and can be treated as a v2 refinement.

### Popover Position Memory

The popover has a **draggable anchor** along the gap line. Users can drag it up or down to reposition it — useful if it appears over a UI element they need to access frequently. The anchor position is saved per-gap and persists across sessions.

### Hover Delay

The trigger delay is ~300ms. This is long enough that mousing across the gap to reach the adjacent zone does not accidentally trigger the popover, but short enough to feel responsive when intentionally hovering.

### Keyboard Navigation

For keyboard-first users, the popover should support tab/arrow key navigation through the tab entries once it is open, with Enter to select and Escape to close.

---

## Fullscreen Mode

### Behavior

Pressing **Alt+Ctrl+Enter** fullscreens the currently focused app over all zones — the entire monitor, edge to edge.

### Unified Tab Bar in Fullscreen

When in fullscreen, the gap popover is no longer accessible. Instead, a **unified tab strip** appears (on hover or as an always-visible option) that consolidates all tabs from all zones into a single interface. The user can switch to any app across all zones without leaving fullscreen.

Exiting fullscreen (Alt+Ctrl+Enter again, or Escape) returns the app to its zone, and all other apps restore to their previous state.

---

## Session Restore

### Concept

Pave launches at startup before any other applications. On first boot or after a clean logout it opens with zones empty. After the first session, Pave saves state on logout/shutdown and restores it on next boot — similar to how a browser reopens tabs from the last session.

### What Gets Saved

- The zone layout for each monitor
- Which apps were assigned to which zones
- Which apps were in each zone's tab stack
- Which tab was the active (visible) tab in each zone
- The popover anchor position for each gap

### Restore Behavior

On boot:
1. Pave launches first via autostart.
2. Pave reads the saved session state.
3. Each app from the previous session is launched and placed into its zone.
4. Active tabs are brought forward, others are hidden.
5. The desktop looks exactly as it did before logout.

### Conflict with App Self-Positioning

Some apps (browsers, Electron apps) attempt to restore their own window position and geometry on launch, which can fight with Pave's placement. Pave handles this by:
- Waiting for the window to fully appear, then overriding its position and size to match the assigned zone.
- Or writing KWin window rules dynamically for each assigned app, so KWin intercepts placement at the compositor level before the app settles.

---

## Technical Implementation Notes

### Zone Tracking

Pave already knows zone geometry. This system extends that to track a list of windows per zone, the active window in each zone, and hidden windows per zone.

### Hiding Windows

KWin scripts can minimize windows or move them off-screen. The preferred approach is moving hidden windows off-screen (or to a hidden virtual desktop) so they do not appear as minimized entries in the taskbar.

### Tab Popover UI

KWin scripts cannot render custom UI. The tab popover is a separate always-on-top Tauri window that Pave spawns and positions dynamically based on gap geometry and cursor position. Pave's existing Tauri frontend makes this straightforward.

### App-to-Zone Rules

Pave writes KWin window rules dynamically for each zone-assigned app. This lets KWin enforce placement at the compositor level on window open, before Pave's script layer even needs to act.

---

## Feature Summary

| Feature | Description |
|---|---|
| Persistent zones | Monitor always divided into named zones, apps assigned to zones |
| Floating fallback | Unassigned apps open floating, no forced behavior |
| Snap to assign | Snapping a window into a zone assigns it there going forward |
| Pin to zone | Locks an app to a zone, prevents dragging |
| Right-click context menu | Add to zone, pin, unassign — available on tab entries |
| Tabbed stacking | Multiple apps per zone, only one visible at a time |
| True window hiding | Hidden apps removed from taskbar, not just minimized |
| Tab drag between zones | Drag a tab from one zone to another |
| Gap popover | Hover the gap between zones to reveal a split tab popover |
| Popover position memory | Draggable anchor, position saved per gap |
| Fullscreen mode | Alt+Ctrl+Enter, unified tab strip across all zones |
| Session restore | Pave saves and restores full session state on reboot/logout |
