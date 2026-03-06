use super::{MonitorInfo, WindowInfo};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot, Mutex};
use zbus::proxy::Proxy;
use zbus::{interface, Connection};

const KWIN_SCRIPT_METADATA: &str =
    include_str!("../../kwin-scripts/almostmaximize/metadata.json");
const KWIN_SCRIPT_MAIN_JS: &str =
    include_str!("../../kwin-scripts/almostmaximize/contents/code/main.js");

/// D-Bus object that receives data callbacks from KWin scripts
struct PaveScriptReceiver {
    sender: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

#[interface(name = "com.pave.ScriptReceiver")]
impl PaveScriptReceiver {
    async fn receive_result(&self, result: &str) {
        log::info!("Received KWin script callback ({} bytes)", result.len());
        let mut sender_lock = self.sender.lock().await;
        if let Some(sender) = sender_lock.take() {
            let _ = sender.send(result.to_string());
        } else {
            log::warn!("No listener waiting for script callback");
        }
    }
}

/// D-Bus object that receives shortcut press callbacks from KWin scripts
struct PaveShortcutReceiver {
    sender: broadcast::Sender<String>,
}

#[interface(name = "com.pave.Shortcuts")]
impl PaveShortcutReceiver {
    async fn shortcut_pressed(&self, action: &str) {
        log::info!("Shortcut pressed via KWin: {action}");
        let _ = self.sender.send(action.to_string());
    }
}

/// D-Bus object that receives resize event callbacks from KWin scripts
struct PaveResizeReceiver {
    sender: broadcast::Sender<String>,
}

#[interface(name = "com.pave.ResizeEvents")]
impl PaveResizeReceiver {
    async fn window_resized(&self, payload: &str) {
        log::info!("Resize event received ({} bytes)", payload.len());
        let _ = self.sender.send(payload.to_string());
    }
}

/// D-Bus object that receives window lifecycle events from KWin scripts
struct PaveWindowEventReceiver {
    sender: broadcast::Sender<String>,
}

#[interface(name = "com.pave.WindowEvents")]
impl PaveWindowEventReceiver {
    async fn window_removed(&self, window_id: &str) {
        log::info!("Window removed: {window_id}");
        let _ = self.sender.send(window_id.to_string());
    }
}

/// D-Bus object that receives preset activation requests (from CLI or CiderDeck)
struct PavePresetReceiver {
    sender: broadcast::Sender<String>,
}

#[interface(name = "com.pave.Presets")]
impl PavePresetReceiver {
    async fn activate(&self, name: &str) {
        log::info!("Preset activation requested via D-Bus: {name}");
        let _ = self.sender.send(name.to_string());
    }

    async fn throw_monitor(&self) {
        log::info!("Throw to monitor requested via D-Bus");
        let _ = self.sender.send("__throw__".to_string());
    }

    async fn resurface(&self) {
        log::info!("Resurface zones requested via D-Bus");
        let _ = self.sender.send("__resurface__".to_string());
    }
}

pub struct KWinBackend {
    connection: Connection,
    result_sender: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

impl KWinBackend {
    /// Create a new KWin backend. Returns the backend and broadcast receivers
    /// for shortcut press events, resize events, and preset activation requests
    /// (kept separate so the backend can be wrapped in Arc).
    pub async fn new() -> Result<
        (
            Self,
            broadcast::Receiver<String>,
            broadcast::Receiver<String>,
            broadcast::Receiver<String>,
            broadcast::Receiver<String>,
        ),
        String,
    > {
        let connection = Connection::session()
            .await
            .map_err(|e| format!("Failed to connect to D-Bus session bus: {e}"))?;

        let result_sender: Arc<Mutex<Option<oneshot::Sender<String>>>> =
            Arc::new(Mutex::new(None));

        let (shortcut_tx, shortcut_rx) = broadcast::channel(16);
        let (resize_tx, resize_rx) = broadcast::channel(16);
        let (preset_tx, preset_rx) = broadcast::channel(16);
        let (window_event_tx, window_event_rx) = broadcast::channel(16);

        // Register D-Bus objects for script data callbacks, shortcut presses, resize events, and window lifecycle
        let script_receiver = PaveScriptReceiver {
            sender: result_sender.clone(),
        };
        let shortcut_receiver = PaveShortcutReceiver {
            sender: shortcut_tx,
        };
        let resize_receiver = PaveResizeReceiver {
            sender: resize_tx,
        };
        let preset_receiver = PavePresetReceiver {
            sender: preset_tx,
        };
        let window_event_receiver = PaveWindowEventReceiver {
            sender: window_event_tx,
        };

        connection
            .object_server()
            .at("/com/pave/ScriptReceiver", script_receiver)
            .await
            .map_err(|e| format!("Failed to register script receiver: {e}"))?;

        connection
            .object_server()
            .at("/com/pave/Shortcuts", shortcut_receiver)
            .await
            .map_err(|e| format!("Failed to register shortcut receiver: {e}"))?;

        connection
            .object_server()
            .at("/com/pave/ResizeEvents", resize_receiver)
            .await
            .map_err(|e| format!("Failed to register resize receiver: {e}"))?;

        connection
            .object_server()
            .at("/com/pave/Presets", preset_receiver)
            .await
            .map_err(|e| format!("Failed to register preset receiver: {e}"))?;

        connection
            .object_server()
            .at("/com/pave/WindowEvents", window_event_receiver)
            .await
            .map_err(|e| format!("Failed to register window event receiver: {e}"))?;

        // Request a well-known name so KWin scripts can find us
        connection
            .request_name("com.pave.app")
            .await
            .map_err(|e| format!("Failed to request D-Bus name: {e}"))?;

        Ok((
            Self {
                connection,
                result_sender,
            },
            shortcut_rx,
            resize_rx,
            preset_rx,
            window_event_rx,
        ))
    }

    async fn build_proxy(&self, dest: &str, path: &str, iface: &str) -> Result<Proxy<'_>, String> {
        Proxy::new(
            &self.connection,
            dest.to_string(),
            path.to_string(),
            iface.to_string(),
        )
        .await
        .map_err(|e| format!("Failed to build D-Bus proxy ({dest} {path}): {e}"))
    }

    /// Execute a KWin script (fire and forget, no output).
    async fn run_kwin_script(&self, script: &str) -> Result<(), String> {
        let proxy = self
            .build_proxy("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
            .await?;

        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("pave_kwin_script.js");
        std::fs::write(&script_path, script)
            .map_err(|e| format!("Failed to write temp script: {e}"))?;

        let script_path_str = script_path
            .to_str()
            .ok_or("Invalid temp path")?
            .to_string();

        // Unload any previous instance
        let _: Result<bool, _> = proxy.call("unloadScript", &("pave_temp",)).await;

        let script_id: i32 = proxy
            .call("loadScript", &(script_path_str.as_str(), "pave_temp"))
            .await
            .map_err(|e| format!("Failed to load KWin script: {e}"))?;

        let script_obj_path = format!("/Scripting/Script{script_id}");
        let script_proxy = self
            .build_proxy("org.kde.KWin", &script_obj_path, "org.kde.kwin.Script")
            .await?;

        script_proxy
            .call_noreply("run", &())
            .await
            .map_err(|e| format!("Failed to run KWin script: {e}"))?;

        // Delay for execution before cleanup
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Cleanup
        let _ = script_proxy.call_noreply("stop", &()).await;
        let _: Result<bool, _> = proxy.call("unloadScript", &("pave_temp",)).await;
        let _ = std::fs::remove_file(&script_path);

        Ok(())
    }

    /// Execute a KWin script that calls back to our D-Bus service with the result.
    async fn run_kwin_script_with_output(&self, inner_script: &str) -> Result<String, String> {
        log::debug!("Running KWin script with output callback");
        // Set up the oneshot channel to receive the result
        let (tx, rx) = oneshot::channel();
        {
            let mut sender_lock = self.result_sender.lock().await;
            *sender_lock = Some(tx);
        }

        // Wrap the script to call back to our D-Bus service
        let script = format!(
            r#"
            (function() {{
                var data = (function() {{
                    {inner_script}
                }})();
                callDBus("com.pave.app", "/com/pave/ScriptReceiver",
                         "com.pave.ScriptReceiver", "ReceiveResult",
                         String(data));
            }})();
            "#
        );

        let proxy = self
            .build_proxy("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
            .await?;

        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("pave_kwin_output.js");
        std::fs::write(&script_path, &script)
            .map_err(|e| format!("Failed to write temp script: {e}"))?;

        let script_path_str = script_path
            .to_str()
            .ok_or("Invalid temp path")?
            .to_string();

        // Unload any previous instance
        let _: Result<bool, _> = proxy.call("unloadScript", &("pave_output",)).await;

        let script_id: i32 = proxy
            .call("loadScript", &(script_path_str.as_str(), "pave_output"))
            .await
            .map_err(|e| format!("Failed to load KWin script: {e}"))?;

        let script_obj_path = format!("/Scripting/Script{script_id}");
        let script_proxy = self
            .build_proxy("org.kde.KWin", &script_obj_path, "org.kde.kwin.Script")
            .await?;

        script_proxy
            .call_noreply("run", &())
            .await
            .map_err(|e| format!("Failed to run KWin script: {e}"))?;

        // Wait for the callback with a timeout
        let result = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .map_err(|_| "KWin script callback timed out".to_string())?
            .map_err(|_| "KWin script callback channel closed".to_string())?;

        // Cleanup — use call_noreply so we don't wait for responses
        let _ = script_proxy.call_noreply("stop", &()).await;
        let _ = proxy
            .call_noreply("unloadScript", &("pave_output",))
            .await;
        let _ = std::fs::remove_file(&script_path);

        Ok(result)
    }

    /// Ensure key bindings are set in kglobalaccel.
    /// Uses the original AlmostMaximize/SnapLeft/SnapRight shortcut names
    /// since those already have working Wayland key grabs from the installed
    /// KWin script at ~/.local/share/kwin/scripts/almostmaximize/.
    /// Also clears any conflicting Pave* shortcuts.
    async fn ensure_shortcut_keys(&self) -> Result<(), String> {
        let kga_proxy = self
            .build_proxy(
                "org.kde.kglobalaccel",
                "/kglobalaccel",
                "org.kde.KGlobalAccel",
            )
            .await?;

        // Clear conflicting Pave* shortcuts (from the disabled pave_shortcuts script)
        // Clear shortcuts from other components that conflict with our key bindings
        // (cider-linux used Ctrl+Alt+Up/Down for TileTop/TileBottom)
        let conflict_components = ["kwin", "cider-linux"];
        let conflict_names = ["TileTop", "TileBottom"];
        for component in conflict_components {
            for name in conflict_names {
                let action_id: Vec<&str> = vec![component, name, "", ""];
                let empty_keys: Vec<i32> = vec![];
                let _: Result<Vec<i32>, _> = kga_proxy
                    .call("setShortcut", &(&action_id, &empty_keys, 0u32))
                    .await;
            }
        }

        let clear_shortcuts = ["PaveAlmostMaximize", "PaveSnapLeft", "PaveSnapRight", "PaveSnapUp", "PaveSnapDown", "PaveRestoreWindow", "PaveGrowWindow", "PaveShrinkWindow", "PaveTabCycle"];
        for name in clear_shortcuts {
            let action_id: Vec<&str> = vec!["kwin", name, "KWin", ""];
            let empty_keys: Vec<i32> = vec![];
            let _: Result<Vec<i32>, _> = kga_proxy
                .call("setShortcut", &(&action_id, &empty_keys, 0u32))
                .await;
        }

        // Set the actual shortcuts
        // Ctrl+Alt+Return = 0x04000000 | 0x08000000 | 0x01000004
        // Ctrl+Alt+Left   = 0x04000000 | 0x08000000 | 0x01000012
        // Ctrl+Alt+Right  = 0x04000000 | 0x08000000 | 0x01000014
        let shortcuts: [(&str, &str, i32); 9] = [
            ("AlmostMaximize", "Almost Maximize Window", 0x0D000004),
            ("SnapLeft", "Snap Window Left with Gap", 0x0D000012),
            ("SnapRight", "Snap Window Right with Gap", 0x0D000014),
            ("SnapUp", "Snap Window Up (Quarter)", 0x0D000013),
            ("SnapDown", "Snap Window Down (Quarter)", 0x0D000015),
            ("RestoreWindow", "Restore Window to Pre-Snap Size", 0x0C00005A),
            ("GrowWindow", "Grow Window by 10%", 0x0C00003D),
            ("ShrinkWindow", "Shrink Window by 10%", 0x0C00002D),
            ("TabCycle", "Cycle Tabbed Windows in Zone", 0x0D000001),
        ];
        for (name, friendly, key) in shortcuts {
            let action_id: Vec<&str> = vec!["kwin", name, "KWin", friendly];
            let keys: Vec<i32> = vec![key];

            // Best-effort: registerShortcut in the KWin script usually handles this,
            // but setShortcut ensures the binding if it wasn't set by the script
            let result: Result<Vec<i32>, _> = kga_proxy
                .call("setShortcut", &(&action_id, &keys, 0u32))
                .await;
            match result {
                Ok(actual) => log::info!("setShortcut for {name}: key={key:#x}, result={actual:?}"),
                Err(e) => log::warn!("setShortcut for {name} failed: {e}"),
            }
        }

        log::info!("Ensured shortcut key bindings in kglobalaccel");
        Ok(())
    }

    /// Ensure the bundled KWin script is installed at the expected location.
    /// Writes (or updates) the script files if they're missing or out of date.
    pub fn ensure_kwin_script_installed() -> Result<(), String> {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let base = std::path::PathBuf::from(format!(
            "{}/.local/share/kwin/scripts/almostmaximize",
            home
        ));
        let code_dir = base.join("contents/code");
        let metadata_path = base.join("metadata.json");
        let main_js_path = code_dir.join("main.js");

        // Check if already up to date
        if main_js_path.exists() && metadata_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(&main_js_path) {
                if existing == KWIN_SCRIPT_MAIN_JS {
                    log::info!("KWin script already up to date");
                    return Ok(());
                }
            }
            log::info!("KWin script outdated, updating");
        } else {
            log::info!("KWin script missing, installing");
        }

        std::fs::create_dir_all(&code_dir)
            .map_err(|e| format!("Failed to create KWin script dir: {e}"))?;
        std::fs::write(&metadata_path, KWIN_SCRIPT_METADATA)
            .map_err(|e| format!("Failed to write metadata.json: {e}"))?;
        std::fs::write(&main_js_path, KWIN_SCRIPT_MAIN_JS)
            .map_err(|e| format!("Failed to write main.js: {e}"))?;

        log::info!("KWin script installed to {}", base.display());
        Ok(())
    }

    /// Ensure shortcuts are properly configured.
    /// Reloads the almostmaximize KWin script to register shortcut handlers,
    /// and ensures key bindings are set in kglobalaccel.
    pub async fn register_all_shortcuts(&self) -> Result<(), String> {
        Self::ensure_kwin_script_installed()?;

        // Unregister SnapUp/SnapDown so registerShortcut creates them fresh
        // with proper defaults (clears stale entries with key=0)
        let kga_proxy = self
            .build_proxy(
                "org.kde.kglobalaccel",
                "/kglobalaccel",
                "org.kde.KGlobalAccel",
            )
            .await?;

        let fresh_shortcuts = ["SnapUp", "SnapDown", "RestoreWindow", "GrowWindow", "ShrinkWindow", "TabCycle"];
        for name in fresh_shortcuts {
            // Check if current key is 0 (unassigned) and if so, unregister
            let action_id: Vec<&str> = vec!["kwin", name, "KWin", ""];
            let current: Result<Vec<i32>, _> = kga_proxy
                .call("shortcut", &(&action_id,))
                .await;
            if let Ok(keys) = current {
                if keys.is_empty() || keys.iter().all(|k| *k == 0) {
                    let _: Result<bool, _> = kga_proxy
                        .call("unregister", &("kwin", name))
                        .await;
                    log::info!("Unregistered stale {name} (had key=0)");
                }
            }
        }

        // Load the KWin script first — registerShortcut() creates the shortcut
        // entries in kglobalaccel. Only then can we assign key bindings.
        let scripting_proxy = self
            .build_proxy("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
            .await?;

        // Unload any previous instance
        let _: Result<bool, _> = scripting_proxy
            .call("unloadScript", &("almostmaximize",))
            .await;

        // Load from installed location
        let script_path = format!(
            "{}/.local/share/kwin/scripts/almostmaximize/contents/code/main.js",
            std::env::var("HOME").unwrap_or_default()
        );

        let script_id: i32 = scripting_proxy
            .call("loadScript", &(script_path.as_str(), "almostmaximize"))
            .await
            .map_err(|e| format!("Failed to load almostmaximize script: {e}"))?;

        let script_obj_path = format!("/Scripting/Script{script_id}");
        let script_proxy = self
            .build_proxy("org.kde.KWin", &script_obj_path, "org.kde.kwin.Script")
            .await?;

        script_proxy
            .call_noreply("run", &())
            .await
            .map_err(|e| format!("Failed to run almostmaximize script: {e}"))?;

        log::info!("Registered all global shortcuts via KWin script");

        // Give KWin time to process the registerShortcut calls
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Now assign key bindings (shortcuts exist in kglobalaccel at this point)
        self.ensure_shortcut_keys().await?;

        Ok(())
    }

    pub async fn get_windows(&self) -> Result<Vec<WindowInfo>, String> {
        let script = r#"
            var result = [];
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {
                var c = clients[i];
                if (c.normalWindow) {
                    result.push({
                        id: c.internalId.toString(),
                        title: c.caption,
                        x: Math.round(c.frameGeometry.x),
                        y: Math.round(c.frameGeometry.y),
                        width: Math.round(c.frameGeometry.width),
                        height: Math.round(c.frameGeometry.height),
                        maximized: !!(c.maximizedHorizontally && c.maximizedVertically),
                        minimized: !!c.minimized,
                        resource_class: c.resourceClass || "",
                        active: (c === workspace.activeWindow),
                        desktop: c.desktops.length > 0 ? c.desktops[0].x11DesktopNumber : -1,
                        screen: c.output ? c.output.name : ""
                    });
                }
            }
            return JSON.stringify(result);
        "#;

        let output = self.run_kwin_script_with_output(script).await?;
        if output.is_empty() || output == "undefined" {
            return Ok(Vec::new());
        }
        serde_json::from_str(&output).map_err(|e| format!("Failed to parse window list: {e}"))
    }

    pub async fn get_monitors(&self) -> Result<Vec<MonitorInfo>, String> {
        let script = r#"
            var result = [];
            var screens = workspace.screens;
            for (var i = 0; i < screens.length; i++) {
                var s = screens[i];
                var geom = workspace.clientArea(KWin.PlacementArea, s, workspace.currentDesktop);
                result.push({
                    name: s.name,
                    x: Math.round(geom.x),
                    y: Math.round(geom.y),
                    width: Math.round(geom.width),
                    height: Math.round(geom.height)
                });
            }
            return JSON.stringify(result);
        "#;

        let output = self.run_kwin_script_with_output(script).await?;
        if output.is_empty() || output == "undefined" {
            return Ok(Vec::new());
        }
        serde_json::from_str(&output).map_err(|e| format!("Failed to parse monitor list: {e}"))
    }

    pub async fn move_window(
        &self,
        window_id: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), String> {
        log::info!("move_window: id={window_id} -> ({x}, {y}, {w}x{h})");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    c.frameGeometry = {{
                        x: {x},
                        y: {y},
                        width: {w},
                        height: {h}
                    }};
                    return "moved";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("move_window result: {result}");
        Ok(())
    }

    pub async fn get_active_window(&self) -> Result<Option<WindowInfo>, String> {
        let script = r#"
            var c = workspace.activeWindow;
            if (c && c.normalWindow) {
                return JSON.stringify({
                    id: c.internalId.toString(),
                    title: c.caption,
                    x: Math.round(c.frameGeometry.x),
                    y: Math.round(c.frameGeometry.y),
                    width: Math.round(c.frameGeometry.width),
                    height: Math.round(c.frameGeometry.height),
                    maximized: !!(c.maximizedHorizontally && c.maximizedVertically),
                    minimized: !!c.minimized,
                    resource_class: c.resourceClass || "",
                    active: true,
                    desktop: c.desktops.length > 0 ? c.desktops[0].x11DesktopNumber : -1,
                    screen: c.output ? c.output.name : ""
                });
            } else {
                return "null";
            }
        "#;

        let output = self.run_kwin_script_with_output(script).await?;
        if output.is_empty() || output == "null" || output == "undefined" {
            return Ok(None);
        }
        let info: WindowInfo =
            serde_json::from_str(&output).map_err(|e| format!("Failed to parse window: {e}"))?;
        Ok(Some(info))
    }

    pub async fn maximize_window(&self, window_id: &str) -> Result<(), String> {
        log::info!("maximize_window: id={window_id}");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    c.setMaximize(true, true);
                    return "maximized";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("maximize_window result: {result}");
        Ok(())
    }

    fn write_kwin_config(key: &str, value: &str) -> Result<(), String> {
        let status = std::process::Command::new("kwriteconfig6")
            .args(["--file", "kwinrc", "--group", "Round-Corners", "--key", key, value])
            .status()
            .map_err(|e| format!("Failed to run kwriteconfig6: {e}"))?;
        if !status.success() {
            return Err(format!("kwriteconfig6 failed for key {key}"));
        }
        Ok(())
    }

    /// One-time setup of static ShapeCorners keys (outlines off, tiled rounding on).
    pub fn ensure_shapecorners_defaults() -> Result<(), String> {
        let static_keys: &[(&str, &str)] = &[
            ("DisableRoundTile", "false"),
            ("DisableOutlineTile", "false"),
            ("OutlineThickness", "0.0"),
            ("InactiveOutlineThickness", "0.0"),
            ("SecondOutlineThickness", "0.0"),
            ("InactiveSecondOutlineThickness", "0.0"),
            ("ActiveOutlineUseCustom", "false"),
            ("InactiveOutlineUseCustom", "false"),
            ("ActiveSecondOutlineUseCustom", "false"),
            ("InactiveSecondOutlineUseCustom", "false"),
        ];
        for (key, value) in static_keys {
            Self::write_kwin_config(key, value)?;
        }
        Ok(())
    }

    pub async fn apply_corner_radius(&self, radius: u32) -> Result<(), String> {
        // Write radius values to kwinrc [Round-Corners]
        Self::write_kwin_config("Size", &radius.to_string())?;
        Self::write_kwin_config("InactiveCornerRadius", &radius.to_string())?;

        // Ensure the effect is loaded, then reconfigure
        let proxy = self
            .build_proxy("org.kde.KWin", "/Effects", "org.kde.kwin.Effects")
            .await?;
        let _: Result<bool, _> = proxy
            .call("loadEffect", &("kwin4_effect_shapecorners",))
            .await;
        let _: Result<(), _> = proxy
            .call("reconfigureEffect", &("kwin4_effect_shapecorners",))
            .await;

        // Force KWin to reconfigure globally so all windows repaint immediately
        let kwin_proxy = self
            .build_proxy("org.kde.KWin", "/KWin", "org.kde.KWin")
            .await?;
        let _: Result<(), _> = kwin_proxy.call("reconfigure", &()).await;

        log::info!("Applied corner radius: {radius}px");
        Ok(())
    }

    pub async fn minimize_window(&self, window_id: &str) -> Result<(), String> {
        log::info!("minimize_window: id={window_id}");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    c.minimized = true;
                    return "minimized";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("minimize_window result: {result}");
        Ok(())
    }

    pub async fn unminimize_window(&self, window_id: &str) -> Result<(), String> {
        log::info!("unminimize_window: id={window_id}");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    c.minimized = false;
                    return "unminimized";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("unminimize_window result: {result}");
        Ok(())
    }

    pub async fn activate_window(&self, window_id: &str) -> Result<(), String> {
        log::info!("activate_window: id={window_id}");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    workspace.activeWindow = c;
                    return "activated";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("activate_window result: {result}");
        Ok(())
    }

    pub async fn unmaximize_window(&self, window_id: &str) -> Result<(), String> {
        log::info!("unmaximize_window: id={window_id}");
        let script = format!(
            r#"
            var clients = workspace.stackingOrder;
            for (var i = 0; i < clients.length; i++) {{
                var c = clients[i];
                if (c.internalId.toString() === "{window_id}") {{
                    c.setMaximize(false, false);
                    if (c.tile) c.tile = null;
                    return "unmaximized";
                }}
            }}
            return "not_found";
            "#
        );

        let result = self.run_kwin_script_with_output(&script).await?;
        log::info!("unmaximize_window result: {result}");
        Ok(())
    }
}
