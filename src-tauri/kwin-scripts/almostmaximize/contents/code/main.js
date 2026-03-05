// Pave Window Tiling - KWin Shortcut Relay + Resize Tracking
// Relays keyboard shortcuts to Pave's D-Bus service.
// Uses the original shortcut names (AlmostMaximize, SnapLeft, SnapRight)
// because they already have working Wayland key grabs.
// Also tracks interactive resize events and relays them to Pave.

registerShortcut(
    "AlmostMaximize",
    "Almost Maximize Window",
    "Ctrl+Alt+Return",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_maximize");
    }
);

registerShortcut(
    "SnapLeft",
    "Snap Window Left with Gap",
    "Ctrl+Alt+Left",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_snap_left");
    }
);

registerShortcut(
    "SnapRight",
    "Snap Window Right with Gap",
    "Ctrl+Alt+Right",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_snap_right");
    }
);

registerShortcut(
    "SnapUp",
    "Snap Window Up (Quarter)",
    "Ctrl+Alt+Up",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_snap_up");
    }
);

registerShortcut(
    "SnapDown",
    "Snap Window Down (Quarter)",
    "Ctrl+Alt+Down",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_snap_down");
    }
);

registerShortcut(
    "RestoreWindow",
    "Restore Window to Pre-Snap Size",
    "Ctrl+Alt+Z",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_restore");
    }
);

registerShortcut(
    "GrowWindow",
    "Grow Window by 10%",
    "Ctrl+Alt+=",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_grow");
    }
);

registerShortcut(
    "ShrinkWindow",
    "Shrink Window by 10%",
    "Ctrl+Alt+-",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_shrink");
    }
);

registerShortcut(
    "TabCycle",
    "Cycle Tabbed Windows in Zone",
    "Ctrl+Alt+Tab",
    function() {
        callDBus("com.pave.app", "/com/pave/Shortcuts",
                 "com.pave.Shortcuts", "ShortcutPressed", "pave_tab_cycle");
    }
);

// --- Resize tracking ---
// Track pre-resize geometry so we can detect which edge moved and notify Pave.

var preResizeGeometry = {};

function connectResizeSignals(client) {
    if (!client || !client.normalWindow) return;

    client.interactiveMoveResizeStarted.connect(function() {
        var g = client.frameGeometry;
        preResizeGeometry[client.internalId.toString()] = {
            x: Math.round(g.x),
            y: Math.round(g.y),
            width: Math.round(g.width),
            height: Math.round(g.height)
        };
    });

    client.interactiveMoveResizeFinished.connect(function() {
        var id = client.internalId.toString();
        var oldGeom = preResizeGeometry[id];
        if (!oldGeom) return;
        delete preResizeGeometry[id];

        var g = client.frameGeometry;
        var newGeom = {
            x: Math.round(g.x),
            y: Math.round(g.y),
            width: Math.round(g.width),
            height: Math.round(g.height)
        };

        // Skip if geometry unchanged (cancelled drag)
        if (oldGeom.x === newGeom.x && oldGeom.y === newGeom.y &&
            oldGeom.width === newGeom.width && oldGeom.height === newGeom.height) {
            return;
        }

        // Skip if only position changed (move, not resize)
        if (oldGeom.width === newGeom.width && oldGeom.height === newGeom.height) {
            return;
        }

        var payload = JSON.stringify({
            windowId: id,
            screen: client.output ? client.output.name : "",
            oldGeometry: oldGeom,
            newGeometry: newGeom
        });

        callDBus("com.pave.app", "/com/pave/ResizeEvents",
                 "com.pave.ResizeEvents", "WindowResized", payload);
    });
}

// Connect to all existing windows
var clients = workspace.stackingOrder;
for (var i = 0; i < clients.length; i++) {
    connectResizeSignals(clients[i]);
}

// Connect to newly added windows
workspace.windowAdded.connect(function(client) {
    connectResizeSignals(client);
});
