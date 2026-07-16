var DEFAULT_GAP = 10;
var GEOMETRY_TOLERANCE = 6;

var SIZE_STEPS = [
    { name: "quarter", ratio: 1 / 4 },
    { name: "third", ratio: 1 / 3 },
    { name: "half", ratio: 1 / 2 },
    { name: "two-thirds", ratio: 2 / 3 },
    { name: "three-quarters", ratio: 3 / 4 }
];

var STATES = {};
var STATE_SIDES = ["left", "right"];
for (var stateSideIndex = 0; stateSideIndex < STATE_SIDES.length; stateSideIndex++) {
    var stateSide = STATE_SIDES[stateSideIndex];
    for (var sizeIndex = 0; sizeIndex < SIZE_STEPS.length; sizeIndex++) {
        var size = SIZE_STEPS[sizeIndex];
        var stateName = stateSide + "-" + size.name;
        STATES[stateName] = {
            name: stateName,
            side: stateSide,
            sizeIndex: sizeIndex,
            ratio: size.ratio
        };
    }
}

var pairedWindows = {};
var resizeSessions = {};
var almostMaximizeSessions = {};

function configuredGap() {
    var value = Number(readConfig("gap", DEFAULT_GAP));
    if (!isFinite(value) || value < 0) {
        return DEFAULT_GAP;
    }
    return Math.round(value);
}

function clientId(client) {
    if (!client || !client.internalId) {
        return "";
    }
    return client.internalId.toString();
}

function isUsableWindow(client) {
    return !!client && !!client.normalWindow && !client.fullScreen &&
        !client.minimized && !client.deleted;
}

function activeClient() {
    var client = workspace.activeWindow || workspace.activeClient;
    return isUsableWindow(client) ? client : null;
}

function clientOutput(client) {
    return client.output !== undefined ? client.output : client.screen;
}

function clientScreenKey(client) {
    var output = clientOutput(client);
    if (output && output.name !== undefined) {
        return output.name.toString();
    }
    return output !== undefined && output !== null ? output.toString() : "";
}

function desktopKey(desktop) {
    if (desktop && desktop.id !== undefined) {
        return desktop.id.toString();
    }
    return desktop !== undefined && desktop !== null ? desktop.toString() : "";
}

function isOnCurrentDesktop(client) {
    if (client.onAllDesktops) {
        return true;
    }
    if (client.desktops && client.desktops.length !== undefined) {
        if (client.desktops.length === 0) {
            return true;
        }
        var currentKey = desktopKey(workspace.currentDesktop);
        for (var i = 0; i < client.desktops.length; i++) {
            if (desktopKey(client.desktops[i]) === currentKey) {
                return true;
            }
        }
        return false;
    }
    if (client.desktop !== undefined) {
        return desktopKey(client.desktop) === desktopKey(workspace.currentDesktop);
    }
    return true;
}

function sameScreenAndDesktop(a, b) {
    return !!a && !!b && clientScreenKey(a) === clientScreenKey(b) &&
        isOnCurrentDesktop(a) && isOnCurrentDesktop(b);
}

function currentWorkArea(client) {
    var screen = clientOutput(client);
    var desktop = workspace.currentDesktop;
    try {
        return workspace.clientArea(KWin.MaximizeArea, screen, desktop);
    } catch (error) {
        try {
            return workspace.clientArea(KWin.PlacementArea, screen, desktop);
        } catch (fallbackError) {
            return workspace.clientArea(0, screen, desktop);
        }
    }
}

function almostMaximizeGeometry(area, gap) {
    return {
        x: Math.round(area.x + gap),
        y: Math.round(area.y + gap),
        width: Math.max(1, Math.round(area.width - (gap * 2))),
        height: Math.max(1, Math.round(area.height - (gap * 2)))
    };
}

function splitGeometry(area, gap, side, ratio) {
    var usableWidth = Math.max(1, area.width - (gap * 3));
    var width = Math.max(1, Math.round(usableWidth * ratio));
    var x = side === "left"
        ? area.x + gap
        : area.x + area.width - gap - width;
    return {
        x: Math.round(x),
        y: Math.round(area.y + gap),
        width: width,
        height: Math.max(1, Math.round(area.height - (gap * 2)))
    };
}

function complementGeometry(area, gap, side, focusedGeometry) {
    if (side === "left") {
        var rightX = focusedGeometry.x + focusedGeometry.width + gap;
        return {
            x: Math.round(rightX),
            y: Math.round(area.y + gap),
            width: Math.max(1, Math.round(area.x + area.width - gap - rightX)),
            height: Math.max(1, Math.round(area.height - (gap * 2)))
        };
    }
    var leftX = area.x + gap;
    return {
        x: Math.round(leftX),
        y: Math.round(area.y + gap),
        width: Math.max(1, Math.round(focusedGeometry.x - gap - leftX)),
        height: Math.max(1, Math.round(area.height - (gap * 2)))
    };
}

function geometryCopy(geometry) {
    return {
        x: Math.round(geometry.x),
        y: Math.round(geometry.y),
        width: Math.round(geometry.width),
        height: Math.round(geometry.height)
    };
}

function geometrySnapshot(client) {
    return geometryCopy(client.frameGeometry);
}

function numberClose(a, b) {
    return Math.abs(a - b) <= GEOMETRY_TOLERANCE;
}

function geometryClose(actual, expected) {
    return numberClose(actual.x, expected.x) &&
        numberClose(actual.y, expected.y) &&
        numberClose(actual.width, expected.width) &&
        numberClose(actual.height, expected.height);
}

function geometriesEqual(a, b) {
    return a.x === b.x && a.y === b.y &&
        a.width === b.width && a.height === b.height;
}

function findClientById(id) {
    if (!id) {
        return null;
    }
    var candidates = workspace.stackingOrder || [];
    for (var i = 0; i < candidates.length; i++) {
        if (clientId(candidates[i]) === id) {
            return candidates[i];
        }
    }
    return null;
}

function unpairById(id) {
    if (!id) {
        return;
    }
    var pairId = pairedWindows[id];
    delete pairedWindows[id];
    if (pairId && pairedWindows[pairId] === id) {
        delete pairedWindows[pairId];
    }
}

function forgetWindow(client) {
    var id = clientId(client);
    unpairById(id);
    delete resizeSessions[id];
    delete almostMaximizeSessions[id];
}

function rememberPair(client, pair) {
    var id = clientId(client);
    var pairId = clientId(pair);
    if (!id || !pairId || id === pairId) {
        return;
    }
    unpairById(id);
    unpairById(pairId);
    pairedWindows[id] = pairId;
    pairedWindows[pairId] = id;
}

function standardStateForGeometry(area, gap, geometry) {
    for (var sideIndex = 0; sideIndex < STATE_SIDES.length; sideIndex++) {
        var side = STATE_SIDES[sideIndex];
        for (var sizeIndex = 0; sizeIndex < SIZE_STEPS.length; sizeIndex++) {
            var state = STATES[side + "-" + SIZE_STEPS[sizeIndex].name];
            if (geometryClose(
                    geometry,
                    splitGeometry(area, gap, state.side, state.ratio)
            )) {
                return state;
            }
        }
    }
    return null;
}

function horizontalSplitSide(area, gap, geometry) {
    var expectedY = area.y + gap;
    var expectedHeight = area.height - (gap * 2);
    if (!numberClose(geometry.y, expectedY) ||
            !numberClose(geometry.height, expectedHeight)) {
        return null;
    }

    var leftEdge = area.x + gap;
    var rightEdge = area.x + area.width - gap;
    var leftAnchored = numberClose(geometry.x, leftEdge);
    var rightAnchored = numberClose(geometry.x + geometry.width, rightEdge);
    if (leftAnchored === rightAnchored) {
        return null;
    }
    if (leftAnchored && geometry.x + geometry.width + gap < rightEdge) {
        return "left";
    }
    if (rightAnchored && geometry.x - gap > leftEdge) {
        return "right";
    }
    return null;
}

function existingExactPair(client, side, geometry, area, gap) {
    var id = clientId(client);
    var pairId = pairedWindows[id];
    var pair = findClientById(pairId);
    var expected = complementGeometry(area, gap, side, geometry);
    if (!isUsableWindow(pair) || pairedWindows[pairId] !== id ||
            !sameScreenAndDesktop(client, pair) ||
            !geometryClose(geometrySnapshot(pair), expected)) {
        unpairById(id);
        return null;
    }
    return pair;
}

function discoverExactPair(client, side, geometry, area, gap) {
    var candidates = workspace.stackingOrder || [];
    var id = clientId(client);
    var expected = complementGeometry(area, gap, side, geometry);
    for (var i = candidates.length - 1; i >= 0; i--) {
        var candidate = candidates[i];
        var candidateId = clientId(candidate);
        if (!isUsableWindow(candidate) || candidateId === id ||
                !sameScreenAndDesktop(client, candidate) ||
                (pairedWindows[candidateId] && pairedWindows[candidateId] !== id) ||
                !geometryClose(geometrySnapshot(candidate), expected)) {
            continue;
        }
        rememberPair(client, candidate);
        return candidate;
    }
    return null;
}

function exactPairFor(client, side, geometry, area, gap) {
    return existingExactPair(client, side, geometry, area, gap) ||
        discoverExactPair(client, side, geometry, area, gap);
}

function applyGeometry(client, geometry) {
    client.setMaximize(false, false);
    client.frameGeometry = geometry;
}

function nextState(currentState, direction, currentSide, currentRatio) {
    if (!currentState && !currentSide) {
        return STATES[direction + "-half"];
    }

    if (!currentState) {
        if (currentSide === direction) {
            for (var smallerIndex = SIZE_STEPS.length - 1;
                    smallerIndex >= 0; smallerIndex--) {
                if (SIZE_STEPS[smallerIndex].ratio < currentRatio) {
                    return STATES[
                        currentSide + "-" + SIZE_STEPS[smallerIndex].name
                    ];
                }
            }
            return null;
        }

        for (var largerIndex = 0;
                largerIndex < SIZE_STEPS.length; largerIndex++) {
            if (SIZE_STEPS[largerIndex].ratio > currentRatio) {
                return STATES[
                    currentSide + "-" + SIZE_STEPS[largerIndex].name
                ];
            }
        }
        return STATES[direction + "-quarter"];
    }

    var targetIndex = currentState.side === direction
        ? currentState.sizeIndex - 1
        : currentState.sizeIndex + 1;
    if (targetIndex < 0) {
        return null;
    }
    if (targetIndex >= SIZE_STEPS.length) {
        return STATES[direction + "-quarter"];
    }
    return STATES[
        currentState.side + "-" + SIZE_STEPS[targetIndex].name
    ];
}

function snapActiveWindow(direction) {
    var client = activeClient();
    if (!client) {
        return;
    }

    var id = clientId(client);
    var gap = configuredGap();
    var area = currentWorkArea(client);
    var originalGeometry = geometrySnapshot(client);
    var currentState = standardStateForGeometry(area, gap, originalGeometry);
    var originalSide = currentState
        ? currentState.side
        : horizontalSplitSide(area, gap, originalGeometry);
    var usableWidth = Math.max(1, area.width - (gap * 3));
    var currentRatio = originalGeometry.width / usableWidth;
    var targetState = nextState(
        currentState,
        direction,
        originalSide,
        currentRatio
    );
    if (!targetState) {
        return;
    }

    var pair = originalSide
        ? exactPairFor(client, originalSide, originalGeometry, area, gap)
        : null;
    if (!pair) {
        unpairById(id);
    }

    var targetGeometry = splitGeometry(
        area,
        gap,
        targetState.side,
        targetState.ratio
    );
    applyGeometry(client, targetGeometry);

    if (pair) {
        var pairGeometry = complementGeometry(
            area,
            gap,
            targetState.side,
            targetGeometry
        );
        applyGeometry(pair, pairGeometry);
        rememberPair(client, pair);
        return;
    }

    discoverExactPair(client, targetState.side, targetGeometry, area, gap);
}

function almostMaximizeActiveWindow() {
    var client = activeClient();
    if (!client) {
        return;
    }

    var id = clientId(client);
    var gap = configuredGap();
    var area = currentWorkArea(client);
    var currentGeometry = geometrySnapshot(client);
    var targetGeometry = almostMaximizeGeometry(area, gap);
    var screenKey = clientScreenKey(client);
    var currentDesktopKey = desktopKey(workspace.currentDesktop);
    var session = almostMaximizeSessions[id];

    if (session && session.screenKey === screenKey &&
            session.desktopKey === currentDesktopKey &&
            geometryClose(currentGeometry, targetGeometry)) {
        delete almostMaximizeSessions[id];
        applyGeometry(client, session.geometry);

        var restoredSide = horizontalSplitSide(area, gap, session.geometry);
        if (restoredSide) {
            discoverExactPair(client, restoredSide, session.geometry, area, gap);
        }
        return;
    }

    almostMaximizeSessions[id] = {
        geometry: currentGeometry,
        screenKey: screenKey,
        desktopKey: currentDesktopKey
    };
    unpairById(id);
    delete resizeSessions[id];
    applyGeometry(client, targetGeometry);
}

function startInteractiveResize(client) {
    if (!isUsableWindow(client)) {
        return;
    }
    var id = clientId(client);
    var gap = configuredGap();
    var area = currentWorkArea(client);
    var geometry = geometrySnapshot(client);
    var side = horizontalSplitSide(area, gap, geometry);
    var pair = side ? exactPairFor(client, side, geometry, area, gap) : null;
    resizeSessions[id] = {
        startGeometry: geometry,
        side: side,
        pairId: clientId(pair),
        pairStartGeometry: pair ? geometrySnapshot(pair) : null,
        changedPair: false
    };
}

function restoreSessionPair(session) {
    var pair = findClientById(session.pairId);
    if (session.changedPair && isUsableWindow(pair) && session.pairStartGeometry) {
        applyGeometry(pair, session.pairStartGeometry);
    }
}

function stepInteractiveResize(client, steppedGeometry) {
    var id = clientId(client);
    var session = resizeSessions[id];
    if (!session || !session.pairId || !session.side) {
        return;
    }
    var pair = findClientById(session.pairId);
    if (!isUsableWindow(pair) || !sameScreenAndDesktop(client, pair)) {
        unpairById(id);
        session.pairId = "";
        return;
    }

    var gap = configuredGap();
    var area = currentWorkArea(client);
    var geometry = steppedGeometry
        ? geometryCopy(steppedGeometry)
        : geometrySnapshot(client);
    if (horizontalSplitSide(area, gap, geometry) !== session.side) {
        restoreSessionPair(session);
        unpairById(id);
        session.pairId = "";
        return;
    }

    var target = complementGeometry(area, gap, session.side, geometry);
    applyGeometry(pair, target);
    session.changedPair = true;
}

function finishInteractiveResize(client) {
    var id = clientId(client);
    var session = resizeSessions[id];
    delete resizeSessions[id];
    if (!session) {
        return;
    }

    var geometry = geometrySnapshot(client);
    if (geometriesEqual(session.startGeometry, geometry)) {
        return;
    }

    var pair = findClientById(session.pairId);
    var gap = configuredGap();
    var area = currentWorkArea(client);
    if (!isUsableWindow(pair) || !session.side ||
            horizontalSplitSide(area, gap, geometry) !== session.side) {
        restoreSessionPair(session);
        forgetWindow(client);
        return;
    }

    var target = complementGeometry(area, gap, session.side, geometry);
    applyGeometry(pair, target);
    session.changedPair = true;
    rememberPair(client, pair);
}

function connectResizeSignals(client) {
    if (!isUsableWindow(client)) {
        return;
    }
    client.interactiveMoveResizeStarted.connect(function() {
        startInteractiveResize(client);
    });
    if (client.interactiveMoveResizeStepped) {
        client.interactiveMoveResizeStepped.connect(function(geometry) {
            stepInteractiveResize(client, geometry);
        });
    }
    client.interactiveMoveResizeFinished.connect(function() {
        finishInteractiveResize(client);
    });
}

var initialClients = workspace.stackingOrder || [];
for (var i = 0; i < initialClients.length; i++) {
    connectResizeSignals(initialClients[i]);
}

workspace.windowAdded.connect(function(client) {
    connectResizeSignals(client);
});

workspace.windowRemoved.connect(function(client) {
    forgetWindow(client);
});

registerShortcut("PaveSnapLeft", "Pave: Snap Left", "Ctrl+Alt+Left", function() {
    snapActiveWindow("left");
});

registerShortcut("PaveSnapRight", "Pave: Snap Right", "Ctrl+Alt+Right", function() {
    snapActiveWindow("right");
});

registerShortcut(
    "PaveAlmostMaximize",
    "Pave: Almost Maximize",
    "Ctrl+Alt+Return",
    function() {
        almostMaximizeActiveWindow();
    }
);
