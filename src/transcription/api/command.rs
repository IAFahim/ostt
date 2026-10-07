use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::TranscriptionConfig;

pub(super) async fn transcribe(
    config: &TranscriptionConfig,
    audio_path: &Path,
) -> anyhow::Result<String> {
    let command = config.endpoint.trim();
    if command.is_empty() {
        anyhow::bail!(
            "external command profile '{}' has no command",
            config.model_id
        );
    }

    let expanded = command.replace("{audio_path}", &shell_quote_path(audio_path));
    tracing::debug!("External command transcription: {expanded}");

    let mut process = shell_command(&expanded);
    #[cfg(windows)]
    process.env("OSTT_EXTERNAL_AUDIO_PATH", audio_path);
    process.kill_on_drop(true);
    let child = process
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| anyhow::anyhow!("failed to start external command: {err}"))?;

    let output = if let Some(timeout_secs) = config.timeout_secs {
        tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("external command timed out after {timeout_secs}s"))??
    } else {
        child.wait_with_output().await?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let summary = if stderr.is_empty() {
            "no stderr".to_string()
        } else {
            stderr
        };
        anyhow::bail!(
            "external command failed with status {}: {}",
            output.status,
            summary
        );
    }

    let transcript = String::from_utf8(output.stdout)
        .map_err(|err| anyhow::anyhow!("external command produced invalid UTF-8: {err}"))?
        .trim()
        .to_string();
    if transcript.is_empty() {
        anyhow::bail!("external backend produced no transcript");
    }

    Ok(transcript)
}

#[cfg(unix)]
fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn shell_quote_path(_path: &Path) -> String {
    // cmd expands variables once. Keep path characters out of the command source.
    "\"%OSTT_EXTERNAL_AUDIO_PATH%\"".to_string()
}

#[cfg(unix)]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    process
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    let mut process = Command::new("cmd.exe");
    process.args(["/D", "/V:OFF", "/S", "/C"]);
    // cmd uses its own quoting rules, rather than the C runtime's argument rules.
    process.raw_arg(format!("\"{command}\""));
    process
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn shell_quote_path_preserves_spaces_and_quotes() {
        let quoted = shell_quote_path(Path::new("/tmp/audio file's clip.mp3"));

        assert_eq!(quoted, "'/tmp/audio file'\\''s clip.mp3'");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_audio_path_preserves_spaces_and_shell_characters() {
        let path = Path::new(r"C:\audio files\clip's %PATH% !OSTT! & | (sample).mp3");
        let command = format!("echo {}", shell_quote_path(path));
        let output = shell_command(&command)
            .env("OSTT_EXTERNAL_AUDIO_PATH", path)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        // cmd echo retains quotes; the path must remain literal, not expand PATH.
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("\"{}\"", path.display())
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_external_command_reads_the_quoted_audio_file() {
        let dir =
            std::env::temp_dir().join(format!("ostt_external_command_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("音声 clip's %PATH% ! & (sample).mp3");
        std::fs::write(&path, b"expected transcript").unwrap();
        let output = shell_command(&format!("type {}", shell_quote_path(&path)))
            .env("OSTT_EXTERNAL_AUDIO_PATH", &path)
            .output()
            .await
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"expected transcript");
    }
}
