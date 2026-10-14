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
