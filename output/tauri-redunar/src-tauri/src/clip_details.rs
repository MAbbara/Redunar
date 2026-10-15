//! Native mutations of exact owned recording identifiers.
use serde::Serialize;

#[derive(Serialize)]
pub struct ClipMutation {
    file_name: String,
    favorite: bool,
}
#[tauri::command]
pub async fn rename_replay_clip(
    file_name: String,
    new_file_name: String,
    exports: tauri::State<'_, crate::clip_export::ClipExports>,
) -> Result<ClipMutation, String> {
    crate::backend::ensure_write_access()?;
    let exports = exports.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        exports.with_idle(|| {
            let service = crate::backend::service();
            service
                .rename_replay_clip(&file_name, &new_file_name)
                .map_err(|e| e.to_string())?;
            let favorite = service
                .replay_clip_inventory()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|c| c.file_name == new_file_name)
                .is_some_and(|c| c.favorite);
            Ok(ClipMutation {
                file_name: new_file_name,
                favorite,
            })
        })
    })
    .await
    .map_err(|_| "Filename editing stopped unexpectedly".to_owned())?
}
#[tauri::command]
pub async fn set_clip_details(file_name: String, favorite: bool) -> Result<ClipMutation, String> {
    crate::backend::ensure_write_access()?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::backend::service()
            .set_replay_clip_favorite(&file_name, favorite)
            .map_err(|e| e.to_string())?;
        Ok(ClipMutation {
            file_name,
            favorite,
        })
    })
    .await
    .map_err(|_| "Saving the favorite stopped unexpectedly".to_owned())?
}

#[tauri::command]
pub async fn trashed_replay_clips() -> Result<Vec<crate::media::Clip>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::backend::service()
            .replay_trash_inventory()
            .map(|clips| clips.into_iter().map(Into::into).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Reading the trash bin stopped unexpectedly".to_owned())?
}

// No batch selection exists. Enforce one exact identifier at the native boundary
// so malformed requests cannot turn a single-clip action into an unbounded write.
fn single_clip(file_names: Vec<String>) -> Result<String, String> {
    if file_names.len() != 1 {
        return Err("Choose one clip".into());
    }
    Ok(file_names.into_iter().next().expect("one clip"))
}

async fn change_trash(
    file_names: Vec<String>,
    trashed: bool,
    exports: crate::clip_export::ClipExports,
) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    let name = single_clip(file_names)?;
    tauri::async_runtime::spawn_blocking(move || {
        exports.with_idle(|| {
            crate::backend::service()
                .set_replay_clip_trashed(&name, trashed)
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|_| "Changing the trash bin stopped unexpectedly".to_owned())?
}

#[tauri::command]
pub async fn move_replay_clips_to_trash(
    file_names: Vec<String>,
    exports: tauri::State<'_, crate::clip_export::ClipExports>,
) -> Result<(), String> {
    change_trash(file_names, true, exports.inner().clone()).await
}

#[tauri::command]
pub async fn restore_replay_clips(
    file_names: Vec<String>,
    exports: tauri::State<'_, crate::clip_export::ClipExports>,
) -> Result<(), String> {
    change_trash(file_names, false, exports.inner().clone()).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn trash_commands_bound_the_number_of_targets() {
        assert!(super::single_clip(vec![]).is_err());
        assert!(super::single_clip(vec!["one.mp4".into(), "two.mp4".into()]).is_err());
        assert_eq!(
            super::single_clip(vec!["one.mp4".into()]).unwrap(),
            "one.mp4"
        );
    }
}
