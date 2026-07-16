#!/usr/bin/env python3
import os
import shutil
import subprocess
import sys
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import (
    QApplication,
    QCheckBox,
    QFrame,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QMainWindow,
    QPushButton,
    QSpinBox,
    QVBoxLayout,
    QWidget,
)


HOME = Path.home()
SOURCE_DIR = Path(__file__).resolve().parent
PACKAGED_DATA_DIR = Path("/usr/share/pave")
SOURCE_RESOURCE_DIR = SOURCE_DIR / "resources"
RESOURCE_DIR = (
    SOURCE_RESOURCE_DIR if SOURCE_RESOURCE_DIR.is_dir() else PACKAGED_DATA_DIR
)
KWINRC = HOME / ".config" / "kwinrc"
SHORTCUTSRC = HOME / ".config" / "kglobalshortcutsrc"
SCRIPT_DIR = HOME / ".local" / "share" / "kwin" / "scripts" / "pave"
SCRIPT_SOURCE_DIR = (
    SOURCE_DIR / "kwin"
    if (SOURCE_DIR / "kwin").is_dir()
    else PACKAGED_DATA_DIR / "kwin"
)
AUTOSTART_FILE = HOME / ".config" / "autostart" / "pave.desktop"
AUTOSTART_DESKTOP_FILE = RESOURCE_DIR / "pave-autostart.desktop"
ICON_FILE = RESOURCE_DIR / "pave.svg"


def run(command, check=False):
    result = subprocess.run(command, text=True, capture_output=True)
    if check and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result


def kread(group, key, default=""):
    result = run([
        "kreadconfig6",
        "--file",
        str(KWINRC),
        "--group",
        group,
        "--key",
        key,
    ])
    value = result.stdout.strip()
    return value if value else default


def kwrite(group, key, value):
    run([
        "kwriteconfig6",
        "--file",
        str(KWINRC),
        "--group",
        group,
        "--key",
        key,
        str(value),
    ], check=True)


def write_shortcut(key, value):
    run([
        "kwriteconfig6",
        "--file",
        str(SHORTCUTSRC),
        "--group",
        "kwin",
        "--key",
        key,
        value,
    ], check=True)


def qdbus(*args):
    return run(["qdbus6", *args])


def busctl_set_foreign(action, description, keycode):
    run([
        "busctl",
        "--user",
        "call",
        "org.kde.kglobalaccel",
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
        "setForeignShortcut",
        "asai",
        "4",
        "kwin",
        action,
        "KWin",
        description,
        "1",
        str(keycode),
    ])


def install_script_source():
    source_script = SCRIPT_SOURCE_DIR / "contents" / "code" / "main.js"
    source_metadata = SCRIPT_SOURCE_DIR / "metadata.json"
    if not source_script.exists() or not source_metadata.exists():
        raise RuntimeError("Pave KWin script source is incomplete")

    installed_code_dir = SCRIPT_DIR / "contents" / "code"
    installed_code_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_script, installed_code_dir / "main.js")
    shutil.copy2(source_metadata, SCRIPT_DIR / "metadata.json")


def reload_pave_script():
    install_script_source()
    qdbus(
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting.unloadScript",
        "pave",
    )
    qdbus("org.kde.KWin", "/KWin", "reconfigure")


def set_pave_shortcuts_live():
    write_shortcut("PaveSnapLeft", "Ctrl+Alt+Left,none,Pave: Snap Left")
    write_shortcut("PaveSnapRight", "Ctrl+Alt+Right,none,Pave: Snap Right")
    write_shortcut("PaveAlmostMaximize", "Ctrl+Alt+Return,none,Pave: Almost Maximize")
    write_shortcut("PaveSnapTop", "none,none,Pave: Snap Top")
    write_shortcut("PaveSnapBottom", "none,none,Pave: Snap Bottom")

    write_shortcut(
        "KZones: Move active window to next zone",
        "none,none,KZones: Move active window to next zone",
    )
    write_shortcut(
        "KZones: Move active window to previous zone",
        "none,none,KZones: Move active window to previous zone",
    )
    write_shortcut(
        "RectanglePaddingLeft",
        "none,none,Rectangle Padding: Left Half",
    )
    write_shortcut(
        "RectanglePaddingRight",
        "none,none,Rectangle Padding: Right Half",
    )
    write_shortcut(
        "RectanglePaddingAlmostMaximize",
        "none,none,Rectangle Padding: Almost Maximize",
    )

    busctl_set_foreign("PaveSnapLeft", "Pave: Snap Left", 218103826)
    busctl_set_foreign("PaveSnapRight", "Pave: Snap Right", 218103828)
    busctl_set_foreign("PaveAlmostMaximize", "Pave: Almost Maximize", 218103812)
    run([
        "busctl",
        "--user",
        "call",
        "org.kde.kglobalaccel",
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
        "unregister",
        "ss",
        "kwin",
        "PaveSnapTop",
    ])
    run([
        "busctl",
        "--user",
        "call",
        "org.kde.kglobalaccel",
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
        "unregister",
        "ss",
        "kwin",
        "PaveSnapBottom",
    ])
    write_shortcut("PaveSnapTop", "none,none,Pave: Snap Top")
    write_shortcut("PaveSnapBottom", "none,none,Pave: Snap Bottom")


def script_loaded():
    result = qdbus(
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting.isScriptLoaded",
        "pave",
    )
    return result.stdout.strip() == "true"


def autostart_enabled():
    return AUTOSTART_FILE.exists()


def set_autostart(enabled):
    if enabled:
        AUTOSTART_FILE.write_text(AUTOSTART_DESKTOP_FILE.read_text(), encoding="utf-8")
    elif AUTOSTART_FILE.exists():
        AUTOSTART_FILE.unlink()


def ensure_pave_active():
    kwrite("Plugins", "paveEnabled", "true")
    if not kread("Script-pave", "gap", ""):
        kwrite("Script-pave", "gap", 10)
    reload_pave_script()
    set_pave_shortcuts_live()


class StatusPill(QLabel):
    def __init__(self, text="", parent=None):
        super().__init__(text, parent)
        self.setAlignment(Qt.AlignCenter)
        self.setMinimumHeight(28)
        self.setStyleSheet(
            "QLabel { border-radius: 4px; padding: 4px 10px;"
            " background: #263238; color: #e7f1f4; }"
        )


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Pave")
        self.setWindowIcon(QIcon(str(ICON_FILE)))
        self.setMinimumSize(500, 360)

        root = QWidget()
        self.setCentralWidget(root)
        layout = QVBoxLayout(root)
        layout.setContentsMargins(20, 18, 20, 20)
        layout.setSpacing(14)

        title = QLabel("Pave")
        title.setStyleSheet("font-size: 26px; font-weight: 700;")
        subtitle = QLabel("Padded Rectangle-style window snapping for KDE")
        subtitle.setStyleSheet("color: #8b989d;")
        layout.addWidget(title)
        layout.addWidget(subtitle)

        status_row = QHBoxLayout()
        self.loaded_status = StatusPill()
        self.enabled_status = StatusPill()
        status_row.addWidget(self.loaded_status)
        status_row.addWidget(self.enabled_status)
        layout.addLayout(status_row)

        settings = QGroupBox("Layout")
        settings_layout = QGridLayout(settings)
        settings_layout.setColumnStretch(1, 1)
        settings_layout.addWidget(QLabel("Padding"), 0, 0)
        self.gap_spin = QSpinBox()
        self.gap_spin.setRange(0, 80)
        self.gap_spin.setSuffix(" px")
        self.gap_spin.setValue(int(kread("Script-pave", "gap", "10")))
        settings_layout.addWidget(self.gap_spin, 0, 1)
        settings_layout.addWidget(QLabel("Cycle"), 1, 0)
        settings_layout.addWidget(QLabel("1/4 -> 1/3 -> 1/2 -> 2/3 -> 3/4"), 1, 1)
        settings_layout.addWidget(QLabel("Shortcuts"), 2, 0)
        settings_layout.addWidget(QLabel("Ctrl+Alt+Left, Ctrl+Alt+Right, Ctrl+Alt+Enter"), 2, 1)
        layout.addWidget(settings)

        startup = QGroupBox("Startup")
        startup_layout = QVBoxLayout(startup)
        self.autostart_check = QCheckBox("Enable Pave at login")
        self.autostart_check.setChecked(autostart_enabled())
        startup_layout.addWidget(self.autostart_check)
        layout.addWidget(startup)

        line = QFrame()
        line.setFrameShape(QFrame.HLine)
        line.setFrameShadow(QFrame.Sunken)
        layout.addWidget(line)

        button_row = QHBoxLayout()
        self.apply_button = QPushButton("Apply")
        self.reload_button = QPushButton("Reload Script")
        self.fix_button = QPushButton("Fix Shortcuts")
        button_row.addWidget(self.apply_button)
        button_row.addWidget(self.reload_button)
        button_row.addWidget(self.fix_button)
        layout.addLayout(button_row)

        self.message = QLabel()
        self.message.setWordWrap(True)
        self.message.setStyleSheet("color: #8b989d;")
        layout.addWidget(self.message)
        layout.addStretch()

        self.apply_button.clicked.connect(self.apply_settings)
        self.reload_button.clicked.connect(self.reload_script)
        self.fix_button.clicked.connect(self.fix_shortcuts)
        self.refresh_status()

    def refresh_status(self):
        enabled = kread("Plugins", "paveEnabled", "false") == "true"
        loaded = script_loaded()
        self.enabled_status.setText("Enabled" if enabled else "Disabled")
        self.loaded_status.setText("Loaded" if loaded else "Not loaded")

    def apply_settings(self):
        try:
            kwrite("Plugins", "paveEnabled", "true")
            kwrite("Script-pave", "gap", self.gap_spin.value())
            set_autostart(self.autostart_check.isChecked())
            reload_pave_script()
            set_pave_shortcuts_live()
            self.refresh_status()
            self.message.setText("Applied settings.")
        except Exception as exc:
            self.message.setText(f"Failed to apply settings: {exc}")

    def reload_script(self):
        try:
            kwrite("Plugins", "paveEnabled", "true")
            reload_pave_script()
            self.refresh_status()
            self.message.setText("Reloaded Pave.")
        except Exception as exc:
            self.message.setText(f"Failed to reload: {exc}")

    def fix_shortcuts(self):
        try:
            set_pave_shortcuts_live()
            self.message.setText("Shortcuts refreshed.")
        except Exception as exc:
            self.message.setText(f"Failed to refresh shortcuts: {exc}")


def main():
    if "--apply" in sys.argv:
        ensure_pave_active()
        return 0

    app = QApplication(sys.argv)
    app.setApplicationName("Pave")
    app.setDesktopFileName("pave")
    window = MainWindow()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
