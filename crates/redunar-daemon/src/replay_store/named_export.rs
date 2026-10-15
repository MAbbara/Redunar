//! Exclusive named exports. Authorize the temporary inode before publishing its
//! user-facing name, preserving old readers and crash-safe ownership recovery.
use super::{
    CLIP_PREFIX, Label, NEXT_FILE, Ordering, OsStr, Read, ReplayClipStore, ReplayStoreError,
    StoredReplayClip, SystemTime, UNIX_EPOCH, create_temporary, fs, io, io_error,
    is_owned_clip_name, labels, scan_owned_clips, store_operation, sync_directory, validate_store,
    write_bounded_mp4,
};

impl ReplayClipStore {
    /// Save a separate named MP4 without overwriting or deleting any recording.
    /// The caller supplies a stem only; the native store always appends `.mp4`.
    ///
    /// # Errors
    /// Invalid stems, collisions, unsafe state, capacity or persistence failures.
    pub fn save_named_mp4_preserving_existing(
        &self,
        source: &mut impl Read,
        stem: &str,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        let name = format!("{stem}.mp4");
        if stem.is_empty()
            || stem.trim() != stem
            || stem.len() > 200
            || !labels::valid_name(&name)
            || is_owned_clip_name(OsStr::new(&name))
        {
            return Err(ReplayStoreError::new("Use a valid export filename"));
        }
        let (temporary_path, mut temporary) = {
            let _operation = store_operation();
            validate_store(&self.directory)?;
            if fs::symlink_metadata(self.directory.join(&name)).is_ok() {
                return Err(ReplayStoreError::new(
                    "A clip with that filename already exists",
                ));
            }
            create_temporary(&self.directory)?
        };
        let result = (|| {
            let bytes = write_bounded_mp4(
                &mut temporary,
                source,
                self.maximum_clip_bytes,
                self.storage_limit_bytes,
            )?;
            temporary
                .sync_all()
                .map_err(|e| io_error("could not flush export", &e))?;
            let _operation = store_operation();
            validate_store(&self.directory)?;
            let used =
                scan_owned_clips(&self.directory)?
                    .iter()
                    .try_fold(0_u64, |total, clip| {
                        total
                            .checked_add(clip.bytes)
                            .ok_or_else(|| ReplayStoreError::new("replay store size overflows"))
                    })?;
            if used
                .checked_add(bytes)
                .is_none_or(|total| total > self.storage_limit_bytes)
            {
                return Err(ReplayStoreError::new(
                    "trimmed clip does not fit without removing an existing replay",
                ));
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ReplayStoreError::new("system clock is before the Unix epoch"))?;
            let original = format!(
                "{CLIP_PREFIX}{}-{:09}-{}.mp4",
                now.as_secs(),
                now.subsec_nanos(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            );
            let metadata = temporary
                .metadata()
                .map_err(|e| io_error("could not inspect export", &e))?;
            let mut registry = labels::load(&self.directory)
                .map_err(|e| io_error("could not read export metadata", &e))?;
            labels::prune(&self.directory, &mut registry);
            // A collision must never replace existing label authorization either.
            if fs::symlink_metadata(self.directory.join(&name)).is_ok() {
                return Err(ReplayStoreError::new(
                    "A clip with that filename already exists",
                ));
            }
            registry.insert(name.clone(), Label::new(original, &metadata));
            labels::save(&self.directory, &mut registry)
                .map_err(|e| io_error("could not save export metadata", &e))?;
            let destination = self.directory.join(&name);
            nix::fcntl::renameat2(
                None,
                &temporary_path,
                None,
                &destination,
                nix::fcntl::RenameFlags::RENAME_NOREPLACE,
            )
            .map_err(|e| {
                io_error(
                    "could not commit named export",
                    &io::Error::from_raw_os_error(e as i32),
                )
            })?;
            sync_directory(&self.directory)
                .map_err(|e| io_error("export committed but directory flush failed", &e))?;
            Ok(StoredReplayClip {
                path: destination,
                bytes,
                clips_removed_for_quota: 0,
                bytes_removed_for_quota: 0,
            })
        })();
        drop(temporary);
        // This is only the unpublished temporary path; never remove a committed clip.
        let _ = fs::remove_file(temporary_path);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_export_budget_includes_trash_and_never_evicts_recordings() {
        let root = std::env::temp_dir().join(format!(
            "redunar-named-budget-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let media = b"\0\0\0\x18ftypisom";
        let store = ReplayClipStore::open_with_limits(
            root.clone(),
            media.len() as u64,
            (media.len() * 2) as u64,
        )
        .unwrap();
        store
            .save_named_mp4_preserving_existing(&mut media.as_slice(), "First")
            .unwrap();
        store.set_trashed("First.mp4", true).unwrap();
        store
            .save_named_mp4_preserving_existing(&mut media.as_slice(), "Second")
            .unwrap();
        assert!(
            store
                .save_named_mp4_preserving_existing(&mut media.as_slice(), "Third")
                .is_err()
        );
        assert!(!root.join("Third.mp4").exists());
        assert_eq!(fs::read(root.join("First.mp4")).unwrap(), media);
        assert_eq!(fs::read(root.join("Second.mp4")).unwrap(), media);
        assert_eq!(store.trash_inventory().unwrap().len(), 1);
        assert_eq!(store.inventory().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn named_exports_preserve_ownership_collisions_source_and_quota() {
        let root = std::env::temp_dir().join(format!(
            "redunar-named-export-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let store = ReplayClipStore::open_with_limits(root.clone(), 1024, 2048).unwrap();
        let media = b"\0\0\0\x18ftypisom";
        let saved = store
            .save_named_mp4_preserving_existing(&mut media.as_slice(), "My trim")
            .unwrap();
        assert_eq!(saved.path.file_name().unwrap(), "My trim.mp4");
        let clips = store.inventory().unwrap();
        assert_eq!(clips.len(), 1);
        assert!(is_owned_clip_name(OsStr::new(&clips[0].original_file_name)));
        assert!(
            store
                .save_named_mp4_preserving_existing(&mut media.as_slice(), "My trim")
                .is_err()
        );
        assert_eq!(fs::read(&saved.path).unwrap(), media);
        for invalid in [
            "../bad",
            "bad/name",
            "bad\\name",
            ".hidden",
            " trailing ",
            "redunar-replay-1-2-3",
        ] {
            assert!(
                store
                    .save_named_mp4_preserving_existing(&mut media.as_slice(), invalid)
                    .is_err()
            );
        }
        assert!(
            store
                .save_named_mp4_preserving_existing(&mut b"invalid".as_slice(), "Bad")
                .is_err()
        );
        fs::rename(root.join(labels::FILE), root.join("old-labels")).unwrap();
        fs::create_dir(root.join(labels::FILE)).unwrap();
        assert!(
            store
                .save_named_mp4_preserving_existing(&mut media.as_slice(), "Failed")
                .is_err()
        );
        assert!(!root.join("Failed.mp4").exists());
        assert_eq!(fs::read(&saved.path).unwrap(), media);
        fs::remove_dir_all(root).unwrap();
    }
}
