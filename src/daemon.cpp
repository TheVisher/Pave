#include "daemon.h"
#include "daemonadaptor.h"

#include <QAction>
#include <QCursor>
#include <QDBusConnection>
#include <QGuiApplication>
#include <QScreen>

#include <QDBusArgument>
#include <QDBusInterface>
#include <QDBusMessage>
#include <QDBusReply>

#include <QJsonDocument>
#include <QJsonObject>

#include <KGlobalAccel>
#include <KConfig>
#include <KConfigGroup>

PaveDaemon::PaveDaemon(QObject *parent)
    : QObject(parent)
    , m_windowManager(std::make_unique<WindowManager>())
{
}

PaveDaemon::~PaveDaemon() = default;

bool PaveDaemon::init()
{
    // Register D-Bus adaptor FIRST — the KWin helper script calls back to
    // /Daemon on load, so the object must exist before the script runs.
    new DaemonAdaptor(this);
    auto bus = QDBusConnection::sessionBus();
    if (!bus.registerObject(QStringLiteral("/Daemon"), this)) {
        qWarning("Failed to register PaveDaemon on D-Bus at /Daemon");
        return false;
    }

    if (!m_windowManager->connect()) {
        qWarning("Failed to connect to KWin");
        return false;
    }

    loadConfig();
    setupShortcuts();

    // Scan open windows so the active window cache is populated immediately
    scanWindows();

    qInfo("Pave daemon initialized");
    return true;
}

// --- Startup scan ---

void PaveDaemon::scanWindows()
{
    // Query the active window so shortcuts work immediately (before any click)
    WindowInfo active = m_windowManager->activeWindow();
    if (!active.id.isEmpty()) {
        m_cachedActiveWindow = active;
        qInfo("scanWindows: active='%s' (%s)", qPrintable(active.appClass),
              qPrintable(active.id.left(8)));
    }
}

void PaveDaemon::refreshActiveWindow()
{
    // If windowActivated was called recently (e.g., from KWin event or D-Bus test),
    // trust that data instead of querying KWin again. This prevents overwriting
    // valid activation data with a stale KWin query result.
    if (m_lastActivationTime.isValid() && m_lastActivationTime.elapsed() < ACTIVATION_SUPPRESS_MS) {
        return;
    }

    WindowInfo active = m_windowManager->activeWindow();
    if (!active.id.isEmpty()) {
        m_cachedActiveWindow = active;
    }
}

// --- Shortcuts ---

/// Clear old v1 Pave shortcuts that were registered under KWin's component.
static void clearOldV1Shortcuts()
{
    static const QStringList oldNames = {
        QStringLiteral("SnapLeft"),
        QStringLiteral("SnapRight"),
        QStringLiteral("SnapUp"),
        QStringLiteral("SnapDown"),
        QStringLiteral("ZoneSnapLeft"),
        QStringLiteral("ZoneSnapRight"),
        QStringLiteral("ZoneSnapUp"),
        QStringLiteral("ZoneSnapDown"),
        QStringLiteral("RestoreSnap"),
    };

    auto bus = QDBusConnection::sessionBus();

    for (const QString &name : oldNames) {
        QDBusMessage msg = QDBusMessage::createMethodCall(
            QStringLiteral("org.kde.kglobalaccel"),
            QStringLiteral("/kglobalaccel"),
            QStringLiteral("org.kde.KGlobalAccel"),
            QStringLiteral("setForeignShortcut")
        );

        QStringList actionId = {
            QStringLiteral("kwin"),
            name,
            QStringLiteral("KWin"),
            QStringLiteral("default"),
        };
        QList<int> keys = {0};

        msg << QVariant::fromValue(actionId) << QVariant::fromValue(keys);
        bus.call(msg);
    }

    qInfo("Cleared old v1 Pave shortcuts from KWin component");
}

static QAction *createAction(QObject *parent, const QString &name,
                              const QString &text, const QKeySequence &shortcut)
{
    auto *action = new QAction(parent);
    action->setObjectName(name);
    action->setText(text);
    KGlobalAccel::self()->removeAllShortcuts(action);
    KGlobalAccel::self()->setDefaultShortcut(action, {shortcut});
    KGlobalAccel::self()->setShortcut(action, {shortcut}, KGlobalAccel::NoAutoloading);
    return action;
}

void PaveDaemon::setupShortcuts()
{
    clearOldV1Shortcuts();
    // Layout control — moves the convergence point
    auto *moveLeft = createAction(this,
        QStringLiteral("pave-move-left"),
        QStringLiteral("Move Zone Split Left"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Left));
    QObject::connect(moveLeft, &QAction::triggered, this, &PaveDaemon::onMoveLeft);

    auto *moveRight = createAction(this,
        QStringLiteral("pave-move-right"),
        QStringLiteral("Move Zone Split Right"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Right));
    QObject::connect(moveRight, &QAction::triggered, this, &PaveDaemon::onMoveRight);

    auto *moveUp = createAction(this,
        QStringLiteral("pave-move-up"),
        QStringLiteral("Move Zone Split Up"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Up));
    QObject::connect(moveUp, &QAction::triggered, this, &PaveDaemon::onMoveUp);

    auto *moveDown = createAction(this,
        QStringLiteral("pave-move-down"),
        QStringLiteral("Move Zone Split Down"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Down));
    QObject::connect(moveDown, &QAction::triggered, this, &PaveDaemon::onMoveDown);

    // Window actions
    auto *maximize = createAction(this,
        QStringLiteral("pave-almost-maximize"),
        QStringLiteral("Almost Maximize"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Return));
    QObject::connect(maximize, &QAction::triggered, this, &PaveDaemon::onAlmostMaximize);

    auto *winLeft = createAction(this,
        QStringLiteral("pave-window-left"),
        QStringLiteral("Move Window to Left Zone"),
        QKeySequence(Qt::SHIFT | Qt::META | Qt::ALT | Qt::Key_Left));
    QObject::connect(winLeft, &QAction::triggered, this, &PaveDaemon::onMoveWindowLeft);

    auto *winRight = createAction(this,
        QStringLiteral("pave-window-right"),
        QStringLiteral("Move Window to Right Zone"),
        QKeySequence(Qt::SHIFT | Qt::META | Qt::ALT | Qt::Key_Right));
    QObject::connect(winRight, &QAction::triggered, this, &PaveDaemon::onMoveWindowRight);

    auto *winUp = createAction(this,
        QStringLiteral("pave-window-up"),
        QStringLiteral("Move Window to Upper Zone"),
        QKeySequence(Qt::SHIFT | Qt::META | Qt::ALT | Qt::Key_Up));
    QObject::connect(winUp, &QAction::triggered, this, &PaveDaemon::onMoveWindowUp);

    auto *winDown = createAction(this,
        QStringLiteral("pave-window-down"),
        QStringLiteral("Move Window to Lower Zone"),
        QKeySequence(Qt::SHIFT | Qt::META | Qt::ALT | Qt::Key_Down));
    QObject::connect(winDown, &QAction::triggered, this, &PaveDaemon::onMoveWindowDown);

    auto *unassign = createAction(this,
        QStringLiteral("pave-unassign"),
        QStringLiteral("Unassign Window from Zone"),
        QKeySequence(Qt::META | Qt::ALT | Qt::Key_Z));
    QObject::connect(unassign, &QAction::triggered, this, &PaveDaemon::onUnassignWindow);
}

// --- Config ---

void PaveDaemon::loadConfig()
{
    KConfig config(QStringLiteral("paverc"));
    KConfigGroup general = config.group(QStringLiteral("General"));
    m_gapSize = general.readEntry("GapSize", 15);

    // Don't load stale assignments — they cause zone ping-pong on startup.
    // Assignments will be rebuilt as the user assigns windows.
    // Clear any stale ones from the config file.
    KConfigGroup assignments = config.group(QStringLiteral("Assignments"));
    for (const QString &group : assignments.groupList()) {
        assignments.deleteGroup(group);
    }
    config.sync();

    qInfo("Config loaded: gap=%d", m_gapSize);
}

void PaveDaemon::saveConfig()
{
    KConfig config(QStringLiteral("paverc"));
    KConfigGroup general = config.group(QStringLiteral("General"));
    general.writeEntry("GapSize", m_gapSize);

    // Save assignments
    KConfigGroup assignments = config.group(QStringLiteral("Assignments"));
    for (const QString &group : assignments.groupList()) {
        assignments.deleteGroup(group);
    }
    for (auto it = m_assignments.constBegin(); it != m_assignments.constEnd(); ++it) {
        KConfigGroup entry = assignments.group(it.key());
        entry.writeEntry("Desktop", it.value().desktop);
        entry.writeEntry("Monitor", it.value().monitor);
        entry.writeEntry("Zone", it.value().zoneId);
    }

    config.sync();
}

// --- Helpers ---

QString PaveDaemon::monitorUnderCursor() const
{
    QPoint cursorPos = QCursor::pos();
    const auto screens = QGuiApplication::screens();
    for (const QScreen *screen : screens) {
        if (screen->geometry().contains(cursorPos)) {
            return screen->name();
        }
    }
    if (auto *primary = QGuiApplication::primaryScreen()) {
        return primary->name();
    }
    return QString();
}

QRect PaveDaemon::screenGeometry(const QString &monitor) const
{
    const auto screens = QGuiApplication::screens();
    for (const QScreen *screen : screens) {
        if (screen->name() == monitor) {
            return screen->geometry();
        }
    }
    return QRect();
}

QString PaveDaemon::ensureLayout(const QString &monitor, int desktop)
{
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    if (!m_layouts.contains(layoutKey)) {
        m_layouts.insert(layoutKey, ZoneLayout());
    }
    return layoutKey;
}

QSet<QString> PaveDaemon::activeZonesOnMonitor(const QString &layoutKey, const QString &monitor) const
{
    Q_UNUSED(layoutKey)
    QSet<QString> active;
    for (auto it = m_windowZones.constBegin(); it != m_windowZones.constEnd(); ++it) {
        if (m_windowMonitors.value(it.key()) == monitor) {
            active.insert(it.value());
        }
    }
    return active;
}

void PaveDaemon::assignWindowToZone(const QString &windowId, const QString &appClass,
                                     const QString &monitor, int desktop,
                                     const QString &zoneId, const QRect &currentGeometry)
{
    // Save pre-snap geometry on first assignment
    if (!m_preSnapGeometry.contains(windowId)) {
        m_preSnapGeometry.insert(windowId, currentGeometry);
    }

    m_windowZones.insert(windowId, zoneId);
    m_windowMonitors.insert(windowId, monitor);

    // Update app class assignment
    m_assignments.insert(appClass, Assignment{desktop, monitor, zoneId});

    // Add to zone stack
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    QString stackKey = QStringLiteral("%1:%2").arg(layoutKey, zoneId);
    QVector<QString> &stack = m_zoneStacks[stackKey];
    stack.removeAll(windowId);
    stack.append(windowId);

    // Clear any maximize state
    m_maxState.remove(windowId);
}

// --- Layout application ---

void PaveDaemon::applyLayout(const QString &monitor)
{
    QRect rect = screenGeometry(monitor);
    if (rect.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = ensureLayout(monitor, desktop);
    ZoneLayout &layout = m_layouts[layoutKey];

    QSet<QString> activeZones = activeZonesOnMonitor(layoutKey, monitor);
    auto zones = layout.computeRects(rect, m_gapSize, activeZones);

    // Batch all window moves
    QHash<QString, QRect> moves;
    for (auto it = m_windowZones.constBegin(); it != m_windowZones.constEnd(); ++it) {
        const QString &windowId = it.key();
        const QString &zoneId = it.value();

        if (m_windowMonitors.value(windowId) != monitor) continue;
        if (m_maxState.contains(windowId)) continue;

        if (zones.contains(zoneId)) {
            moves.insert(windowId, zones.value(zoneId));
        }
    }

    if (!moves.isEmpty()) {
        m_lastMoveTime.start();
        m_windowManager->moveWindows(moves);
    }
}

// --- Layout shortcuts ---
// Left/Right: linear stepping through V_STEPS (1/4, 1/3, 1/2, 2/3, 3/4).
// At the far edge, enter almost-maximize. Clamp after that — no wrapping.
// Pressing the opposite direction from almost-max returns to the edge step.

void PaveDaemon::onMoveLeft()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = ensureLayout(monitor, desktop);
    ZoneLayout &layout = m_layouts[layoutKey];
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    if (m_maxState.contains(win.id)) {
        m_maxState.remove(win.id);
        if (m_windowZones.contains(win.id)) {
            applyLayout(monitor);
        }
        return;
    }

    QString zone = m_windowZones.value(win.id);

    if (zone.isEmpty()) {
        if (!layout.hasVerticalSplit()) {
            layout.stepVerticalRatio(false);
        }
        // If the left column has an h-split, assign to the empty sub-zone
        QString targetZone = QStringLiteral("L");
        if (layout.hasHorizontalSplit(true)) {
            QSet<QString> active = activeZonesOnMonitor(layoutKey, monitor);
            bool topOccupied = active.contains(QStringLiteral("L.T"));
            bool botOccupied = active.contains(QStringLiteral("L.B"));
            if (topOccupied && !botOccupied)
                targetZone = QStringLiteral("L.B");
            else
                targetZone = QStringLiteral("L.T");
        }
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           targetZone, win.geometry);
    } else {
        bool stepped = layout.stepVerticalRatio(false);
        if (!stepped) {
            // Clamped at min — check if R zone is empty, if so just apply
            // (expand-to-fill will handle it)
            return;
        }
    }

    applyLayout(monitor);
}

void PaveDaemon::onMoveRight()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = ensureLayout(monitor, desktop);
    ZoneLayout &layout = m_layouts[layoutKey];
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    if (m_maxState.contains(win.id)) {
        m_maxState.remove(win.id);
        if (m_windowZones.contains(win.id)) {
            applyLayout(monitor);
        }
        return;
    }

    QString zone = m_windowZones.value(win.id);

    if (zone.isEmpty()) {
        if (!layout.hasVerticalSplit()) {
            layout.stepVerticalRatio(true);
        }
        // If the right column has an h-split, assign to the empty sub-zone
        QString targetZone = QStringLiteral("R");
        if (layout.hasHorizontalSplit(false)) {
            QSet<QString> active = activeZonesOnMonitor(layoutKey, monitor);
            bool topOccupied = active.contains(QStringLiteral("R.T"));
            bool botOccupied = active.contains(QStringLiteral("R.B"));
            if (topOccupied && !botOccupied)
                targetZone = QStringLiteral("R.B");
            else
                targetZone = QStringLiteral("R.T");
        }
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           targetZone, win.geometry);
    } else {
        bool stepped = layout.stepVerticalRatio(true);
        if (!stepped) return;
    }

    applyLayout(monitor);
}

void PaveDaemon::onMoveUp()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString zone = m_windowZones.value(win.id);
    if (zone.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = ensureLayout(monitor, desktop);
    ZoneLayout &layout = m_layouts[layoutKey];

    // Determine which column this window is in
    bool leftColumn = zone.startsWith(QLatin1String("L"));

    if (zone == QLatin1String("L") || zone == QLatin1String("R")) {
        // No h-split yet — create one, assign window to top sub-zone
        layout.stepHorizontalRatio(leftColumn, false);
        QString topZone = leftColumn ? QStringLiteral("L.T") : QStringLiteral("R.T");
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           topZone, win.geometry);
    } else {
        bool stepped = layout.stepHorizontalRatio(leftColumn, false);
        if (!stepped) {
            // Clamped — collapse h-split, reassign sub-zone windows to column
            QString colZone = leftColumn ? QStringLiteral("L") : QStringLiteral("R");
            layout.collapseHorizontalSplit(leftColumn);
            for (auto it = m_windowZones.begin(); it != m_windowZones.end(); ++it) {
                if (m_windowMonitors.value(it.key()) == monitor &&
                    it.value().startsWith(colZone) && it.value().contains(QLatin1Char('.'))) {
                    it.value() = colZone;
                }
            }
            assignWindowToZone(win.id, win.appClass, monitor, desktop,
                               colZone, win.geometry);
        }
    }

    applyLayout(monitor);
}

void PaveDaemon::onMoveDown()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString zone = m_windowZones.value(win.id);
    if (zone.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = ensureLayout(monitor, desktop);
    ZoneLayout &layout = m_layouts[layoutKey];

    bool leftColumn = zone.startsWith(QLatin1String("L"));

    if (zone == QLatin1String("L") || zone == QLatin1String("R")) {
        // No h-split yet — create one, assign window to bottom sub-zone
        layout.stepHorizontalRatio(leftColumn, true);
        QString botZone = leftColumn ? QStringLiteral("L.B") : QStringLiteral("R.B");
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           botZone, win.geometry);
    } else {
        bool stepped = layout.stepHorizontalRatio(leftColumn, true);
        if (!stepped) {
            // Clamped — collapse h-split, reassign sub-zone windows to column
            QString colZone = leftColumn ? QStringLiteral("L") : QStringLiteral("R");
            layout.collapseHorizontalSplit(leftColumn);
            for (auto it = m_windowZones.begin(); it != m_windowZones.end(); ++it) {
                if (m_windowMonitors.value(it.key()) == monitor &&
                    it.value().startsWith(colZone) && it.value().contains(QLatin1Char('.'))) {
                    it.value() = colZone;
                }
            }
            assignWindowToZone(win.id, win.appClass, monitor, desktop,
                               colZone, win.geometry);
        }
    }

    applyLayout(monitor);
}

// --- Window assignment shortcuts (Shift+Meta+Alt) ---

void PaveDaemon::onMoveWindowLeft()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    if (!m_layouts.contains(layoutKey)) return;
    const ZoneLayout &layout = m_layouts[layoutKey];

    if (!layout.hasVerticalSplit()) return;

    QString currentZone = m_windowZones.value(win.id);

    if (currentZone.isEmpty()) {
        QVector<QString> zones = layout.activeZoneIds();
        for (const QString &z : zones) {
            if (z.startsWith(QLatin1String("L"))) {
                assignWindowToZone(win.id, win.appClass, monitor, desktop,
                                   z, win.geometry);
                break;
            }
        }
    } else {
        QString adjacent = layout.adjacentZone(currentZone, QStringLiteral("left"));
        if (adjacent.isEmpty()) return;
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           adjacent, win.geometry);
    }

    saveConfig();
    applyLayout(monitor);
}

void PaveDaemon::onMoveWindowRight()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    if (!m_layouts.contains(layoutKey)) return;
    const ZoneLayout &layout = m_layouts[layoutKey];

    if (!layout.hasVerticalSplit()) return;

    QString currentZone = m_windowZones.value(win.id);

    if (currentZone.isEmpty()) {
        QVector<QString> zones = layout.activeZoneIds();
        for (const QString &z : zones) {
            if (z.startsWith(QLatin1String("R"))) {
                assignWindowToZone(win.id, win.appClass, monitor, desktop,
                                   z, win.geometry);
                break;
            }
        }
    } else {
        QString adjacent = layout.adjacentZone(currentZone, QStringLiteral("right"));
        if (adjacent.isEmpty()) return;
        assignWindowToZone(win.id, win.appClass, monitor, desktop,
                           adjacent, win.geometry);
    }

    saveConfig();
    applyLayout(monitor);
}

void PaveDaemon::onMoveWindowUp()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    if (!m_layouts.contains(layoutKey)) return;
    const ZoneLayout &layout = m_layouts[layoutKey];

    QString currentZone = m_windowZones.value(win.id);
    if (currentZone.isEmpty()) return;

    QString adjacent = layout.adjacentZone(currentZone, QStringLiteral("up"));
    if (adjacent.isEmpty()) return;

    assignWindowToZone(win.id, win.appClass, monitor, desktop,
                       adjacent, win.geometry);
    saveConfig();
    applyLayout(monitor);
}

void PaveDaemon::onMoveWindowDown()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    if (monitor.isEmpty()) return;

    int desktop = 1;
    QString layoutKey = QStringLiteral("%1:%2").arg(desktop).arg(monitor);
    if (!m_layouts.contains(layoutKey)) return;
    const ZoneLayout &layout = m_layouts[layoutKey];

    QString currentZone = m_windowZones.value(win.id);
    if (currentZone.isEmpty()) return;

    QString adjacent = layout.adjacentZone(currentZone, QStringLiteral("down"));
    if (adjacent.isEmpty()) return;

    assignWindowToZone(win.id, win.appClass, monitor, desktop,
                       adjacent, win.geometry);
    saveConfig();
    applyLayout(monitor);
}

void PaveDaemon::onUnassignWindow()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = m_windowMonitors.value(win.id);

    if (m_preSnapGeometry.contains(win.id)) {
        m_windowManager->moveWindow(win.id, m_preSnapGeometry.value(win.id));
        m_preSnapGeometry.remove(win.id);
    }

    m_windowZones.remove(win.id);
    m_windowMonitors.remove(win.id);
    m_maxState.remove(win.id);

    for (auto it = m_zoneStacks.begin(); it != m_zoneStacks.end(); ++it) {
        it.value().removeAll(win.id);
    }

    m_assignments.remove(win.appClass);
    saveConfig();

    if (!monitor.isEmpty()) {
        // Collapse h-splits if no windows remain in that column's sub-zones
        int desktop = 1;
        QString layoutKey = ensureLayout(monitor, desktop);
        ZoneLayout &layout = m_layouts[layoutKey];
        QSet<QString> active = activeZonesOnMonitor(layoutKey, monitor);

        if (layout.hasHorizontalSplit(true)) {
            bool leftOccupied = active.contains(QStringLiteral("L"))
                             || active.contains(QStringLiteral("L.T"))
                             || active.contains(QStringLiteral("L.B"));
            if (!leftOccupied) {
                layout.collapseHorizontalSplit(true);
            }
        }
        if (layout.hasHorizontalSplit(false)) {
            bool rightOccupied = active.contains(QStringLiteral("R"))
                              || active.contains(QStringLiteral("R.T"))
                              || active.contains(QStringLiteral("R.B"));
            if (!rightOccupied) {
                layout.collapseHorizontalSplit(false);
            }
        }

        applyLayout(monitor);
    }
}

// --- Almost Maximize ---

void PaveDaemon::onAlmostMaximize()
{
    if (m_windowManager->isMoveInFlight()) return;
    refreshActiveWindow();
    doAlmostMaximize();
}

void PaveDaemon::doAlmostMaximize()
{
    const WindowInfo &win = m_cachedActiveWindow;
    if (win.id.isEmpty()) return;

    QString monitor = monitorUnderCursor();
    QRect rect = screenGeometry(monitor);
    if (rect.isEmpty()) return;

    MaxState state = m_maxState.value(win.id, MaxState::None);

    switch (state) {
    case MaxState::None: {
        if (!m_preSnapGeometry.contains(win.id)) {
            m_preSnapGeometry.insert(win.id, win.geometry);
        }
        QRect almostRect(
            rect.x() + m_gapSize,
            rect.y() + m_gapSize,
            rect.width() - 2 * m_gapSize,
            rect.height() - 2 * m_gapSize
        );
        m_windowManager->moveWindow(win.id, almostRect);
        m_maxState.insert(win.id, MaxState::Almost);
        m_lastMoveTime.start();
        break;
    }
    case MaxState::Almost: {
        QRect fullRect(
            rect.x() + 1,
            rect.y() + 1,
            rect.width() - 2,
            rect.height() - 2
        );
        m_windowManager->moveWindow(win.id, fullRect);
        m_maxState.insert(win.id, MaxState::Full);
        m_lastMoveTime.start();
        break;
    }
    case MaxState::Full: {
        m_maxState.remove(win.id);
        if (m_windowZones.contains(win.id)) {
            QString winMonitor = m_windowMonitors.value(win.id);
            applyLayout(winMonitor);
        } else if (m_preSnapGeometry.contains(win.id)) {
            m_windowManager->moveWindow(win.id, m_preSnapGeometry.value(win.id));
            m_preSnapGeometry.remove(win.id);
            m_lastMoveTime.start();
        }
        break;
    }
    }
}

// --- Window lifecycle ---

void PaveDaemon::windowActivated(const QString &data)
{
    QJsonDocument doc = QJsonDocument::fromJson(data.toUtf8());
    if (!doc.isObject()) return;

    QJsonObject obj = doc.object();
    QString windowId = obj.value(QLatin1String("id")).toString();
    QString appClass = obj.value(QLatin1String("appClass")).toString();
    if (windowId.isEmpty()) return;

    m_cachedActiveWindow.id = windowId;
    m_cachedActiveWindow.appClass = appClass;
    m_cachedActiveWindow.geometry = QRect(
        obj.value(QLatin1String("x")).toInt(),
        obj.value(QLatin1String("y")).toInt(),
        obj.value(QLatin1String("width")).toInt(),
        obj.value(QLatin1String("height")).toInt()
    );
    m_cachedActiveWindow.screen = obj.value(QLatin1String("screen")).toString();

    m_lastActivationTime.start();

    if (m_lastMoveTime.isValid() && m_lastMoveTime.elapsed() < MOVE_SUPPRESS_MS) {
        return;
    }
}

// --- Testing / introspection ---

void PaveDaemon::triggerAction(const QString &action)
{
    // Re-arm the activation timer so refreshActiveWindow() keeps trusting
    // the cached window set by windowActivated(). Without this, the 500ms
    // suppression window expires between consecutive triggerAction calls
    // and the daemon would query KWin for the (wrong) real active window.
    if (m_lastActivationTime.isValid()) {
        m_lastActivationTime.start();
    }

    if (action == QLatin1String("moveLeft")) onMoveLeft();
    else if (action == QLatin1String("moveRight")) onMoveRight();
    else if (action == QLatin1String("moveUp")) onMoveUp();
    else if (action == QLatin1String("moveDown")) onMoveDown();
    else if (action == QLatin1String("almostMaximize")) onAlmostMaximize();
    else if (action == QLatin1String("moveWindowLeft")) onMoveWindowLeft();
    else if (action == QLatin1String("moveWindowRight")) onMoveWindowRight();
    else if (action == QLatin1String("moveWindowUp")) onMoveWindowUp();
    else if (action == QLatin1String("moveWindowDown")) onMoveWindowDown();
    else if (action == QLatin1String("unassign")) onUnassignWindow();
    else qWarning("Unknown action: %s", qPrintable(action));
}

QString PaveDaemon::getState()
{
    QJsonObject state;

    // Layouts
    QJsonObject layouts;
    for (auto it = m_layouts.constBegin(); it != m_layouts.constEnd(); ++it) {
        const ZoneLayout &l = it.value();
        QJsonObject lo;
        lo.insert(QStringLiteral("hasVerticalSplit"), l.hasVerticalSplit());
        lo.insert(QStringLiteral("verticalRatio"), l.verticalRatio());
        lo.insert(QStringLiteral("hasLeftHSplit"), l.hasHorizontalSplit(true));
        lo.insert(QStringLiteral("leftHRatio"), l.horizontalRatio(true));
        lo.insert(QStringLiteral("hasRightHSplit"), l.hasHorizontalSplit(false));
        lo.insert(QStringLiteral("rightHRatio"), l.horizontalRatio(false));
        layouts.insert(it.key(), lo);
    }
    state.insert(QStringLiteral("layouts"), layouts);

    // Window zones
    QJsonObject zones;
    for (auto it = m_windowZones.constBegin(); it != m_windowZones.constEnd(); ++it) {
        zones.insert(it.key(), it.value());
    }
    state.insert(QStringLiteral("windowZones"), zones);

    // Window monitors
    QJsonObject monitors;
    for (auto it = m_windowMonitors.constBegin(); it != m_windowMonitors.constEnd(); ++it) {
        monitors.insert(it.key(), it.value());
    }
    state.insert(QStringLiteral("windowMonitors"), monitors);

    // Max state
    QJsonObject maxStates;
    for (auto it = m_maxState.constBegin(); it != m_maxState.constEnd(); ++it) {
        QString s = it.value() == MaxState::Almost ? QStringLiteral("almost")
                  : it.value() == MaxState::Full   ? QStringLiteral("full")
                  : QStringLiteral("none");
        maxStates.insert(it.key(), s);
    }
    state.insert(QStringLiteral("maxState"), maxStates);

    // Active window
    QJsonObject active;
    active.insert(QStringLiteral("id"), m_cachedActiveWindow.id);
    active.insert(QStringLiteral("appClass"), m_cachedActiveWindow.appClass);
    active.insert(QStringLiteral("screen"), m_cachedActiveWindow.screen);
    state.insert(QStringLiteral("activeWindow"), active);

    return QString::fromUtf8(QJsonDocument(state).toJson(QJsonDocument::Compact));
}

void PaveDaemon::resetState()
{
    m_layouts.clear();
    m_assignments.clear();
    m_preSnapGeometry.clear();
    m_windowZones.clear();
    m_windowMonitors.clear();
    m_maxState.clear();
    m_zoneStacks.clear();
    m_cachedActiveWindow = WindowInfo{};
    qInfo("State reset (all layouts, assignments, and window tracking cleared)");
}

void PaveDaemon::windowClosed(const QString &windowId)
{
    QString zoneId = m_windowZones.value(windowId);
    QString monitor = m_windowMonitors.value(windowId);

    m_windowZones.remove(windowId);
    m_windowMonitors.remove(windowId);
    m_preSnapGeometry.remove(windowId);
    m_maxState.remove(windowId);

    for (auto it = m_zoneStacks.begin(); it != m_zoneStacks.end(); ++it) {
        it.value().removeAll(windowId);
    }

    // If this window was in a zone, unminimize the next window in that zone
    if (!zoneId.isEmpty() && !monitor.isEmpty()) {
        for (auto it = m_windowZones.constBegin(); it != m_windowZones.constEnd(); ++it) {
            if (it.value() == zoneId && m_windowMonitors.value(it.key()) == monitor) {
                m_windowManager->unminimizeWindow(it.key());
                m_lastMoveTime.start();
                break;
            }
        }

        applyLayout(monitor);
    }
}
