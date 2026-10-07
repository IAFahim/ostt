//! Clipboard utilities for ostt.
//!
//! Handles Unicode clipboard access on Windows, pbcopy (macOS), wl-copy (Wayland), or xclip (X11).

#[cfg(not(windows))]
use anyhow::Context;
#[cfg(not(windows))]
use std::io::Write;
#[cfg(not(windows))]
use std::process::{Command, Stdio};

/// Copies text using the Windows clipboard, pbcopy (macOS), wl-copy (Wayland), or xclip (X11).
///
/// # Errors
/// - If no clipboard tool is available or the selected backend fails.
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    set_clipboard(text)
}

pub(crate) fn read_clipboard() -> anyhow::Result<String> {
    #[cfg(windows)]
    {
        clipboard_win::get_clipboard_string()
            .map_err(|error| anyhow::anyhow!("Failed to read Windows Unicode clipboard: {error}"))
    }

    #[cfg(target_os = "macos")]
    {
        read_command("pbpaste", &[])
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            if let Ok(text) = read_command("wl-paste", &["--no-newline"]) {
                return Ok(text);
            }
        }

        read_command("xclip", &["-selection", "clipboard", "-out"])
    }
}

pub(crate) fn set_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        clipboard_win::set_clipboard_string(text)
            .map_err(|error| anyhow::anyhow!("Failed to write Windows Unicode clipboard: {error}"))
    }

    #[cfg(target_os = "macos")]
    {
        write_command("pbcopy", &[], text)
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok()
            && write_command("wl-copy", &["--type", "text/plain", "--trim-newline"], text).is_ok()
        {
            return Ok(());
        }

        write_command("xclip", &["-selection", "clipboard", "-in"], text)
    }
}

#[cfg(not(windows))]
fn read_command(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run {program}"))?;
    if !output.status.success() {
        anyhow::bail!("{program} failed with status {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(not(windows))]
fn write_command(program: &str, args: &[&str], text: &str) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to run {program}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open {program} stdin"))?;
    stdin
        .write_all(text.as_bytes())
        .with_context(|| format!("Failed to write to {program}"))?;
    drop(stdin);

    let status = child
        .wait()
        .with_context(|| format!("Failed to wait for {program}"))?;
    if !status.success() {
        anyhow::bail!("{program} failed with status {status}");
    }
    Ok(())
}
