#include <QGuiApplication>
#include <KAboutData>
#include <KCrash>
#include <KDBusService>

#include "daemon.h"

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);

    KAboutData aboutData(
        QStringLiteral("pave"),
        QStringLiteral("Pave"),
        QStringLiteral("2.0.0"),
        QStringLiteral("Zone-based window tiling for KDE Plasma"),
        KAboutLicense::MIT,
        QStringLiteral("(c) 2025-2026")
    );
    KAboutData::setApplicationData(aboutData);

    KCrash::initialize();

    // Ensure only one instance runs
    KDBusService service(KDBusService::Unique);

    PaveDaemon daemon;
    if (!daemon.init()) {
        qCritical("Failed to initialize Pave daemon");
        return 1;
    }

    return app.exec();
}
