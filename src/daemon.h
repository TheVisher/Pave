#pragma once

#include <QObject>
#include <QElapsedTimer>
#include <QSet>
#include <memory>

#include "windowmanager.h"
#include "zonelayout.h"

class QAction;

/// Core Pave daemon. Owns the zone layouts, window assignments, and shortcuts.
class PaveDaemon : public QObject
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.pave.Daemon")

public:
    explicit PaveDaemon(QObject *parent = nullptr);
    ~PaveDaemon() override;

    /// Initialize the daemon: connect to KWin, register shortcuts, load config.
    bool init();

public Q_SLOTS:
    // D-Bus methods called by KWin script event forwarding
    void windowActivated(const QString &data);
    void windowClosed(const QString &windowId);

    // Testing / introspection
    void triggerAction(const QString &action);
    QString getState();
    void resetState();

private:
    void setupShortcuts();
    void loadConfig();
    void saveConfig();

    /// Scan currently open windows and populate the active window cache.
    void scanWindows();

    /// Query KWin for the current active window and update the cache.
    void refreshActiveWindow();

    /// Recompute all zone rects for a monitor and move windows to match.
    void applyLayout(const QString &monitor);

    /// Determine which monitor the mouse cursor is currently on.
    QString monitorUnderCursor() const;

    /// Get screen geometry for a monitor by name.
    QRect screenGeometry(const QString &monitor) const;

    /// Get the layout key and ensure layout exists for the given monitor.
    QString ensureLayout(const QString &monitor, int desktop = 1);

    /// Compute which zones are active (have assigned windows) on a monitor.
    QSet<QString> activeZonesOnMonitor(const QString &layoutKey, const QString &monitor) const;

    /// Assign a window to a zone, updating all tracking state.
    void assignWindowToZone(const QString &windowId, const QString &appClass,
                            const QString &monitor, int desktop, const QString &zoneId,
                            const QRect &currentGeometry);

private Q_SLOTS:
    // Layout control (no focus required — operates on monitor under cursor)
    void onMoveLeft();
    void onMoveRight();
    void onMoveUp();
    void onMoveDown();

    // Window actions (focused window)
    void onAlmostMaximize();
    void doAlmostMaximize();
    void onMoveWindowLeft();
    void onMoveWindowRight();
    void onMoveWindowUp();
    void onMoveWindowDown();
    void onUnassignWindow();

private:
    std::unique_ptr<WindowManager> m_windowManager;

    /// Zone layouts keyed by "desktop:monitor" (e.g., "1:DP-1")
    QHash<QString, ZoneLayout> m_layouts;

    /// Window-to-zone assignments keyed by app class (e.g., "zen-browser" -> "L")
    /// Scoped per desktop:monitor.
    struct Assignment {
        int desktop;
        QString monitor;
        QString zoneId;  // "L", "R", "L.T", "L.B", "R.T", "R.B"
    };
    QHash<QString, Assignment> m_assignments;

    /// Pre-snap floating geometry per window ID (for restore)
    QHash<QString, QRect> m_preSnapGeometry;

    /// Which zone each running window is currently in, keyed by window ID
    QHash<QString, QString> m_windowZones;

    /// Which monitor each running window is on, keyed by window ID
    QHash<QString, QString> m_windowMonitors;

    /// Almost-maximize state per window ID
    enum class MaxState { None, Almost, Full };
    QHash<QString, MaxState> m_maxState;

    /// Zone stacks: "layoutKey:zoneId" -> ordered window IDs
    /// The last entry is the visible (top) window.
    QHash<QString, QVector<QString>> m_zoneStacks;

    int m_gapSize = 15;

    /// Cached active window info (updated by KWin event callback).
    WindowInfo m_cachedActiveWindow;

    /// Timer to suppress windowActivated stacking logic after moves.
    /// moveWindows is fire-and-forget — KWin sends activation events AFTER
    /// our call returns, so a bool flag can't suppress them.
    QElapsedTimer m_lastMoveTime;
    static constexpr int MOVE_SUPPRESS_MS = 200;

    /// Timer to suppress refreshActiveWindow when windowActivated was called recently.
    /// This lets D-Bus-injected window activations (from tests or KWin events) take
    /// precedence over the KWin query in refreshActiveWindow().
    QElapsedTimer m_lastActivationTime;
    static constexpr int ACTIVATION_SUPPRESS_MS = 500;

};
