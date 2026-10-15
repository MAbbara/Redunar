//! Owner-controlled restart after confirmed package installation. The fixed
//! installed executable is launched only after all native Exit cleanup runs.
use std::os::unix::process::CommandExt;
use std::{
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::Manager;
#[derive(Default)]
pub struct UpdateRestart(AtomicBool);

#[tauri::command]
pub async fn restart_redunar(app: tauri::AppHandle) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    let owner = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::updates::ensure_restart_ready()?;
        validate_executable(Path::new("/usr/bin/redunar-tauri"))?;
        owner
            .state::<crate::clip_export::ClipExports>()
            .claim_restart(|| {
                owner
                    .state::<crate::sessions::Sessions>()
                    .claim_update_restart()
            })?;
        owner
            .state::<UpdateRestart>()
            .0
            .store(true, Ordering::Release);
        // Use an ordinary exit, so cleanup can veto it. Tauri's special restart
        // exit code cannot be prevented and may reuse background startup args.
        owner.exit(0);
        Ok(())
    })
    .await
    .map_err(|_| "Restart preparation stopped unexpectedly".to_owned())?
}

fn validate_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "The installed Redunar executable is unavailable")?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err("The installed Redunar executable has unsafe permissions".into());
    }
    Ok(())
}

impl UpdateRestart {
    pub fn cancel(&self) {
        self.0.store(false, Ordering::Release);
    }
    pub fn finish(&self) {
        if self.0.swap(false, Ordering::AcqRel) {
            // Replace this process after cleanup: spawning a child here races the
            // old service clones' kernel lease. Their close-on-exec descriptors
            // are released atomically by exec, before the new backend starts.
            // Drop --steam-background so the replacement opens a visible window.
            let error = Command::new("/usr/bin/redunar-tauri").exec();
            eprintln!("Redunar could not reopen after updating: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_unowned_restart_executable_is_rejected() {
        assert!(validate_executable(Path::new("/nonexistent/redunar-tauri")).is_err());
        let file =
            std::env::temp_dir().join(format!("redunar-restart-test-{}", std::process::id()));
        std::fs::write(&file, b"fixture").unwrap();
        assert!(validate_executable(&file).is_err());
        std::fs::remove_file(file).unwrap();
        let state = UpdateRestart::default();
        state.0.store(true, Ordering::Release);
        state.cancel();
        assert!(!state.0.load(Ordering::Acquire));
    }
}
