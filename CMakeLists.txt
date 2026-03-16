cmake_minimum_required(VERSION 3.16)
project(pave VERSION 2.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(QT_MIN_VERSION "6.6.0")
set(KF6_MIN_VERSION "6.0.0")

# ECM (Extra CMake Modules) — must be found first
find_package(ECM ${KF6_MIN_VERSION} REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)

# Qt 6
find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS
    Core
    DBus
    Gui
)

# KDE Frameworks 6
find_package(KF6 ${KF6_MIN_VERSION} REQUIRED COMPONENTS
    GlobalAccel
    DBusAddons
    Config
    CoreAddons
    Crash
)

# Generate D-Bus adaptors
qt6_add_dbus_adaptor(PAVE_DBUS_SRCS
    data/org.kde.pave.Daemon.xml
    src/daemon.h PaveDaemon
    daemonadaptor DaemonAdaptor)

qt6_add_dbus_adaptor(PAVE_DBUS_SRCS
    data/org.kde.pave.ScriptResult.xml
    src/windowmanager.h ScriptResultReceiver
    scriptresultadaptor ScriptResultAdaptor)

add_executable(pave
    src/main.cpp
    src/daemon.cpp
    src/zonelayout.cpp
    src/windowmanager.cpp
    ${PAVE_DBUS_SRCS}
)

# Dev build: point to source tree KWin script
target_compile_definitions(pave PRIVATE
    PAVE_KWIN_SCRIPT_PATH="${CMAKE_SOURCE_DIR}/data/kwin-script/main.js"
)

target_link_libraries(pave PRIVATE
    Qt6::Core
    Qt6::DBus
    Qt6::Gui
    KF6::GlobalAccel
    KF6::DBusAddons
    KF6::ConfigCore
    KF6::CoreAddons
    KF6::Crash
)

install(TARGETS pave DESTINATION ${KDE_INSTALL_BINDIR})

# Systemd user service
install(FILES data/pave.service
    DESTINATION ${CMAKE_INSTALL_PREFIX}/lib/systemd/user)

# KWin script for window manipulation
install(DIRECTORY data/kwin-script/
    DESTINATION ${KDE_INSTALL_DATADIR}/kwin/scripts/pave/contents/code)
