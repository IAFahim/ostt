//! Application directory helpers using XDG paths or native Windows app data.
//!
//! All path construction for ostt's data, config, log, and runtime directories goes here.
//! Callers should not build these paths inline.

use std::path::PathBuf;

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

/// Returns `~/.local/share/ostt`, or LocalAppData/ostt on Windows (XDG_DATA_HOME overrides either).
///
/// This is where model files, recordings, history, and the daemon socket live.
pub(crate) fn data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("ostt");
    }
    #[cfg(windows)]
    if let Some(dir) = dirs::data_local_dir() {
        return dir.join("ostt");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".local")
        .join("share")
        .join("ostt")
}

/// Returns the recordings directory and ensures it exists.
pub(crate) fn recordings_dir() -> Result<PathBuf, anyhow::Error> {
    let dir = data_dir().join("recordings");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Returns `~/.config/ostt`, or RoamingAppData/ostt on Windows (XDG_CONFIG_HOME overrides either).
pub(crate) fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("ostt");
    }
    #[cfg(windows)]
    if let Some(dir) = dirs::config_dir() {
        return dir.join("ostt");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
        .join("ostt")
}

/// Returns `~/.config/ostt/ostt.toml` and ensures the parent directory exists.
///
/// # Errors
/// - If the home directory cannot be determined
/// - If the config directory cannot be created
pub(crate) fn config_path() -> Result<PathBuf, std::io::Error> {
    let path = config_dir().join("ostt.toml");
    std::fs::create_dir_all(path.parent().unwrap())?;
    Ok(path)
}

/// Returns `~/.local/state/ostt` (XDG_STATE_HOME/ostt if set) and ensures it exists.
///
/// This is where log files are written.
///
/// # Errors
/// - If the home directory cannot be determined
/// - If the directory cannot be created
pub(crate) fn log_dir() -> Result<PathBuf, anyhow::Error> {
    let dir = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("ostt")
    } else {
        #[cfg(windows)]
        let state_dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine local data directory"))?;
        #[cfg(not(windows))]
        let state_dir = home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
            .join(".local")
            .join("state");
        state_dir.join("ostt")
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Returns the per-user runtime directory for transient files.
///
/// On Linux this follows XDG_RUNTIME_DIR. On platforms without XDG runtime
/// directories, it falls back to the system temp directory with a per-user name.
pub(crate) fn runtime_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("ostt");
    }

    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| {
            home_dir()
                .and_then(|home| {
                    home.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "unknown".to_string())
        });

    std::env::temp_dir().join(format!("ostt-{user}"))
}

/// Returns the active recorder PID file path.
pub(crate) fn recording_pid_path() -> PathBuf {
    runtime_dir().join("recording.pid")
}
