//! Replay a previous recording from history using the system audio player.

use crate::recording::recording_history;
#[cfg(not(windows))]
use std::process::Command;

/// Plays back a previous recording using the system's best available audio player.
///
/// On macOS: Uses `open` command to open with default application
/// On Linux: Tries dedicated audio players first (mpv, cvlc, ffplay, paplay) for better UX,
///           then falls back to xdg-open if none are available
///
/// # Arguments
/// * `recording_index` - Optional index of recording to play (1 = most recent, None = most recent)
pub async fn handle_replay(recording_index: Option<usize>) -> Result<(), anyhow::Error> {
    tracing::info!("=== ostt Replay Command ===");

    let all_recordings = recording_history::get_all_recordings()?;

    if all_recordings.is_empty() {
        return Err(anyhow::anyhow!("No recordings found in history"));
    }

    // Get recording by index (1-indexed, where 1 is most recent)
    let index = recording_index.unwrap_or(1);
    if index < 1 || index > all_recordings.len() {
        return Err(anyhow::anyhow!(
            "Recording index out of range. Available recordings: 1-{}",
            all_recordings.len()
        ));
    }

    let audio_path = &all_recordings[index - 1];

    if !audio_path.exists() {
        return Err(anyhow::anyhow!(
            "Audio file not found: {}",
            audio_path.display()
        ));
    }

    tracing::info!("Playing recording #{}", index);
    tracing::info!("Audio file path: {}", audio_path.display());

    // Platform-specific audio player invocation
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let path: Vec<u16> = audio_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        anyhow::ensure!(result > 32, "Failed to open the Windows audio player (error {result}). Set a default app for this audio format.");
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(audio_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to open audio player: {e}"))?
            .wait()
            .map_err(|e| anyhow::anyhow!("Audio player error: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        // Prefer terminal-friendly playback commands to avoid GUI/DBus noise in stdout/stderr.
        let players = [
            ("mpv", vec!["--really-quiet", "--no-video"]),
            ("cvlc", vec!["--play-and-exit", "--no-video"]),
            ("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet"]),
            ("paplay", vec![]),
        ];
        let mut played = false;

        for (player, args) in players {
            if let Ok(mut child) = Command::new(player).args(args).arg(audio_path).spawn() {
                let _ = child.wait();
                played = true;
                break;
            }
        }

        // If no dedicated player found, try xdg-open as fallback
        if !played {
            if let Ok(mut child) = Command::new("xdg-open").arg(audio_path).spawn() {
                child
                    .wait()
                    .map_err(|e| anyhow::anyhow!("Audio player error: {e}"))?;
                played = true;
            }
        }

        if !played {
            return Err(anyhow::anyhow!(
                "No audio player found. Install mpv, vlc, ffplay, or paplay"
            ));
        }
    }

    tracing::debug!("Playback finished for recording #{}", index);
    Ok(())
}
