#include "windowmanager.h"

#include <QDBusConnection>
#include <QDBusPendingReply>
#include <QDBusReply>
#include <QDir>
#include <QFile>
#include <QStandardPaths>
#include <QTemporaryFile>
#include <QTimer>

#include "scriptresultadaptor.h"

// --- ScriptResultReceiver ---

ScriptResultReceiver::ScriptResultReceiver(QObject *parent)
    : QObject(parent)
{
}

void ScriptResultReceiver::reset()
{
    m_result.clear();
    m_received = false;
}

QString ScriptResultReceiver::waitForResult(int timeoutMs)
{
    if (m_received) {
        return m_result;
    }

    QTimer timer;
    timer.setSingleShot(true);
    QObject::connect(&timer, &QTimer::timeout, &m_loop, &QEventLoop::quit);
    timer.start(timeoutMs);

    m_loop.exec();

    return m_received ? m_result : QString();
}

void ScriptResultReceiver::receiveResult(const QString &result)
{
    m_result = result;
    m_received = true;
    m_loop.quit();
}

// --- WindowManager ---

WindowManager::WindowManager(QObject *parent)
    : QObject(parent)
{
}

WindowManager::~WindowManager()
{
    // Unload the persistent KWin script by plugin name
    if (m_kwinScripting) {
        m_kwinScripting->call(QStringLiteral("unloadScript"), QStringLiteral("pave"));
    }
    delete m_kwinScripting;
}

bool WindowManager::connect()
{
    auto bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        qWarning("Cannot connect to session D-Bus");
        return false;
    }

    // Check that KWin is running
    m_kwinScripting = new QDBusInterface(
        QStringLiteral("org.kde.KWin"),
        QStringLiteral("/Scripting"),
        QStringLiteral("org.kde.kwin.Scripting"),
        bus
    );

    if (!m_kwinScripting->isValid()) {
        qWarning("KWin scripting interface not available");
        return false;
    }

    // Set up the ScriptResultReceiver on D-Bus
    m_scriptResult = new ScriptResultReceiver(this);
    new ScriptResultAdaptor(m_scriptResult);

    if (!bus.registerObject(QStringLiteral("/Daemon/ScriptResult"), m_scriptResult)) {
        qWarning("Failed to register ScriptResultReceiver on D-Bus");
        return false;
    }

    return loadKWinScript();
}

bool WindowManager::loadKWinScript()
{
    // Find the KWin script — installed to kwin/scripts/pave/contents/code/main.js
    QString scriptPath = QStandardPaths::locate(
        QStandardPaths::GenericDataLocation,
        QStringLiteral("kwin/scripts/pave/contents/code/main.js")
    );

    if (scriptPath.isEmpty()) {
        // Fallback: dev build path from compile definition
        scriptPath = QStringLiteral(PAVE_KWIN_SCRIPT_PATH);
    }

    if (scriptPath.isEmpty()) {
        qWarning("Could not find KWin helper script");
        return false;
    }

    // Unload any previous instance (e.g., from a crash)
    m_kwinScripting->call(QStringLiteral("unloadScript"), QStringLiteral("pave"));

    QDBusReply<int> reply = m_kwinScripting->call(
        QStringLiteral("loadScript"),
        scriptPath,
        QStringLiteral("pave")
    );

    if (!reply.isValid()) {
        qWarning("Failed to load KWin script: %s", qPrintable(reply.error().message()));
        return false;
    }

    m_scriptId = reply.value();
    if (m_scriptId < 0) {
        qWarning("KWin returned invalid script ID %d", m_scriptId);
        return false;
    }

    // Run the script
    auto scriptObjPath = QStringLiteral("/Scripting/Script%1").arg(m_scriptId);
    QDBusInterface scriptIface(
        QStringLiteral("org.kde.KWin"),
        scriptObjPath,
        QStringLiteral("org.kde.kwin.Script"),
        QDBusConnection::sessionBus()
    );
    scriptIface.call(QStringLiteral("run"));

    qInfo("KWin helper script loaded (id=%d)", m_scriptId);
    return true;
}

// --- Temp script execution ---

/// Load a temp script, run it, and return the script name for cleanup.
/// Returns the script name and sets scriptId. Returns empty on failure.
static QString loadAndRunTempScript(QDBusInterface *kwinScripting,
                                     const QString &tempPath,
                                     const QString &scriptName)
{
    QDBusReply<int> reply = kwinScripting->call(
        QStringLiteral("loadScript"),
        tempPath,
        scriptName
    );

    if (!reply.isValid()) {
        qWarning("Failed to load temp script '%s': %s",
                 qPrintable(scriptName), qPrintable(reply.error().message()));
        return {};
    }

    int scriptId = reply.value();
    if (scriptId < 0) {
        qWarning("KWin returned invalid ID for temp script '%s'", qPrintable(scriptName));
        return {};
    }

    auto scriptObjPath = QStringLiteral("/Scripting/Script%1").arg(scriptId);
    QDBusInterface scriptIface(
        QStringLiteral("org.kde.KWin"),
        scriptObjPath,
        QStringLiteral("org.kde.kwin.Script"),
        QDBusConnection::sessionBus()
    );
    scriptIface.call(QStringLiteral("run"));

    return scriptName;
}

QString WindowManager::runKWinScript(const QString &scriptBody, int timeoutMs)
{
    // Write temp JS file wrapping the script with a D-Bus callback
    QString tempPath = QDir::tempPath() +
        QStringLiteral("/pave-temp-%1.js").arg(++m_tempScriptCounter);

    QFile tempFile(tempPath);
    if (!tempFile.open(QIODevice::WriteOnly | QIODevice::Text)) {
        qWarning("Failed to create temp script file: %s", qPrintable(tempPath));
        return {};
    }

    QString wrappedScript = QStringLiteral(
        "var __paveResult = (function() {\n"
        "%1\n"
        "})();\n"
        "callDBus('org.kde.pave', '/Daemon/ScriptResult', "
        "'org.kde.pave.ScriptResult', 'receiveResult', "
        "JSON.stringify(__paveResult));\n"
    ).arg(scriptBody);

    tempFile.write(wrappedScript.toUtf8());
    tempFile.close();

    // Load and run
    QString scriptName = QStringLiteral("pave_temp_%1").arg(m_tempScriptCounter);

    // Unload any previous instance with this name (shouldn't happen, but safety)
    m_kwinScripting->call(QStringLiteral("unloadScript"), scriptName);

    m_scriptResult->reset();

    QString loaded = loadAndRunTempScript(m_kwinScripting, tempPath, scriptName);
    if (loaded.isEmpty()) {
        QFile::remove(tempPath);
        return {};
    }

    // Wait for callback
    QString result = m_scriptResult->waitForResult(timeoutMs);

    // Cleanup: unload by plugin name and delete temp file
    m_kwinScripting->call(QStringLiteral("unloadScript"), scriptName);
    QFile::remove(tempPath);

    return result;
}

void WindowManager::runKWinScriptAsync(const QString &scriptBody)
{
    // Write temp JS file — no callback needed, fire-and-forget
    QString tempPath = QDir::tempPath() +
        QStringLiteral("/pave-temp-%1.js").arg(++m_tempScriptCounter);

    QFile tempFile(tempPath);
    if (!tempFile.open(QIODevice::WriteOnly | QIODevice::Text)) {
        qWarning("Failed to create temp script file: %s", qPrintable(tempPath));
        return;
    }

    // Just wrap in IIFE, no callback
    QString wrappedScript = QStringLiteral(
        "(function() {\n"
        "%1\n"
        "})();\n"
    ).arg(scriptBody);

    tempFile.write(wrappedScript.toUtf8());
    tempFile.close();

    QString scriptName = QStringLiteral("pave_temp_%1").arg(m_tempScriptCounter);

    // Async unload any previous instance (fire-and-forget)
    m_kwinScripting->asyncCall(QStringLiteral("unloadScript"), scriptName);

    // Async load — chain run() in the callback so we never block the event loop
    QDBusPendingCall loadCall = m_kwinScripting->asyncCall(
        QStringLiteral("loadScript"), tempPath, scriptName);

    auto *watcher = new QDBusPendingCallWatcher(loadCall, this);
    QObject::connect(watcher, &QDBusPendingCallWatcher::finished,
                     this, [this, scriptName, tempPath](QDBusPendingCallWatcher *w) {
        w->deleteLater();

        QDBusPendingReply<int> reply = *w;
        if (reply.isError()) {
            qWarning("Failed to load temp script '%s': %s",
                     qPrintable(scriptName), qPrintable(reply.error().message()));
            QFile::remove(tempPath);
            return;
        }

        int scriptId = reply.value();
        if (scriptId < 0) {
            qWarning("KWin returned invalid ID for temp script '%s'", qPrintable(scriptName));
            QFile::remove(tempPath);
            return;
        }

        // Async run
        auto scriptObjPath = QStringLiteral("/Scripting/Script%1").arg(scriptId);
        QDBusInterface scriptIface(
            QStringLiteral("org.kde.KWin"),
            scriptObjPath,
            QStringLiteral("org.kde.kwin.Script"),
            QDBusConnection::sessionBus()
        );
        scriptIface.asyncCall(QStringLiteral("run"));

        // Schedule cleanup
        QTimer::singleShot(500, this, [this, scriptName, tempPath]() {
            m_kwinScripting->asyncCall(QStringLiteral("unloadScript"), scriptName);
            QFile::remove(tempPath);
        });
    });
}

// --- Window commands ---

void WindowManager::moveWindow(const QString &windowId, const QRect &rect)
{
    m_pendingMoves.insert(windowId, rect);
    flushMoves();
}

void WindowManager::moveWindows(const QHash<QString, QRect> &windowRects)
{
    for (auto it = windowRects.constBegin(); it != windowRects.constEnd(); ++it) {
        m_pendingMoves.insert(it.key(), it.value());
    }
    flushMoves();
}

void WindowManager::flushMoves()
{
    if (m_moveInFlight) return;
    if (m_pendingMoves.isEmpty()) return;

    QHash<QString, QRect> moves;
    moves.swap(m_pendingMoves);
    m_moveInFlight = true;

    // Build a single script that moves all windows
    QString script = QStringLiteral("var windows = workspace.stackingOrder;\n");

    for (auto it = moves.constBegin(); it != moves.constEnd(); ++it) {
        const QString &windowId = it.key();
        const QRect &rect = it.value();
        script += QStringLiteral(
            "for (var i = 0; i < windows.length; i++) {\n"
            "    if (windows[i].internalId.toString() === '%1') {\n"
            "        windows[i].frameGeometry = {x: %2, y: %3, width: %4, height: %5};\n"
            "        break;\n"
            "    }\n"
            "}\n"
        ).arg(windowId).arg(rect.x()).arg(rect.y()).arg(rect.width()).arg(rect.height());
    }

    // Reuse a single script name and file path to avoid KWin script accumulation.
    // Unload previous → write file → load → run is the minimal cycle.
    static const QString tempPath = QDir::tempPath() + QStringLiteral("/pave-move.js");
    static const QString scriptName = QStringLiteral("pave_move");

    QFile tempFile(tempPath);
    if (!tempFile.open(QIODevice::WriteOnly | QIODevice::Text | QIODevice::Truncate)) {
        qWarning("Failed to create temp script file: %s", qPrintable(tempPath));
        m_moveInFlight = false;
        return;
    }
    tempFile.write(script.toUtf8());
    tempFile.close();

    // Synchronous unload + load + run — three D-Bus calls but keeps KWin clean.
    // Use sync calls here: they're fast (local D-Bus) and we NEED them ordered.
    m_kwinScripting->call(QStringLiteral("unloadScript"), scriptName);

    QDBusReply<int> loadReply = m_kwinScripting->call(
        QStringLiteral("loadScript"), tempPath, scriptName);

    if (!loadReply.isValid() || loadReply.value() < 0) {
        qWarning("[MOVE] loadScript failed: %s",
                 qPrintable(loadReply.error().message()));
        m_moveInFlight = false;
        return;
    }

    int scriptId = loadReply.value();

    auto scriptObjPath = QStringLiteral("/Scripting/Script%1").arg(scriptId);
    QDBusInterface scriptIface(
        QStringLiteral("org.kde.KWin"),
        scriptObjPath,
        QStringLiteral("org.kde.kwin.Script"),
        QDBusConnection::sessionBus()
    );
    scriptIface.call(QStringLiteral("run"));
    m_moveInFlight = false;
}

void WindowManager::minimizeWindow(const QString &windowId)
{
    QString script = QStringLiteral(
        "var windows = workspace.stackingOrder;\n"
        "for (var i = 0; i < windows.length; i++) {\n"
        "    if (windows[i].internalId.toString() === '%1') {\n"
        "        windows[i].minimized = true;\n"
        "        break;\n"
        "    }\n"
        "}\n"
    ).arg(windowId);

    runKWinScriptAsync(script);
}

void WindowManager::unminimizeWindow(const QString &windowId)
{
    QString script = QStringLiteral(
        "var windows = workspace.stackingOrder;\n"
        "for (var i = 0; i < windows.length; i++) {\n"
        "    if (windows[i].internalId.toString() === '%1') {\n"
        "        windows[i].minimized = false;\n"
        "        break;\n"
        "    }\n"
        "}\n"
    ).arg(windowId);

    runKWinScriptAsync(script);
}

WindowInfo WindowManager::activeWindow()
{
    QString script = QStringLiteral(
        "var w = workspace.activeWindow;\n"
        "if (!w || !w.normalWindow) return null;\n"
        "return {\n"
        "    id: w.internalId.toString(),\n"
        "    appClass: w.resourceClass.toString(),\n"
        "    x: w.frameGeometry.x,\n"
        "    y: w.frameGeometry.y,\n"
        "    width: w.frameGeometry.width,\n"
        "    height: w.frameGeometry.height,\n"
        "    minimized: w.minimized,\n"
        "    fullScreen: w.fullScreen,\n"
        "    desktop: w.desktops.length > 0 ? w.desktops[0].id : 0,\n"
        "    screen: w.output ? w.output.name : ''\n"
        "};\n"
    );

    QString result = runKWinScript(script);
    if (result.isEmpty() || result == QLatin1String("null")) {
        return {};
    }

    QJsonDocument doc = QJsonDocument::fromJson(result.toUtf8());
    if (!doc.isObject()) {
        return {};
    }

    return parseWindowInfo(doc.object());
}

QVector<WindowInfo> WindowManager::getWindows()
{
    QString script = QStringLiteral(
        "var result = [];\n"
        "var windows = workspace.stackingOrder;\n"
        "for (var i = 0; i < windows.length; i++) {\n"
        "    var w = windows[i];\n"
        "    if (!w.normalWindow) continue;\n"
        "    result.push({\n"
        "        id: w.internalId.toString(),\n"
        "        appClass: w.resourceClass.toString(),\n"
        "        x: w.frameGeometry.x,\n"
        "        y: w.frameGeometry.y,\n"
        "        width: w.frameGeometry.width,\n"
        "        height: w.frameGeometry.height,\n"
        "        minimized: w.minimized,\n"
        "        fullScreen: w.fullScreen,\n"
        "        desktop: w.desktops.length > 0 ? w.desktops[0].id : 0,\n"
        "        screen: w.output ? w.output.name : ''\n"
        "    });\n"
        "}\n"
        "return result;\n"
    );

    QString result = runKWinScript(script);
    if (result.isEmpty()) {
        return {};
    }

    QJsonDocument doc = QJsonDocument::fromJson(result.toUtf8());
    if (!doc.isArray()) {
        return {};
    }

    QVector<WindowInfo> windows;
    const QJsonArray arr = doc.array();
    windows.reserve(arr.size());
    for (const QJsonValue &val : arr) {
        if (val.isObject()) {
            windows.append(parseWindowInfo(val.toObject()));
        }
    }
    return windows;
}

WindowInfo WindowManager::parseWindowInfo(const QJsonObject &obj)
{
    WindowInfo info;
    info.id = obj.value(QLatin1String("id")).toString();
    info.appClass = obj.value(QLatin1String("appClass")).toString();
    info.geometry = QRect(
        obj.value(QLatin1String("x")).toInt(),
        obj.value(QLatin1String("y")).toInt(),
        obj.value(QLatin1String("width")).toInt(),
        obj.value(QLatin1String("height")).toInt()
    );
    info.minimized = obj.value(QLatin1String("minimized")).toBool();
    info.fullScreen = obj.value(QLatin1String("fullScreen")).toBool();
    info.screen = obj.value(QLatin1String("screen")).toString();
    info.desktop = obj.value(QLatin1String("desktop")).toInt();
    return info;
}
