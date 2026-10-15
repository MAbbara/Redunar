use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

const THUMBNAIL_MAX_BYTES: usize = 256 * 1024;
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Serialize)]
pub struct Clip {
    file_name: String,
    title: String,
    bytes: u64,
    modified_unix_ns: String,
    /// Game that recorded the clip, when local evidence resolves it. The UI
    /// falls back to "Game not recorded" only when this is null.
    game_name: Option<String>,
    favorite: bool,
}
#[tauri::command]
pub fn replay_clips() -> Result<Vec<Clip>, String> {
    let service = crate::backend::service();
    // A save revision is published by the replay worker before the one-second
    // session supervisor necessarily observes it. Flush attribution before the
    // inventory join so the first UI refresh already carries the game name.
    service
        .game_session_coordinator()
        .flush_completed_clip_attribution();
    Ok(service
        .replay_clip_inventory()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(Into::into)
        .collect())
}

impl From<redunar_daemon::ReplayClipEntry> for Clip {
    fn from(clip: redunar_daemon::ReplayClipEntry) -> Self {
        Self {
            title: Path::new(&clip.file_name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            file_name: clip.file_name,
            bytes: clip.bytes,
            modified_unix_ns: clip.modified_unix_ns.to_string(),
            game_name: clip.game_name,
            favorite: clip.favorite,
        }
    }
}

fn regular_clip_path(directory: &Path, file_name: &str) -> Result<PathBuf, String> {
    if Path::new(file_name).file_name().and_then(|s| s.to_str()) != Some(file_name) {
        return Err("Invalid clip identifier".into());
    }
    let path = directory.join(file_name);
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| "The selected clip is unavailable")?;
    if !metadata.file_type().is_file() {
        return Err("The selected clip is not a regular video file".into());
    }
    let parent = directory
        .canonicalize()
        .map_err(|_| "Clip folder is unavailable")?;
    let canonical = path.canonicalize().map_err(|_| "Clip is unavailable")?;
    if canonical.parent() != Some(parent.as_path()) {
        return Err("Clip must remain in its replay folder".into());
    }
    Ok(canonical)
}

pub(crate) struct OpenedClip {
    pub file: File,
    pub bytes: u64,
    /// Game attribution resolved from the inventory for the source clip, so
    /// an export can carry it without a second inventory scan.
    pub game_name: Option<String>,
}

pub(crate) fn open_inventory_clip(file_name: &str) -> Result<OpenedClip, String> {
    let service = crate::backend::service();
    let inventory = service.replay_clip_inventory().map_err(|e| e.to_string())?;
    let clip = inventory
        .iter()
        .find(|clip| clip.file_name == file_name)
        .ok_or("The selected file is not in Redunar's replay inventory")?;
    let game_name = clip.game_name.clone();
    let path = regular_clip_path(
        clip.path.parent().ok_or("Clip folder is unavailable")?,
        file_name,
    )?;
    let before = std::fs::symlink_metadata(&path).map_err(|_| "The clip is unavailable")?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| "The selected clip could not be opened")?;
    let opened = file
        .metadata()
        .map_err(|_| "Clip metadata is unavailable")?;
    if !before.file_type().is_file()
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || before.len() != opened.len()
    {
        return Err("The clip changed while opening. Reload your clips.".into());
    }
    Ok(OpenedClip {
        file,
        bytes: opened.len(),
        game_name,
    })
}

#[tauri::command]
pub async fn clip_playback_path(
    file_name: String,
    playback: tauri::State<'_, crate::playback::Playback>,
) -> Result<String, String> {
    let playback = playback.inner().clone();
    // Inventory I/O and FFmpeg preparation must not block the webview thread.
    tauri::async_runtime::spawn_blocking(move || {
        let opened = open_inventory_clip(&file_name)?;
        playback.select(opened.file, "video/mp4")
    })
    .await
    .map_err(|error| format!("Clip playback worker failed: {error}"))?
}

#[tauri::command]
pub fn cancel_clip_playback(playback: tauri::State<'_, crate::playback::Playback>) {
    playback.cancel_preparation();
}

/// Generate one bounded JPEG frame for a validated local clip. The bytes are
/// returned directly to the local UI so no filesystem path or extra HTTP
/// endpoint is exposed for thumbnails.
#[tauri::command]
pub async fn clip_thumbnail(file_name: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || generate_clip_thumbnail(&file_name))
        .await
        .map_err(|error| format!("Clip thumbnail worker failed: {error}"))?
}

fn generate_clip_thumbnail(file_name: &str) -> Result<Vec<u8>, String> {
    let opened = open_inventory_clip(file_name)?;
    let mut child = crate::media_tools::ffmpeg(Some("mjpeg"))?
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-vf",
            "scale=320:320:force_original_aspect_ratio=decrease",
            "-an",
            "-q:v",
            "5",
            "-fs",
            &(THUMBNAIL_MAX_BYTES + 1).to_string(),
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ])
        .stdin(Stdio::from(opened.file))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "FFmpeg is required to create clip thumbnails".to_owned()
            } else {
                format!("Could not create clip thumbnail: {error}")
            }
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or("FFmpeg thumbnail output is unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((THUMBNAIL_MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < THUMBNAIL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("Creating the clip thumbnail timed out".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(format!("Could not inspect thumbnail process: {error}"));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "Clip thumbnail reader stopped unexpectedly")?
        .map_err(|error| format!("Could not read clip thumbnail: {error}"))?;
    if !status.success()
        || bytes.len() < 4
        || bytes.len() > THUMBNAIL_MAX_BYTES
        || bytes.get(0..2) != Some(&[0xff, 0xd8])
        || bytes.get(bytes.len().saturating_sub(2)..) != Some(&[0xff, 0xd9])
    {
        return Err("FFmpeg could not produce a valid clip thumbnail".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub fn open_replay_folder() -> Result<(), String> {
    let path = crate::backend::service()
        .replay_save_directory()
        .map_err(|e| e.to_string())?;
    if !path.is_dir() {
        return Err("No replay folder exists yet. It is created when a clip is saved.".into());
    }
    let mut child = std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(|_| "The desktop could not open the replay folder")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
pub fn open_clip_external(file_name: String) -> Result<(), String> {
    let clip = crate::backend::service()
        .replay_clip_inventory()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|clip| clip.file_name == file_name)
        .ok_or("The selected clip is not in Redunar's replay inventory")?;
    let parent = clip.path.parent().ok_or("Clip folder is unavailable")?;
    let path = regular_clip_path(parent, &file_name)?;
    let mut child = std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(|_| "The desktop could not open this clip")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_traversal_symlinks_and_directories() {
        let dir =
            std::env::temp_dir().join(format!("redunar-clip-boundary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.mkv");
        std::fs::write(&target, b"fixture").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("linked.mkv")).unwrap();
        assert!(regular_clip_path(&dir, "test.mkv").is_ok());
        for name in ["../test.mkv", "/tmp/test.mkv", "linked.mkv", "."] {
            assert!(regular_clip_path(&dir, name).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
