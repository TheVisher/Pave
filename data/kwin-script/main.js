// Pave v2 — KWin helper script
// Loaded via D-Bus by the Pave daemon.
// Handles window manipulation on Wayland where direct access isn't available.

// --- Window manipulation ---

function moveWindow(windowId, x, y, width, height) {
    const windows = workspace.stackingOrder;
    for (const w of windows) {
        if (w.internalId.toString() === windowId) {
            w.frameGeometry = {x: x, y: y, width: width, height: height};
            return true;
        }
    }
    return false;
}

function minimizeWindow(windowId) {
    const windows = workspace.stackingOrder;
    for (const w of windows) {
        if (w.internalId.toString() === windowId) {
            w.minimized = true;
            return true;
        }
    }
    return false;
}

function unminimizeWindow(windowId) {
    const windows = workspace.stackingOrder;
    for (const w of windows) {
        if (w.internalId.toString() === windowId) {
            w.minimized = false;
            return true;
        }
    }
    return false;
}

// --- Window queries ---

function getActiveWindow() {
    const w = workspace.activeWindow;
    if (!w || !w.normalWindow) return null;
    return {
        id: w.internalId.toString(),
        appClass: w.resourceClass.toString(),
        x: w.frameGeometry.x,
        y: w.frameGeometry.y,
        width: w.frameGeometry.width,
        height: w.frameGeometry.height,
        minimized: w.minimized,
        fullScreen: w.fullScreen,
        desktop: w.desktops.length > 0 ? w.desktops[0].id : 0,
        screen: w.output ? w.output.name : ""
    };
}

function getWindows() {
    const result = [];
    const windows = workspace.stackingOrder;
    for (const w of windows) {
        if (!w.normalWindow) continue;
        result.push({
            id: w.internalId.toString(),
            appClass: w.resourceClass.toString(),
            x: w.frameGeometry.x,
            y: w.frameGeometry.y,
            width: w.frameGeometry.width,
            height: w.frameGeometry.height,
            minimized: w.minimized,
            fullScreen: w.fullScreen,
            desktop: w.desktops.length > 0 ? w.desktops[0].id : 0,
            screen: w.output ? w.output.name : ""
        });
    }
    return result;
}

// --- Event forwarding ---

// Forward window activation to the daemon via D-Bus
workspace.windowActivated.connect(function(w) {
    if (!w || !w.normalWindow) return;
    if (w.fullScreen) return;

    callDBus(
        "org.kde.pave",
        "/Daemon",
        "org.kde.pave.Daemon",
        "windowActivated",
        JSON.stringify({
            id: w.internalId.toString(),
            appClass: w.resourceClass.toString(),
            x: w.frameGeometry.x,
            y: w.frameGeometry.y,
            width: w.frameGeometry.width,
            height: w.frameGeometry.height,
            screen: w.output ? w.output.name : ""
        })
    );
});

// Forward window removal
workspace.windowRemoved.connect(function(w) {
    if (!w) return;
    callDBus(
        "org.kde.pave",
        "/Daemon",
        "org.kde.pave.Daemon",
        "windowClosed",
        w.internalId.toString()
    );
});
