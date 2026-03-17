use anyhow::Result;
use std::path::{Path, PathBuf};

const APP_ID: &str = "com.print-agent";
#[allow(dead_code)]
const APP_NAME: &str = "PrintAgent";

fn exe_path() -> Result<PathBuf> {
    Ok(std::env::current_exe()?)
}

pub fn set_enabled(enabled: bool) {
    let result = if enabled {
        exe_path().and_then(|exe| platform::register(&exe))
    } else {
        platform::unregister()
    };
    match result {
        Ok(()) => tracing::info!("Auto-start {}", if enabled { "enabled" } else { "disabled" }),
        Err(e) => tracing::error!("Failed to set auto-start: {e}"),
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    fn plist_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join("Library/LaunchAgents")
            .join(format!("{APP_ID}.plist"))
    }

    pub fn register(exe: &Path) -> Result<()> {
        let path = plist_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{APP_ID}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#,
            exe = exe.display()
        );
        std::fs::write(&path, plist)?;
        Ok(())
    }

    pub fn unregister() -> Result<()> {
        let path = plist_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    const REG_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn register(exe: &Path) -> Result<()> {
        use std::process::Command;

        let exe_str = exe.to_string_lossy();
        let status = Command::new("reg")
            .args(["add", &format!(r"HKCU\{REG_RUN_KEY}"), "/v", APP_NAME, "/d", &exe_str, "/f"])
            .output()?;

        if status.status.success() {
            Ok(())
        } else {
            anyhow::bail!("reg add failed: {}", String::from_utf8_lossy(&status.stderr))
        }
    }

    pub fn unregister() -> Result<()> {
        use std::process::Command;

        let _ = Command::new("reg")
            .args(["delete", &format!(r"HKCU\{REG_RUN_KEY}"), "/v", APP_NAME, "/f"])
            .output();
        Ok(())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::*;

    fn desktop_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("autostart")
            .join(format!("{APP_ID}.desktop"))
    }

    pub fn register(exe: &Path) -> Result<()> {
        let path = desktop_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = format!(
            "[Desktop Entry]\nType=Application\nName={APP_NAME}\nExec={exe}\nX-GNOME-Autostart-enabled=true\n",
            exe = exe.display()
        );
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn unregister() -> Result<()> {
        let path = desktop_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
