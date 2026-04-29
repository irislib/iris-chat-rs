use std::fs;
use std::io;
use std::path::PathBuf;

const AUTOSTART_FILE: &str = "iris-chat.desktop";
pub const BACKGROUND_ARG: &str = "--background";

pub fn is_supported() -> bool {
    autostart_dir().is_some()
}

pub fn set_enabled(enabled: bool) -> io::Result<()> {
    let Some(dir) = autostart_dir() else {
        return Ok(());
    };
    let path = dir.join(AUTOSTART_FILE);
    if enabled {
        fs::create_dir_all(&dir)?;
        fs::write(path, desktop_entry())?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn autostart_dir() -> Option<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("autostart"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/autostart"))
}

fn desktop_entry() -> String {
    let exec = std::env::current_exe()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| "iris-chat".to_string());
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Iris Chat\n\
         Exec={} {}\n\
         Icon=iris-chat\n\
         Terminal=false\n\
         Categories=Network;InstantMessaging;\n\
         X-GNOME-Autostart-enabled=true\n\
         NoDisplay=true\n",
        desktop_exec_arg(&exec),
        BACKGROUND_ARG
    )
}

fn desktop_exec_arg(value: &str) -> String {
    if !value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '`' | '$'))
    {
        return value.to_string();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    format!("\"{}\"", escaped)
}
