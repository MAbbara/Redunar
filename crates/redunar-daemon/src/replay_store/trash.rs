//! Recoverable logical bin. Media stays in its owned store, never OS Trash.
//! A separate inode-bound ledger preserves the existing filename-label format.
use super::{
    Path, ReplayClipEntry, ReplayClipStore, ReplayStoreError, fs, io_error, labels, owned_metadata,
    store_operation, validate_store,
};
pub(super) const FILE: &str = ".redunar-clip-trash-v1";

impl ReplayClipStore {
    /// List recoverable recordings, with the same bounded ownership checks.
    ///
    /// # Errors
    /// Invalid storage, metadata or directory scans fail closed.
    pub fn trash_inventory(&self) -> Result<Vec<ReplayClipEntry>, ReplayStoreError> {
        self.inventory_state(true)
    }

    /// Persist one clip's bin state atomically without deleting or moving media.
    /// Repeated requests are idempotent. Only matching file inodes can be restored.
    ///
    /// # Errors
    /// Unsafe/replaced files and failed durable metadata writes are rejected.
    pub fn set_trashed(&self, file_name: &str, trashed: bool) -> Result<(), ReplayStoreError> {
        let _operation = store_operation();
        validate_store(&self.directory)?;
        let (metadata, label) = owned_metadata(&self.directory, file_name)?;
        let mut bin = labels::load_from(&self.directory, FILE)
            .map_err(|e| io_error("could not read Replay trash bin", &e))?;
        if bin.get(file_name).is_some_and(|l| l.matches(&metadata)) == trashed {
            return Ok(());
        }
        labels::prune(&self.directory, &mut bin);
        if trashed {
            bin.insert(file_name.to_owned(), label);
        } else {
            bin.remove(file_name);
        }
        labels::save_to(&self.directory, FILE, &bin)
            .map_err(|e| io_error("could not save Replay trash bin", &e))
    }
}

pub(super) fn require_live(
    directory: &Path,
    name: &str,
    metadata: &fs::Metadata,
) -> Result<(), ReplayStoreError> {
    let bin = labels::load_from(directory, FILE)
        .map_err(|e| io_error("could not read Replay trash bin", &e))?;
    if bin.get(name).is_some_and(|l| l.matches(metadata)) {
        return Err(ReplayStoreError::new(
            "Restore the clip from the trash bin first",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{MATROSKA_MAGIC, NEXT_FILE, Ordering};
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    fn store() -> (PathBuf, ReplayClipStore, String) {
        let root = std::env::temp_dir().join(format!(
            "redunar-trash-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let store = ReplayClipStore::open_with_limits(root.clone(), 1024, 8192).unwrap();
        let clip = store.save_matroska(&mut MATROSKA_MAGIC.as_slice()).unwrap();
        let name = clip.path.file_name().unwrap().to_str().unwrap().to_owned();
        (root, store, name)
    }
    #[test]
    fn trash_restore_survives_reopen_with_filename_favorite_and_bytes() {
        let (root, store, name) = store();
        store.rename_clip(&name, "My clip.mkv").unwrap();
        store.set_favorite("My clip.mkv", true).unwrap();
        store.set_trashed("My clip.mkv", true).unwrap();
        store.set_trashed("My clip.mkv", true).unwrap();
        assert!(store.inventory().unwrap().is_empty());
        assert!(store.rename_clip("My clip.mkv", "Other.mkv").is_err());
        let reopened = ReplayClipStore::open_with_limits(root.clone(), 1024, 8192).unwrap();
        let bin = reopened.trash_inventory().unwrap();
        assert_eq!(bin.len(), 1);
        assert!(bin[0].favorite);
        assert_eq!(bin[0].original_file_name, name);
        assert_eq!(fs::read(&bin[0].path).unwrap(), MATROSKA_MAGIC);
        assert_eq!(
            ReplayClipStore::inspect_with_limits(root.clone(), 1024, 8192)
                .unwrap()
                .used_bytes,
            4
        );
        reopened.set_trashed("My clip.mkv", false).unwrap();
        reopened.set_trashed("My clip.mkv", false).unwrap();
        assert!(reopened.trash_inventory().unwrap().is_empty());
        assert!(reopened.inventory().unwrap()[0].favorite);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unsafe_targets_failed_writes_and_replaced_inodes_never_restore_foreign_media() {
        let (root, store, name) = store();
        for invalid in ["../foreign.mkv", "arbitrary.mkv", ""] {
            assert!(store.set_trashed(invalid, true).is_err());
        }
        fs::create_dir(root.join(FILE)).unwrap();
        assert!(store.set_trashed(&name, true).is_err());
        assert_eq!(fs::read(root.join(&name)).unwrap(), MATROSKA_MAGIC);
        fs::remove_dir(root.join(FILE)).unwrap();
        store.set_trashed(&name, true).unwrap();
        // Keep the old inode alive: replacing a name must not inherit its bin identity.
        fs::rename(root.join(&name), root.join("previous")).unwrap();
        symlink("previous", root.join(&name)).unwrap();
        assert!(store.set_trashed(&name, false).is_err());
        assert!(store.trash_inventory().unwrap().is_empty());
        fs::remove_file(root.join(&name)).unwrap();
        fs::write(root.join(&name), MATROSKA_MAGIC).unwrap();
        assert!(store.trash_inventory().unwrap().is_empty());
        assert_eq!(store.inventory().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
