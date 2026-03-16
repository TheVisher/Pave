#pragma once

#include <QObject>
#include <QRect>
#include <QString>
#include <QHash>
#include <QVector>
#include <QDBusInterface>
#include <QDBusPendingCallWatcher>
#include <QEventLoop>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>

/// Receives results from temp KWin scripts via D-Bus callback.
/// Registered at /Daemon/ScriptResult on the session bus.
class ScriptResultReceiver : public QObject
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.pave.ScriptResult")

public:
    explicit ScriptResultReceiver(QObject *parent = nullptr);

    /// Reset state before waiting for a new result.
    void reset();

    /// Block until receiveResult is called or timeout expires.
    /// Returns the result string, or empty on timeout.
    QString waitForResult(int timeoutMs = 5000);

public Q_SLOTS:
    /// Called by KWin temp scripts via callDBus.
    void receiveResult(const QString &result);

private:
    QString m_result;
    bool m_received = false;
    QEventLoop m_loop;
};

/// Info about a single window, returned by queries.
struct WindowInfo {
    QString id;
    QString appClass;
    QRect geometry;
    bool minimized = false;
    bool fullScreen = false;
    QString screen;
    int desktop = 0;
};

/// Wrapper around KWin's D-Bus interface for window management.
/// Uses temp KWin scripts for commands and queries (the only way on Wayland).
class WindowManager : public QObject
{
    Q_OBJECT

public:
    explicit WindowManager(QObject *parent = nullptr);
    ~WindowManager() override;

    /// Connect to KWin via D-Bus and load the helper script.
    bool connect();

    /// Whether a move script is currently in-flight (waiting for KWin).
    bool isMoveInFlight() const { return m_moveInFlight; }

    /// Move and resize a window to the given rect.
    void moveWindow(const QString &windowId, const QRect &rect);

    /// Move multiple windows in a single KWin script call (batch).
    void moveWindows(const QHash<QString, QRect> &windowRects);

    /// Minimize a window.
    void minimizeWindow(const QString &windowId);

    /// Unminimize (restore) a window.
    void unminimizeWindow(const QString &windowId);

    /// Get the active window's info.
    WindowInfo activeWindow();

    /// Get all normal (non-special) windows.
    QVector<WindowInfo> getWindows();

private:
    /// Load the persistent KWin helper script via D-Bus.
    bool loadKWinScript();

    /// Run a temp KWin script and return its result.
    /// The script body should return a value; it will be JSON.stringify'd
    /// and sent back via D-Bus callback.
    QString runKWinScript(const QString &scriptBody, int timeoutMs = 5000);

    /// Run a temp KWin script without waiting for a result (fire-and-forget).
    void runKWinScriptAsync(const QString &scriptBody);

    /// Parse a WindowInfo from a JSON object.
    static WindowInfo parseWindowInfo(const QJsonObject &obj);

    /// Flush pending moves — sends one script if none in-flight.
    void flushMoves();

    QDBusInterface *m_kwinScripting = nullptr;
    ScriptResultReceiver *m_scriptResult = nullptr;
    int m_scriptId = -1;
    int m_tempScriptCounter = 0;

    /// Move coalescing: only one script in-flight at a time.
    /// Rapid keypresses update pending moves; the next script fires when
    /// the current one finishes loading.
    QHash<QString, QRect> m_pendingMoves;
    bool m_moveInFlight = false;
};
