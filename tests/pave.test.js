const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const SCRIPT = fs.readFileSync(
    path.join(__dirname, "..", "kwin", "contents", "code", "main.js"),
    "utf8"
);

function signal() {
    const handlers = [];
    return {
        connect(handler) {
            handlers.push(handler);
        },
        emit(...args) {
            for (const handler of [...handlers]) {
                handler(...args);
            }
        },
    };
}

function geometry(x, y, width, height) {
    return { x, y, width, height };
}

function window(id, frame, options = {}) {
    let currentGeometry = { ...frame };
    const client = {
        internalId: { toString: () => id },
        normalWindow: true,
        fullScreen: false,
        minimized: false,
        deleted: false,
        screen: options.screen ?? 0,
        output: options.output,
        desktops: options.desktops ?? ["desktop-1"],
        onAllDesktops: options.onAllDesktops ?? false,
        interactiveMoveResizeStarted: signal(),
        interactiveMoveResizeStepped: signal(),
        interactiveMoveResizeFinished: signal(),
        maximizeCalls: 0,
        geometryWrites: 0,
        geometryRequests: [],
        setMaximize() {
            this.maximizeCalls += 1;
        },
    };

    Object.defineProperty(client, "frameGeometry", {
        get() {
            return { ...currentGeometry };
        },
        set(value) {
            const transformed = { ...client.geometryTransform(value) };
            client.geometryRequests.push(transformed);
            if (!options.deferGeometryWrites) {
                currentGeometry = transformed;
            }
            client.geometryWrites += 1;
        },
    });

    client.geometryTransform = (value) => value;
    client.setGeometryFromUser = (value) => {
        currentGeometry = { ...value };
    };
    client.flushGeometryWrite = () => {
        if (client.geometryRequests.length > 0) {
            currentGeometry = { ...client.geometryRequests.at(-1) };
        }
    };
    return client;
}

function harness(windows, options = {}) {
    const shortcuts = {};
    const workspace = {
        activeWindow: windows[0] ?? null,
        activeClient: null,
        currentDesktop: options.currentDesktop ?? "desktop-1",
        stackingOrder: windows,
        windowAdded: signal(),
        windowRemoved: signal(),
        clientArea() {
            return geometry(0, 0, 1200, 800);
        },
    };
    const context = {
        console,
        isFinite,
        KWin: { MaximizeArea: 1, PlacementArea: 2 },
        readConfig: () => 10,
        registerShortcut(name, description, keys, callback) {
            shortcuts[name] = callback;
        },
        workspace,
    };

    vm.createContext(context);
    vm.runInContext(SCRIPT, context, { filename: "main.js" });
    return {
        workspace,
        press(name, active) {
            workspace.activeWindow = active;
            shortcuts[name]();
        },
    };
}

const LEFT_QUARTER = geometry(10, 10, 293, 780);
const LEFT_THIRD = geometry(10, 10, 390, 780);
const LEFT_HALF = geometry(10, 10, 585, 780);
const LEFT_TWO_THIRDS = geometry(10, 10, 780, 780);
const LEFT_THREE_QUARTERS = geometry(10, 10, 878, 780);
const RIGHT_THREE_QUARTERS = geometry(312, 10, 878, 780);
const RIGHT_TWO_THIRDS = geometry(410, 10, 780, 780);
const RIGHT_HALF = geometry(605, 10, 585, 780);
const RIGHT_THIRD = geometry(800, 10, 390, 780);
const RIGHT_QUARTER = geometry(897, 10, 293, 780);
const ALMOST_MAXIMIZED = geometry(10, 10, 1180, 780);

test("right shrinks a floating window from half to quarter, then stops", () => {
    const active = window("active", geometry(250, 100, 700, 600));
    const app = harness([active]);

    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, RIGHT_HALF);

    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, RIGHT_THIRD);

    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, RIGHT_QUARTER);

    const writes = active.geometryWrites;
    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, RIGHT_QUARTER);
    assert.equal(active.geometryWrites, writes);
});

test("left progresses back through right-side sizes before crossing", () => {
    const active = window("active", RIGHT_QUARTER);
    const app = harness([active]);

    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, RIGHT_THIRD);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, RIGHT_HALF);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, RIGHT_TWO_THIRDS);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, RIGHT_THREE_QUARTERS);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, LEFT_QUARTER);
});

test("right mirrors the complete sequence from the left side", () => {
    const active = window("active", LEFT_QUARTER);
    const app = harness([active]);

    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, LEFT_THIRD);
    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, LEFT_HALF);
    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, LEFT_TWO_THIRDS);
    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, LEFT_THREE_QUARTERS);
    app.press("PaveSnapRight", active);
    assert.deepEqual(active.frameGeometry, RIGHT_QUARTER);
});

test("left stops at left one-quarter", () => {
    const active = window("active", geometry(250, 100, 700, 600));
    const app = harness([active]);

    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, LEFT_HALF);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, LEFT_THIRD);
    app.press("PaveSnapLeft", active);
    assert.deepEqual(active.frameGeometry, LEFT_QUARTER);

    const writes = active.geometryWrites;
    app.press("PaveSnapLeft", active);
    assert.equal(active.geometryWrites, writes);
});

test("a fresh script instance reconstructs an exact pair before a shortcut", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);

    app.press("PaveSnapRight", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
});

test("a partner uses the requested target when KWin defers focused geometry", () => {
    const left = window("left", LEFT_TWO_THIRDS);
    const right = window("right", RIGHT_THIRD, { deferGeometryWrites: true });
    const app = harness([left, right]);

    app.press("PaveSnapLeft", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.deepEqual(right.geometryRequests.at(-1), RIGHT_HALF);
    assert.deepEqual(left.frameGeometry, LEFT_HALF);

    right.flushGeometryWrite();
    assert.deepEqual(right.frameGeometry, RIGHT_HALF);
});

test("pressing outward at one-quarter does not write either paired window", () => {
    const left = window("left", LEFT_THREE_QUARTERS);
    const right = window("right", RIGHT_QUARTER);
    const app = harness([left, right]);
    const leftWrites = left.geometryWrites;
    const rightWrites = right.geometryWrites;

    app.press("PaveSnapRight", right);

    assert.equal(left.geometryWrites, leftWrites);
    assert.equal(right.geometryWrites, rightWrites);
});

test("crossing to the opposite side moves the verified partner with it", () => {
    const left = window("left", LEFT_QUARTER);
    const right = window("right", RIGHT_THREE_QUARTERS);
    const app = harness([left, right]);

    app.press("PaveSnapLeft", right);

    assert.deepEqual(right.frameGeometry, LEFT_QUARTER);
    assert.deepEqual(left.frameGeometry, geometry(313, 10, 877, 780));
});

test("independently snapped halves become a pair", () => {
    const left = window("left", geometry(40, 40, 500, 600));
    const right = window("right", geometry(500, 80, 650, 620));
    const app = harness([left, right]);

    app.press("PaveSnapLeft", left);
    const leftWrites = left.geometryWrites;
    app.press("PaveSnapRight", right);
    assert.equal(left.geometryWrites, leftWrites);

    app.press("PaveSnapRight", right);
    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
});

test("an unrelated overlapping background window is never resized", () => {
    const active = window("active", geometry(100, 80, 900, 650));
    const background = window("background", geometry(450, 100, 700, 620));
    const app = harness([background, active]);
    const before = background.frameGeometry;

    app.press("PaveSnapLeft", active);

    assert.deepEqual(active.frameGeometry, LEFT_HALF);
    assert.deepEqual(background.frameGeometry, before);
    assert.equal(background.geometryWrites, 0);
});

test("a manual horizontal resize updates a reconstructed pair while dragging", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);
    const resized = geometry(10, 10, 700, 780);

    left.interactiveMoveResizeStarted.emit();
    left.setGeometryFromUser(resized);
    left.interactiveMoveResizeStepped.emit(resized);

    assert.deepEqual(right.frameGeometry, geometry(720, 10, 470, 780));

    left.interactiveMoveResizeFinished.emit();
    assert.deepEqual(left.frameGeometry, resized);
    assert.deepEqual(right.frameGeometry, geometry(720, 10, 470, 780));

    app.press("PaveSnapRight", left);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
});

test("custom split widths grow through presets without changing sides", () => {
    const customLeft = window("custom-left", geometry(10, 10, 700, 780));
    const customRight = window("custom-right", geometry(490, 10, 700, 780));
    const app = harness([customLeft, customRight]);

    app.press("PaveSnapRight", customLeft);
    assert.deepEqual(customLeft.frameGeometry, LEFT_TWO_THIRDS);
    app.press("PaveSnapRight", customLeft);
    assert.deepEqual(customLeft.frameGeometry, LEFT_THREE_QUARTERS);

    app.press("PaveSnapLeft", customRight);
    assert.deepEqual(customRight.frameGeometry, RIGHT_TWO_THIRDS);
    app.press("PaveSnapLeft", customRight);
    assert.deepEqual(customRight.frameGeometry, RIGHT_THREE_QUARTERS);
});

test("custom split widths shrink through presets without changing sides", () => {
    const customLeft = window("custom-left", geometry(10, 10, 700, 780));
    const customRight = window("custom-right", geometry(490, 10, 700, 780));
    const app = harness([customLeft, customRight]);

    app.press("PaveSnapLeft", customLeft);
    assert.deepEqual(customLeft.frameGeometry, LEFT_HALF);

    app.press("PaveSnapRight", customRight);
    assert.deepEqual(customRight.frameGeometry, RIGHT_HALF);
});

test("drag finish synchronizes a pair even when no stepped signal arrives", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    harness([left, right]);

    left.interactiveMoveResizeStarted.emit();
    left.setGeometryFromUser(geometry(10, 10, 700, 780));
    left.interactiveMoveResizeFinished.emit();

    assert.deepEqual(right.frameGeometry, geometry(720, 10, 470, 780));
});

test("a right-side drag updates its left partner while dragging", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    harness([left, right]);
    const resized = geometry(700, 10, 490, 780);

    right.interactiveMoveResizeStarted.emit();
    right.setGeometryFromUser(resized);
    right.interactiveMoveResizeStepped.emit(resized);

    assert.deepEqual(left.frameGeometry, geometry(10, 10, 680, 780));
    right.interactiveMoveResizeFinished.emit();
    assert.deepEqual(left.frameGeometry, geometry(10, 10, 680, 780));
});

test("leaving a horizontal split restores a partner changed earlier in the drag", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    harness([left, right]);

    left.interactiveMoveResizeStarted.emit();
    left.setGeometryFromUser(geometry(10, 10, 700, 780));
    left.interactiveMoveResizeStepped.emit(left.frameGeometry);
    assert.deepEqual(right.frameGeometry, geometry(720, 10, 470, 780));

    left.setGeometryFromUser(geometry(100, 80, 700, 700));
    left.interactiveMoveResizeStepped.emit(left.frameGeometry);
    left.interactiveMoveResizeFinished.emit();

    assert.deepEqual(right.frameGeometry, RIGHT_HALF);
});

test("almost maximize clears a pair", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);

    app.press("PaveSnapRight", right);
    app.press("PaveAlmostMaximize", left);
    const leftAfterMaximize = left.frameGeometry;

    right.interactiveMoveResizeStarted.emit();
    right.setGeometryFromUser(geometry(700, 10, 490, 780));
    right.interactiveMoveResizeFinished.emit();

    assert.deepEqual(left.frameGeometry, leftAfterMaximize);
});

test("almost maximize toggles a split window back to its previous geometry", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);

    app.press("PaveAlmostMaximize", left);
    assert.deepEqual(left.frameGeometry, ALMOST_MAXIMIZED);
    assert.deepEqual(right.frameGeometry, RIGHT_HALF);

    app.press("PaveAlmostMaximize", left);
    assert.deepEqual(left.frameGeometry, LEFT_HALF);
    assert.deepEqual(right.frameGeometry, RIGHT_HALF);

    app.press("PaveSnapRight", left);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
});

test("manual geometry becomes the new restore point after almost maximize", () => {
    const original = geometry(240, 100, 720, 600);
    const moved = geometry(120, 80, 800, 650);
    const active = window("active", original);
    const app = harness([active]);

    app.press("PaveAlmostMaximize", active);
    assert.deepEqual(active.frameGeometry, ALMOST_MAXIMIZED);

    active.setGeometryFromUser(moved);
    app.press("PaveAlmostMaximize", active);
    assert.deepEqual(active.frameGeometry, ALMOST_MAXIMIZED);

    app.press("PaveAlmostMaximize", active);
    assert.deepEqual(active.frameGeometry, moved);
});

test("closing a window clears its almost maximize restore point", () => {
    const original = geometry(240, 100, 720, 600);
    const oldWindow = window("reused-id", original);
    const app = harness([oldWindow]);

    app.press("PaveAlmostMaximize", oldWindow);
    app.workspace.windowRemoved.emit(oldWindow);

    const newWindow = window("reused-id", ALMOST_MAXIMIZED);
    app.workspace.stackingOrder = [newWindow];
    app.press("PaveAlmostMaximize", newWindow);

    assert.deepEqual(newWindow.frameGeometry, ALMOST_MAXIMIZED);
});

test("a client that rejects complementary geometry is safely unpaired", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);
    left.geometryTransform = (requested) => ({
        ...requested,
        width: requested.width - 40,
    });

    app.press("PaveSnapRight", right);
    const leftAfterRejectedResize = left.frameGeometry;

    right.interactiveMoveResizeStarted.emit();
    right.setGeometryFromUser(geometry(700, 10, 490, 780));
    right.interactiveMoveResizeFinished.emit();

    assert.deepEqual(left.frameGeometry, leftAfterRejectedResize);
});

test("removing one window clears its partner relationship", () => {
    const left = window("left", LEFT_HALF);
    const right = window("right", RIGHT_HALF);
    const app = harness([left, right]);
    app.press("PaveSnapRight", right);
    app.workspace.windowRemoved.emit(left);
    app.workspace.stackingOrder = [right];
    const leftWrites = left.geometryWrites;

    right.interactiveMoveResizeStarted.emit();
    right.setGeometryFromUser(geometry(700, 10, 490, 780));
    right.interactiveMoveResizeFinished.emit();

    assert.equal(left.geometryWrites, leftWrites);
});

test("exact split windows on different screens never pair", () => {
    const otherScreen = window("other-screen", LEFT_HALF, { screen: 1 });
    const active = window("active", RIGHT_HALF);
    const app = harness([otherScreen, active]);
    const otherWrites = otherScreen.geometryWrites;

    app.press("PaveSnapRight", active);

    assert.deepEqual(active.frameGeometry, RIGHT_THIRD);
    assert.equal(otherScreen.geometryWrites, otherWrites);
});

test("different KWin output wrappers with the same name can pair", () => {
    const left = window("left", LEFT_HALF, { output: { name: "DP-1" } });
    const right = window("right", RIGHT_HALF, { output: { name: "DP-1" } });
    const app = harness([left, right]);

    app.press("PaveSnapRight", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
});

test("KWin output wrappers with different names never pair", () => {
    const left = window("left", LEFT_HALF, { output: { name: "DP-3" } });
    const right = window("right", RIGHT_HALF, { output: { name: "DP-1" } });
    const app = harness([left, right]);
    const leftWrites = left.geometryWrites;

    app.press("PaveSnapRight", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.equal(left.geometryWrites, leftWrites);
});

test("different KWin desktop wrappers with the same ID can pair", () => {
    const left = window("left", LEFT_HALF, {
        desktops: [{ id: "desktop-1" }],
    });
    const right = window("right", RIGHT_HALF, {
        desktops: [{ id: "desktop-1" }],
    });
    const app = harness([left, right], {
        currentDesktop: { id: "desktop-1" },
    });

    app.press("PaveSnapRight", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.deepEqual(left.frameGeometry, LEFT_TWO_THIRDS);
});

test("KWin desktop wrappers with different IDs never pair", () => {
    const left = window("left", LEFT_HALF, {
        desktops: [{ id: "desktop-2" }],
    });
    const right = window("right", RIGHT_HALF, {
        desktops: [{ id: "desktop-1" }],
    });
    const app = harness([left, right], {
        currentDesktop: { id: "desktop-1" },
    });
    const leftWrites = left.geometryWrites;

    app.press("PaveSnapRight", right);

    assert.deepEqual(right.frameGeometry, RIGHT_THIRD);
    assert.equal(left.geometryWrites, leftWrites);
});

test("exact split windows on different desktops never pair", () => {
    const otherDesktop = window(
        "other-desktop",
        LEFT_HALF,
        { desktops: ["desktop-2"] }
    );
    const active = window("active", RIGHT_HALF);
    const app = harness([otherDesktop, active]);

    app.press("PaveSnapRight", active);

    assert.deepEqual(active.frameGeometry, RIGHT_THIRD);
    assert.equal(otherDesktop.geometryWrites, 0);
});
