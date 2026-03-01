// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle --preset "Name" CLI arg: send D-Bus call to running instance, then exit
    if let Some(pos) = args.iter().position(|a| a == "--preset") {
        if let Some(name) = args.get(pos + 1) {
            match std::process::Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=com.pave.app",
                    "/com/pave/Presets",
                    "com.pave.Presets.Activate",
                    &format!("string:{}", name),
                ])
                .status()
            {
                Ok(status) if status.success() => {
                    println!("Activated preset: {name}");
                }
                Ok(_) => {
                    eprintln!("Failed to activate preset (is Pave running?)");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Failed to send D-Bus message: {e}");
                    std::process::exit(1);
                }
            }
            return;
        } else {
            eprintln!("Usage: pave --preset \"Name\"");
            std::process::exit(1);
        }
    }

    pave_lib::run()
}
