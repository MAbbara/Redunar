use crate::{
    ReplayAudioSnapshot, ReplayAudioTracks, ReplayBudget, ReplayOutputFormat, ReplayRing,
    ReplayVideoStream, write_audio_video_matroska_tracks, write_audio_video_mp4_tracks,
    write_video_only_matroska, write_video_only_mp4,
};
use nix::sys::statvfs::statvfs;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const STORE_MARKER: &str = ".redunar-replay-store-v1";
const STORE_MARKER_CONTENTS: &[u8] = b"redunar-replay-store-v1\n";
const CLIP_PREFIX: &str = "redunar-replay-";
const MATROSKA_SUFFIX: &str = ".mkv";
const MP4_SUFFIX: &str = ".mp4";
const MATROSKA_MAGIC: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];
const MP4_FILE_TYPE_BOX: [u8; 4] = *b"ftyp";
const MAX_DIRECTORY_ENTRIES: usize = 4096;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MINIMUM_FILESYSTEM_RESERVE_BYTES: u64 = 512 * 1024 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
static STORE_OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
pub struct ReplayClipStore {
    directory: PathBuf,
    maximum_clip_bytes: u64,
    storage_limit_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredReplayClip {
    pub path: PathBuf,
    pub bytes: u64,
    pub clips_removed_for_quota: u32,
    pub bytes_removed_for_quota: u64,
}

/// One regular clip owned by the dedicated Redunar replay store.
///
/// `file_name` is the stable, directory-local identifier accepted by
/// [`ReplayClipStore::delete_clip`]. It can never contain a path separator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayClipEntry {
    pub file_name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub modified_unix_ns: u128,
    /// Game that recorded this clip, when local attribution can resolve it.
    /// This is labeling data for the Recent captures view; a missing value
    /// never affects playback, export, or deletion.
    pub game_name: Option<String>,
}

/// Read-only inventory of the dedicated replay directory.
///
/// An uninitialized store is a normal state while production Replay is
/// unavailable. Inspecting it never creates a directory, ownership marker, or
/// clip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayStorageStatus {
    pub directory: PathBuf,
    pub initialized: bool,
    pub owned_clips: u32,
    pub used_bytes: u64,
    pub storage_limit_bytes: u64,
    pub bytes_over_limit: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayStoreError {
    message: String,
}

impl ReplayStoreError {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }

    fn owned(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for ReplayStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ReplayStoreError {}

#[derive(Debug)]
struct OwnedClip {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

impl ReplayClipStore {
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Ensure one clip can be assembled without consuming the filesystem's
    /// fixed safety reserve. This check happens immediately before writing;
    /// quota cleanup is not allowed to assume unrelated free space will
    /// appear later.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for an invalid size, replaced store,
    /// unavailable filesystem statistics, or insufficient free space.
    pub fn ensure_free_space(&self, expected_clip_bytes: u64) -> Result<(), ReplayStoreError> {
        if expected_clip_bytes == 0 || expected_clip_bytes > self.maximum_clip_bytes {
            return Err(ReplayStoreError::new(
                "replay free-space request exceeds the configured clip budget",
            ));
        }
        validate_store(&self.directory)?;
        let status = statvfs(&self.directory).map_err(|error| {
            ReplayStoreError::owned(format!(
                "could not inspect free space for Replay clips: {error}"
            ))
        })?;
        let available = status
            .blocks_available()
            .saturating_mul(status.fragment_size());
        if !has_required_free_space(available, expected_clip_bytes) {
            return Err(ReplayStoreError::new(
                "not enough free space to save Replay while preserving the 512 MiB safety reserve",
            ));
        }
        Ok(())
    }

    /// Reserve enough filesystem space for the largest clip this store can
    /// commit. The clip budget already includes container headroom, so callers
    /// must not add a second allowance and accidentally reject a valid ring.
    pub(crate) fn ensure_free_space_for_clip(&self) -> Result<(), ReplayStoreError> {
        self.ensure_free_space(self.maximum_clip_bytes)
    }

    /// Inspect one replay directory without initializing or modifying it.
    ///
    /// Existing state must have Redunar's exact ownership marker before any
    /// generated clip names are counted. The scan uses the same fixed entry
    /// bound as quota enforcement.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for relative paths, invalid budgets,
    /// unsafe existing state, an excessive entry count, or filesystem errors.
    pub fn inspect(
        directory: impl Into<PathBuf>,
        budget: ReplayBudget,
    ) -> Result<ReplayStorageStatus, ReplayStoreError> {
        Self::inspect_with_limits(
            directory.into(),
            budget.maximum_ring_bytes,
            budget.storage_limit_bytes,
        )
    }

    /// Open or initialize one daemon-owned replay directory.
    ///
    /// A pre-existing non-empty directory without Redunar's exact ownership
    /// marker is rejected. This prevents quota cleanup from ever claiming an
    /// arbitrary user directory.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for relative paths, unsafe directory or
    /// marker state, invalid budgets, and filesystem failures.
    pub fn open(
        directory: impl Into<PathBuf>,
        budget: ReplayBudget,
    ) -> Result<Self, ReplayStoreError> {
        Self::open_with_limits(
            directory.into(),
            budget.maximum_ring_bytes,
            budget.storage_limit_bytes,
        )
    }

    /// Open an already-initialized Redunar replay directory without creating
    /// or changing it.
    ///
    /// This is the appropriate entry point for clip inventory and deletion
    /// controls: viewing an empty Replay page must not initialize storage.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] if the path or budget is unsafe, the
    /// directory is missing, or its ownership marker is not exact.
    pub fn open_existing(
        directory: impl Into<PathBuf>,
        budget: ReplayBudget,
    ) -> Result<Self, ReplayStoreError> {
        let directory = directory.into();
        validate_store_configuration(
            &directory,
            budget.maximum_ring_bytes,
            budget.storage_limit_bytes,
        )?;
        let _operation = store_operation();
        validate_store(&directory)?;
        Ok(Self {
            directory,
            maximum_clip_bytes: budget.maximum_ring_bytes,
            storage_limit_bytes: budget.storage_limit_bytes,
        })
    }

    /// Save one finalized Matroska container and enforce the configured
    /// Redunar-owned clip quota after its durable commit.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for malformed or oversized input, unsafe
    /// store replacement, I/O failures, or quota cleanup failures. Recording
    /// callers must surface the error and must not retry in a busy loop.
    pub fn save_matroska(
        &self,
        source: &mut impl Read,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_generated(ReplayOutputFormat::Matroska, false, |temporary| {
            write_bounded_matroska(
                temporary,
                source,
                self.maximum_clip_bytes,
                self.storage_limit_bytes,
            )
        })
    }

    /// Save one finalized MP4 while retaining every existing replay clip.
    ///
    /// This is used for edits derived from a saved clip. If the new file does
    /// not fit inside the configured store limit, the operation is rejected
    /// before commit rather than deleting the source or another existing clip.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for an invalid or oversized MP4, unsafe
    /// store ownership, insufficient configured capacity, or an I/O failure.
    pub fn save_mp4_preserving_existing(
        &self,
        source: &mut impl Read,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_generated(ReplayOutputFormat::Mp4, true, |temporary| {
            write_bounded_mp4(
                temporary,
                source,
                self.maximum_clip_bytes,
                self.storage_limit_bytes,
            )
        })
    }

    /// Mux one codec-consistent encoded ring directly into the private atomic
    /// clip temporary file. Encoded payloads are not duplicated in memory.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] when the ring cannot form a playable
    /// video-only Matroska stream, exceeds its byte budget, or durable storage
    /// and quota handling fail.
    pub fn save_video_only(
        &self,
        stream: &ReplayVideoStream,
        ring: &ReplayRing,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_video_only_as(stream, ring, ReplayOutputFormat::Matroska)
    }

    /// Mux and atomically commit the selected supported video container.
    ///
    /// # Errors
    ///
    /// Returns an error for incompatible encoded packets, container overflow,
    /// unsafe storage ownership, I/O failure, or quota failure.
    pub fn save_video_only_as(
        &self,
        stream: &ReplayVideoStream,
        ring: &ReplayRing,
        output_format: ReplayOutputFormat,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_generated(output_format, false, |temporary| {
            let hard_limit = self.maximum_clip_bytes.min(self.storage_limit_bytes);
            let mut bounded = BoundedClipWriter::new(temporary, hard_limit);
            match output_format {
                ReplayOutputFormat::Matroska => {
                    write_video_only_matroska(&mut bounded, stream, ring)
                        .map(|summary| summary.bytes_written)
                        .map_err(|error| {
                            ReplayStoreError::owned(format!(
                                "could not finalize video-only Matroska replay: {error}"
                            ))
                        })
                }
                ReplayOutputFormat::Mp4 => write_video_only_mp4(&mut bounded, stream, ring)
                    .map(|summary| summary.bytes_written)
                    .map_err(|error| {
                        ReplayStoreError::owned(format!(
                            "could not finalize video-only MP4 replay: {error}"
                        ))
                    }),
            }
        })
    }

    /// Mux synchronized game audio when available, while preserving the
    /// existing video-only fallback for games without an owned audio stream.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for invalid packet timing, container
    /// overflow, unsafe storage ownership, or durable commit failure.
    pub fn save_with_audio_as(
        &self,
        stream: &ReplayVideoStream,
        ring: &ReplayRing,
        audio: &ReplayAudioSnapshot,
        output_format: ReplayOutputFormat,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        let tracks = ReplayAudioTracks {
            system: audio.clone(),
            microphone: None,
        };
        self.save_with_audio_tracks_as(stream, ring, &tracks, output_format)
    }

    /// Save video with bounded independent audio tracks.
    ///
    /// # Errors
    /// Returns an error on invalid media, unsafe storage or insufficient space.
    pub fn save_with_audio_tracks_as(
        &self,
        stream: &ReplayVideoStream,
        ring: &ReplayRing,
        audio: &ReplayAudioTracks,
        output_format: ReplayOutputFormat,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        let Some(first_video) = ring.packets().next() else {
            return self.save_video_only_as(stream, ring, output_format);
        };
        let last_video = ring.packets().last().unwrap_or(first_video);
        let video_start = first_video.timestamp_ns();
        let video_end = last_video
            .timestamp_ns()
            .saturating_add(last_video.duration_ns());
        let has_audio =
            audio.system.packets.iter().any(|packet| {
                packet.timestamp_ns >= video_start && packet.timestamp_ns < video_end
            }) || audio.microphone.as_ref().is_some_and(|track| {
                track.packets.iter().any(|packet| {
                    packet.timestamp_ns >= video_start && packet.timestamp_ns < video_end
                })
            });
        if !has_audio {
            // Out-of-window audio must not create an empty track.
            return self.save_video_only_as(stream, ring, output_format);
        }
        self.save_generated(output_format, false, |temporary| {
            let hard_limit = self.maximum_clip_bytes.min(self.storage_limit_bytes);
            let mut bounded = BoundedClipWriter::new(temporary, hard_limit);
            match output_format {
                ReplayOutputFormat::Matroska => {
                    write_audio_video_matroska_tracks(&mut bounded, stream, ring, audio)
                        .map(|summary| summary.bytes_written)
                        .map_err(|error| {
                            ReplayStoreError::owned(format!(
                                "could not finalize audio/video Matroska replay: {error}"
                            ))
                        })
                }
                ReplayOutputFormat::Mp4 => {
                    write_audio_video_mp4_tracks(&mut bounded, stream, ring, audio)
                        .map(|summary| summary.bytes_written)
                        .map_err(|error| {
                            ReplayStoreError::owned(format!(
                                "could not finalize audio/video MP4 replay: {error}"
                            ))
                        })
                }
            }
        })
    }

    pub(crate) fn save_streaming_tracks(
        &self,
        stream: &ReplayVideoStream,
        source: &mut impl crate::replay_packet_source::ReplayPacketSource,
        audio: &ReplayAudioTracks,
        output_format: ReplayOutputFormat,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_generated(output_format, false, |temporary| {
            let mut bounded = BoundedClipWriter::new(
                temporary,
                self.maximum_clip_bytes.min(self.storage_limit_bytes),
            );
            match output_format {
                ReplayOutputFormat::Matroska => {
                    crate::replay_matroska::write_streaming_matroska_tracks(
                        &mut bounded,
                        stream,
                        source,
                        audio,
                    )
                    .map_err(|error| {
                        ReplayStoreError::owned(format!(
                            "could not finalize streamed Matroska replay: {error}"
                        ))
                    })
                }
                ReplayOutputFormat::Mp4 => crate::replay_mp4::write_streaming_mp4_tracks(
                    &mut bounded,
                    stream,
                    source,
                    audio,
                )
                .map_err(|error| {
                    ReplayStoreError::owned(format!(
                        "could not finalize streamed MP4 replay: {error}"
                    ))
                }),
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn save_streaming(
        &self,
        stream: &ReplayVideoStream,
        source: &mut impl crate::replay_packet_source::ReplayPacketSource,
        audio: &ReplayAudioSnapshot,
        output_format: ReplayOutputFormat,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        self.save_generated(output_format, false, |temporary| {
            let mut bounded = BoundedClipWriter::new(
                temporary,
                self.maximum_clip_bytes.min(self.storage_limit_bytes),
            );
            match output_format {
                ReplayOutputFormat::Matroska => crate::replay_matroska::write_streaming_matroska(
                    &mut bounded,
                    stream,
                    source,
                    audio,
                )
                .map_err(|error| {
                    ReplayStoreError::owned(format!(
                        "could not finalize streamed Matroska replay: {error}"
                    ))
                }),
                ReplayOutputFormat::Mp4 => {
                    crate::replay_mp4::write_streaming_mp4(&mut bounded, stream, source, audio)
                        .map_err(|error| {
                            ReplayStoreError::owned(format!(
                                "could not finalize streamed MP4 replay: {error}"
                            ))
                        })
                }
            }
        })
    }

    /// List regular Redunar-owned clips, newest first.
    ///
    /// The ownership marker and fixed directory-entry bound are revalidated
    /// on every call. Unknown files and symlinks are never returned.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] when the store was replaced, its marker is
    /// invalid, or the bounded directory scan fails.
    pub fn inventory(&self) -> Result<Vec<ReplayClipEntry>, ReplayStoreError> {
        let _operation = store_operation();
        validate_store(&self.directory)?;
        let mut clips = scan_owned_clips(&self.directory)?;
        clips.sort_by(|left, right| {
            right
                .modified
                .cmp(&left.modified)
                .then_with(|| right.path.file_name().cmp(&left.path.file_name()))
        });
        clips
            .into_iter()
            .map(|clip| {
                let file_name = clip
                    .path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| ReplayStoreError::new("replay clip name is invalid"))?
                    .to_owned();
                let modified_unix_ns = clip
                    .modified
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                Ok(ReplayClipEntry {
                    file_name,
                    path: clip.path,
                    bytes: clip.bytes,
                    modified_unix_ns,
                    // The store itself does not own attribution; the service
                    // joins persisted game evidence when listing clips.
                    game_name: None,
                })
            })
            .collect()
    }

    /// Delete exactly one regular clip returned by [`Self::inventory`].
    ///
    /// Arbitrary paths, unknown files, and symlinks are rejected. The parent
    /// directory is flushed after deletion so the removal is durable.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for an invalid clip identifier, replaced
    /// store ownership, a non-regular target, or an I/O failure.
    pub fn delete_clip(&self, file_name: &str) -> Result<ReplayClipEntry, ReplayStoreError> {
        let name = OsStr::new(file_name);
        if !is_owned_clip_name(name) {
            return Err(ReplayStoreError::new(
                "replay clip identifier is not a Redunar-generated name",
            ));
        }
        let _operation = store_operation();
        validate_store(&self.directory)?;
        let path = self.directory.join(name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error("could not inspect replay clip for deletion", &error))?;
        if !metadata.file_type().is_file() {
            return Err(ReplayStoreError::new(
                "replay clip deletion target is not a regular file",
            ));
        }
        let entry = ReplayClipEntry {
            file_name: file_name.to_owned(),
            path: path.clone(),
            bytes: metadata.len(),
            modified_unix_ns: metadata
                .modified()
                .unwrap_or(UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            game_name: None,
        };
        fs::remove_file(path)
            .map_err(|error| io_error("could not delete Redunar replay clip", &error))?;
        sync_directory(&self.directory)
            .map_err(|error| io_error("could not flush replay clip deletion", &error))?;
        Ok(entry)
    }

    /// Move one owned clip into the store's private `original` archive.
    ///
    /// The archive is deliberately below the inventory directory so archived
    /// sources are not treated as newly saved clips or quota candidates.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] for an invalid clip identifier, replaced
    /// store ownership, an unsafe archive directory, an existing destination,
    /// or a filesystem failure.
    pub fn archive_clip(&self, file_name: &str) -> Result<PathBuf, ReplayStoreError> {
        let name = OsStr::new(file_name);
        if !is_owned_clip_name(name) {
            return Err(ReplayStoreError::new(
                "replay clip identifier is not a Redunar-generated name",
            ));
        }
        let _operation = store_operation();
        validate_store(&self.directory)?;
        let source = self.directory.join(name);
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| io_error("could not inspect replay clip for archiving", &error))?;
        if !metadata.file_type().is_file() {
            return Err(ReplayStoreError::new(
                "replay archive source is not a regular file",
            ));
        }

        let archive = self.directory.join("original");
        match fs::symlink_metadata(&archive) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                if metadata.permissions().mode() & 0o777 != 0o700 {
                    fs::set_permissions(&archive, fs::Permissions::from_mode(0o700)).map_err(
                        |error| io_error("could not secure replay original archive", &error),
                    )?;
                }
            }
            Ok(_) => {
                return Err(ReplayStoreError::new(
                    "replay original archive is not a regular directory",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&archive).map_err(|error| {
                    io_error("could not create replay original archive", &error)
                })?;
                fs::set_permissions(&archive, fs::Permissions::from_mode(0o700)).map_err(
                    |error| io_error("could not secure replay original archive", &error),
                )?;
            }
            Err(error) => {
                return Err(io_error(
                    "could not inspect replay original archive",
                    &error,
                ));
            }
        }
        let destination = archive.join(name);
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(ReplayStoreError::new(
                    "an archived original with that name already exists",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error(
                    "could not inspect archived replay destination",
                    &error,
                ));
            }
        }
        fs::rename(&source, &destination)
            .map_err(|error| io_error("could not archive original replay clip", &error))?;
        sync_directory(&archive)
            .and_then(|()| sync_directory(&self.directory))
            .map_err(|error| io_error("could not flush replay original archive", &error))?;
        Ok(destination)
    }

    fn save_generated(
        &self,
        output_format: ReplayOutputFormat,
        preserve_existing: bool,
        write_clip: impl FnOnce(&mut File) -> Result<u64, ReplayStoreError>,
    ) -> Result<StoredReplayClip, ReplayStoreError> {
        let (temporary_path, mut temporary) = {
            let _operation = store_operation();
            validate_store(&self.directory)?;
            create_temporary(&self.directory)?
        };
        // Muxing can take seconds for long clips. Do not hold the inventory/
        // deletion lock while writing a private, unpublished temporary file.
        let bytes = match write_clip(&mut temporary) {
            Ok(bytes) => bytes,
            Err(error) => {
                drop(temporary);
                let _ = fs::remove_file(&temporary_path);
                return Err(error);
            }
        };
        let _operation = store_operation();
        let mut owned = match validate_store(&self.directory)
            .and_then(|()| scan_owned_clips(&self.directory))
        {
            Ok(clips) => clips,
            Err(error) => {
                drop(temporary);
                let _ = fs::remove_file(&temporary_path);
                return Err(error);
            }
        };
        if preserve_existing {
            let used_bytes = match owned.iter().try_fold(0_u64, |total, clip| {
                total
                    .checked_add(clip.bytes)
                    .ok_or_else(|| ReplayStoreError::new("replay store size overflows"))
            }) {
                Ok(bytes) => bytes,
                Err(error) => {
                    drop(temporary);
                    let _ = fs::remove_file(&temporary_path);
                    return Err(error);
                }
            };
            if match used_bytes.checked_add(bytes) {
                Some(total) => total > self.storage_limit_bytes,
                None => true,
            } {
                drop(temporary);
                let _ = fs::remove_file(&temporary_path);
                return Err(ReplayStoreError::new(
                    "trimmed clip does not fit without removing an existing replay",
                ));
            }
        }
        if let Err(error) = temporary.sync_all() {
            drop(temporary);
            let _ = fs::remove_file(&temporary_path);
            return Err(io_error("could not flush replay temporary file", &error));
        }
        drop(temporary);

        let destination =
            match commit_temporary(&self.directory, &temporary_path, output_format.extension()) {
                Ok(destination) => destination,
                Err(error) => {
                    let _ = fs::remove_file(&temporary_path);
                    return Err(error);
                }
            };
        sync_directory(&self.directory).map_err(|error| {
            ReplayStoreError::owned(format!(
                "replay clip committed but directory flush failed: kind={:?} errno={:?} detail={error}; clip={}",
                error.kind(), error.raw_os_error(),
                destination.display()
            ))
        })?;

        let metadata = fs::symlink_metadata(&destination)
            .map_err(|error| io_error("could not inspect committed replay clip", &error))?;
        owned.push(OwnedClip {
            path: destination.clone(),
            bytes,
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
        });
        let (clips_removed_for_quota, bytes_removed_for_quota) = if preserve_existing {
            (0, 0)
        } else {
            enforce_quota(
                &self.directory,
                &mut owned,
                self.storage_limit_bytes,
                Some(&destination),
            )
            .map_err(|error| {
                ReplayStoreError::owned(format!(
                    "replay clip saved but quota cleanup failed: {error}; clip={}",
                    destination.display()
                ))
            })?
        };

        Ok(StoredReplayClip {
            path: destination,
            bytes,
            clips_removed_for_quota,
            bytes_removed_for_quota,
        })
    }

    /// Re-apply the quota only to regular files with Redunar's generated clip
    /// names. Unknown files and symlinks are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStoreError`] when ownership validation, scanning, or a
    /// required deletion fails.
    pub fn enforce_quota(&self) -> Result<(u32, u64), ReplayStoreError> {
        let _operation = store_operation();
        validate_store(&self.directory)?;
        let mut owned = scan_owned_clips(&self.directory)?;
        enforce_quota(&self.directory, &mut owned, self.storage_limit_bytes, None)
    }

    fn open_with_limits(
        directory: PathBuf,
        maximum_clip_bytes: u64,
        storage_limit_bytes: u64,
    ) -> Result<Self, ReplayStoreError> {
        validate_store_configuration(&directory, maximum_clip_bytes, storage_limit_bytes)?;
        let _operation = store_operation();
        initialize_store(&directory)?;
        Ok(Self {
            directory,
            maximum_clip_bytes,
            storage_limit_bytes,
        })
    }

    fn inspect_with_limits(
        directory: PathBuf,
        maximum_clip_bytes: u64,
        storage_limit_bytes: u64,
    ) -> Result<ReplayStorageStatus, ReplayStoreError> {
        validate_store_configuration(&directory, maximum_clip_bytes, storage_limit_bytes)?;
        let _operation = store_operation();
        match fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ReplayStorageStatus {
                directory,
                initialized: false,
                owned_clips: 0,
                used_bytes: 0,
                storage_limit_bytes,
                bytes_over_limit: 0,
            }),
            Err(error) => Err(io_error("could not inspect replay store directory", &error)),
            Ok(_) => {
                validate_store(&directory)?;
                let clips = scan_owned_clips(&directory)?;
                let used_bytes = clips.iter().try_fold(0_u64, |total, clip| {
                    total
                        .checked_add(clip.bytes)
                        .ok_or_else(|| ReplayStoreError::new("replay store size overflows"))
                })?;
                Ok(ReplayStorageStatus {
                    directory,
                    initialized: true,
                    owned_clips: u32::try_from(clips.len()).unwrap_or(u32::MAX),
                    used_bytes,
                    storage_limit_bytes,
                    bytes_over_limit: used_bytes.saturating_sub(storage_limit_bytes),
                })
            }
        }
    }
}

fn has_required_free_space(available: u64, expected_clip_bytes: u64) -> bool {
    expected_clip_bytes
        .checked_add(MINIMUM_FILESYSTEM_RESERVE_BYTES)
        .is_some_and(|required| available >= required)
}

fn validate_store_configuration(
    directory: &Path,
    maximum_clip_bytes: u64,
    storage_limit_bytes: u64,
) -> Result<(), ReplayStoreError> {
    if !directory.is_absolute() {
        return Err(ReplayStoreError::new(
            "replay store directory must be absolute",
        ));
    }
    if maximum_clip_bytes == 0
        || storage_limit_bytes == 0
        || maximum_clip_bytes > storage_limit_bytes
    {
        return Err(ReplayStoreError::new("replay store budget is invalid"));
    }
    Ok(())
}

struct BoundedClipWriter<'a> {
    destination: &'a mut File,
    limit: u64,
    written: u64,
}

impl<'a> BoundedClipWriter<'a> {
    const fn new(destination: &'a mut File, limit: u64) -> Self {
        Self {
            destination,
            limit,
            written: 0,
        }
    }
}

impl Write for BoundedClipWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let requested = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if self.written.saturating_add(requested) > self.limit {
            return Err(io::Error::other(
                "replay container exceeds its configured byte budget",
            ));
        }
        let written = self.destination.write(buffer)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.destination.flush()
    }
}

fn store_operation() -> MutexGuard<'static, ()> {
    STORE_OPERATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn initialize_store(directory: &Path) -> Result<(), ReplayStoreError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(ReplayStoreError::new(
                "replay store path is not a regular directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(directory)
                .map_err(|error| io_error("could not create replay store directory", &error))?;
        }
        Err(error) => {
            return Err(io_error("could not inspect replay store directory", &error));
        }
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error("could not secure replay store directory", &error))?;

    let marker = directory.join(STORE_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(_) => validate_store(directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if directory_has_entries(directory)? {
                return Err(ReplayStoreError::new(
                    "refusing to claim a non-empty replay directory without Redunar's ownership marker",
                ));
            }
            write_marker(directory, &marker)?;
            validate_store(directory)
        }
        Err(error) => Err(io_error(
            "could not inspect replay ownership marker",
            &error,
        )),
    }
}

fn validate_store(directory: &Path) -> Result<(), ReplayStoreError> {
    let directory_metadata = fs::symlink_metadata(directory)
        .map_err(|error| io_error("could not inspect replay store directory", &error))?;
    if !directory_metadata.file_type().is_dir() {
        return Err(ReplayStoreError::new(
            "replay store path is not a regular directory",
        ));
    }
    let marker = directory.join(STORE_MARKER);
    let marker_metadata = fs::symlink_metadata(&marker)
        .map_err(|error| io_error("could not inspect replay ownership marker", &error))?;
    if !marker_metadata.file_type().is_file()
        || marker_metadata.len() != u64::try_from(STORE_MARKER_CONTENTS.len()).unwrap_or(u64::MAX)
    {
        return Err(ReplayStoreError::new("replay ownership marker is invalid"));
    }
    let mut contents = Vec::with_capacity(STORE_MARKER_CONTENTS.len());
    File::open(&marker)
        .and_then(|file| file.take(128).read_to_end(&mut contents))
        .map_err(|error| io_error("could not read replay ownership marker", &error))?;
    if contents != STORE_MARKER_CONTENTS {
        return Err(ReplayStoreError::new("replay ownership marker is invalid"));
    }
    Ok(())
}

fn directory_has_entries(directory: &Path) -> Result<bool, ReplayStoreError> {
    fs::read_dir(directory)
        .map_err(|error| io_error("could not inspect replay store contents", &error))?
        .next()
        .transpose()
        .map(|entry| entry.is_some())
        .map_err(|error| io_error("could not inspect replay store contents", &error))
}

fn write_marker(directory: &Path, destination: &Path) -> Result<(), ReplayStoreError> {
    let (temporary_path, mut temporary) = create_temporary(directory)?;
    let result = temporary
        .write_all(STORE_MARKER_CONTENTS)
        .and_then(|()| temporary.sync_all())
        .and_then(|()| fs::hard_link(&temporary_path, destination))
        .and_then(|()| fs::remove_file(&temporary_path))
        .and_then(|()| sync_directory(directory));
    if let Err(error) = result {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
        return Err(io_error("could not create replay ownership marker", &error));
    }
    Ok(())
}

fn create_temporary(directory: &Path) -> Result<(PathBuf, File), ReplayStoreError> {
    for _ in 0..32 {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".redunar-replay-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("could not create replay temporary file", &error)),
        }
    }
    Err(ReplayStoreError::new(
        "could not allocate a replay temporary file",
    ))
}

fn write_bounded_matroska(
    destination: &mut File,
    source: &mut impl Read,
    maximum_clip_bytes: u64,
    storage_limit_bytes: u64,
) -> Result<u64, ReplayStoreError> {
    let mut magic = [0_u8; MATROSKA_MAGIC.len()];
    source
        .read_exact(&mut magic)
        .map_err(|error| io_error("replay container is truncated", &error))?;
    if magic != MATROSKA_MAGIC {
        return Err(ReplayStoreError::new("replay container is not Matroska"));
    }
    destination
        .write_all(&magic)
        .map_err(|error| io_error("could not write replay temporary file", &error))?;
    let hard_limit = maximum_clip_bytes.min(storage_limit_bytes);
    let mut total = u64::try_from(magic.len()).unwrap_or(u64::MAX);
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| io_error("could not read replay container", &error))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| ReplayStoreError::new("replay container size overflows"))?;
        if total > hard_limit {
            return Err(ReplayStoreError::new(
                "replay container exceeds its configured byte budget",
            ));
        }
        destination
            .write_all(&buffer[..read])
            .map_err(|error| io_error("could not write replay temporary file", &error))?;
    }
    Ok(total)
}

fn write_bounded_mp4(
    destination: &mut File,
    source: &mut impl Read,
    maximum_clip_bytes: u64,
    storage_limit_bytes: u64,
) -> Result<u64, ReplayStoreError> {
    let mut header = [0_u8; 8];
    source
        .read_exact(&mut header)
        .map_err(|error| io_error("replay MP4 is truncated", &error))?;
    if header[4..] != MP4_FILE_TYPE_BOX {
        return Err(ReplayStoreError::new("replay container is not MP4"));
    }
    destination
        .write_all(&header)
        .map_err(|error| io_error("could not write replay temporary file", &error))?;
    let hard_limit = maximum_clip_bytes.min(storage_limit_bytes);
    let mut total = u64::try_from(header.len()).unwrap_or(u64::MAX);
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| io_error("could not read replay MP4", &error))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| ReplayStoreError::new("replay container size overflows"))?;
        if total > hard_limit {
            return Err(ReplayStoreError::new(
                "replay container exceeds its configured byte budget",
            ));
        }
        destination
            .write_all(&buffer[..read])
            .map_err(|error| io_error("could not write replay temporary file", &error))?;
    }
    Ok(total)
}

fn commit_temporary(
    directory: &Path,
    temporary: &Path,
    extension: &str,
) -> Result<PathBuf, ReplayStoreError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ReplayStoreError::new("system clock is before the Unix epoch"))?;
    for _ in 0..32 {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "{CLIP_PREFIX}{}-{:09}-{id}.{extension}",
            now.as_secs(),
            now.subsec_nanos()
        ));
        match fs::hard_link(temporary, &path) {
            Ok(()) => {
                fs::remove_file(temporary).map_err(|error| {
                    ReplayStoreError::owned(format!(
                        "replay clip committed but temporary link removal failed: kind={:?} errno={:?} detail={error}; clip={}",
                        error.kind(), error.raw_os_error(),
                        path.display()
                    ))
                })?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("could not commit replay clip", &error)),
        }
    }
    Err(ReplayStoreError::new(
        "could not allocate a replay clip name",
    ))
}

fn scan_owned_clips(directory: &Path) -> Result<Vec<OwnedClip>, ReplayStoreError> {
    let mut clips = Vec::new();
    for (index, entry) in fs::read_dir(directory)
        .map_err(|error| io_error("could not scan replay store", &error))?
        .enumerate()
    {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err(ReplayStoreError::new(
                "replay store contains too many directory entries",
            ));
        }
        let entry = entry.map_err(|error| io_error("could not scan replay store", &error))?;
        if !is_owned_clip_name(&entry.file_name()) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| io_error("could not inspect replay clip", &error))?;
        if metadata.file_type().is_file() {
            clips.push(OwnedClip {
                path: entry.path(),
                bytes: metadata.len(),
                modified: metadata.modified().unwrap_or(UNIX_EPOCH),
            });
        }
    }
    Ok(clips)
}

pub(crate) fn is_owned_clip_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(prefixed) = name.strip_prefix(CLIP_PREFIX) else {
        return false;
    };
    let middle = prefixed
        .strip_suffix(MATROSKA_SUFFIX)
        .or_else(|| prefixed.strip_suffix(MP4_SUFFIX));
    let Some(middle) = middle else {
        return false;
    };
    let mut fields = middle.split('-');
    fields
        .by_ref()
        .take(3)
        .all(|field| !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()))
        && fields.next().is_none()
}

fn enforce_quota(
    directory: &Path,
    clips: &mut Vec<OwnedClip>,
    storage_limit_bytes: u64,
    protected: Option<&Path>,
) -> Result<(u32, u64), ReplayStoreError> {
    clips.sort_by(|left, right| {
        let left_is_protected = protected.is_some_and(|path| left.path == path);
        let right_is_protected = protected.is_some_and(|path| right.path == path);
        left_is_protected.cmp(&right_is_protected).then_with(|| {
            left.modified
                .cmp(&right.modified)
                .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
        })
    });
    let mut total = clips.iter().try_fold(0_u64, |total, clip| {
        total
            .checked_add(clip.bytes)
            .ok_or_else(|| ReplayStoreError::new("replay store size overflows"))
    })?;
    let mut removed = 0_u32;
    let mut removed_bytes = 0_u64;
    for clip in clips {
        if total <= storage_limit_bytes {
            break;
        }
        fs::remove_file(&clip.path)
            .map_err(|error| io_error("could not remove an old Redunar replay clip", &error))?;
        total = total.saturating_sub(clip.bytes);
        removed = removed.saturating_add(1);
        removed_bytes = removed_bytes.saturating_add(clip.bytes);
    }
    if removed > 0 {
        sync_directory(directory)
            .map_err(|error| io_error("could not flush replay quota cleanup", &error))?;
    }
    Ok((removed, removed_bytes))
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

fn io_error(context: &str, error: &io::Error) -> ReplayStoreError {
    ReplayStoreError::owned(format!(
        "{context}: kind={:?} errno={:?} detail={error}",
        error.kind(),
        error.raw_os_error()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_gate_preserves_fixed_reserve_with_saturating_arithmetic() {
        assert!(has_required_free_space(
            MINIMUM_FILESYSTEM_RESERVE_BYTES + 4096,
            4096
        ));
        assert!(!has_required_free_space(
            MINIMUM_FILESYSTEM_RESERVE_BYTES + 4095,
            4096
        ));
        assert!(!has_required_free_space(u64::MAX, u64::MAX));
    }
    use crate::{EncodedReplayPacket, ReplayPacketFormat, ReplayVideoCodec, ReplayVideoStream};
    use redunar_core::ReplaySettings;
    use std::io::Cursor;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("redunar-replay-store-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Self { root }
        }

        fn store(&self, maximum_clip_bytes: u64, storage_limit_bytes: u64) -> ReplayClipStore {
            ReplayClipStore::open_with_limits(
                self.root.clone(),
                maximum_clip_bytes,
                storage_limit_bytes,
            )
            .expect("open store")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn matroska(bytes: usize, fill: u8) -> Cursor<Vec<u8>> {
        let mut data = Vec::with_capacity(bytes.max(MATROSKA_MAGIC.len()));
        data.extend_from_slice(&MATROSKA_MAGIC);
        data.resize(bytes.max(MATROSKA_MAGIC.len()), fill);
        Cursor::new(data)
    }

    fn mp4(bytes: usize, fill: u8) -> Cursor<Vec<u8>> {
        let mut data = vec![0, 0, 0, 24];
        data.extend_from_slice(&MP4_FILE_TYPE_BOX);
        data.resize(bytes.max(8), fill);
        Cursor::new(data)
    }

    fn h264_stream() -> ReplayVideoStream {
        ReplayVideoStream::new(
            ReplayVideoCodec::H264,
            ReplayPacketFormat::H264LengthPrefixed4,
            1_920,
            1_080,
            60,
            vec![
                1, 66, 0, 30, 0xff, 0xe1, 0, 2, 0x67, 0x42, 1, 0, 2, 0x68, 0xce,
            ],
        )
        .expect("valid stream")
    }

    #[test]
    fn refuses_to_claim_relative_or_nonempty_unowned_directories() {
        assert!(ReplayClipStore::open_with_limits(PathBuf::from("relative"), 32, 64).is_err());

        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.root).expect("create fixture");
        fs::write(fixture.root.join("user-file"), b"keep").expect("write user file");
        let error = ReplayClipStore::open_with_limits(fixture.root.clone(), 32, 64)
            .expect_err("unowned non-empty directory must fail");
        assert!(error.to_string().contains("refusing to claim"));
        assert_eq!(
            fs::read(fixture.root.join("user-file")).expect("read user file"),
            b"keep"
        );
    }

    #[test]
    fn read_only_inspection_does_not_initialize_a_missing_store() {
        let fixture = Fixture::new();
        let status = ReplayClipStore::inspect_with_limits(fixture.root.clone(), 32, 64)
            .expect("inspect missing store");
        assert_eq!(
            status,
            ReplayStorageStatus {
                directory: fixture.root.clone(),
                initialized: false,
                owned_clips: 0,
                used_bytes: 0,
                storage_limit_bytes: 64,
                bytes_over_limit: 0,
            }
        );
        assert!(!fixture.root.exists());
        assert!(
            ReplayClipStore::open_existing(
                fixture.root.clone(),
                ReplayBudget::from_settings(ReplaySettings::default()),
            )
            .is_err()
        );
        assert!(!fixture.root.exists());
    }

    #[test]
    fn read_only_inspection_counts_only_owned_regular_clips_and_reports_overage() {
        let fixture = Fixture::new();
        let _store = fixture.store(64, 128);
        fs::write(
            fixture.root.join("redunar-replay-1-000000001-1.mkv"),
            vec![1; 48],
        )
        .expect("write owned clip");
        fs::write(fixture.root.join("important.mkv"), vec![2; 80]).expect("write unknown file");
        let status = ReplayClipStore::inspect_with_limits(fixture.root.clone(), 32, 40)
            .expect("inspect owned store");
        assert!(status.initialized);
        assert_eq!(status.owned_clips, 1);
        assert_eq!(status.used_bytes, 48);
        assert_eq!(status.storage_limit_bytes, 40);
        assert_eq!(status.bytes_over_limit, 8);
        assert_eq!(
            fs::read(fixture.root.join("important.mkv")).unwrap(),
            vec![2; 80]
        );
    }

    #[test]
    fn read_only_inspection_rejects_an_unowned_existing_directory() {
        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.root).expect("create fixture");
        fs::write(fixture.root.join("important.mkv"), b"keep").expect("write unknown file");
        assert!(ReplayClipStore::inspect_with_limits(fixture.root.clone(), 32, 64).is_err());
        assert_eq!(
            fs::read(fixture.root.join("important.mkv")).expect("unknown file retained"),
            b"keep"
        );
    }

    #[test]
    fn save_is_private_atomic_and_prunes_only_old_owned_clips() {
        let fixture = Fixture::new();
        let store = fixture.store(64, 80);
        let unknown = fixture.root.join("important.mkv");
        fs::write(&unknown, b"user").expect("write unknown file");
        let matching_symlink = fixture.root.join("redunar-replay-1-000000001-1.mkv");
        symlink(&unknown, &matching_symlink).expect("create matching symlink");

        let first = store
            .save_matroska(&mut matroska(48, 1))
            .expect("save first clip");
        let second = store
            .save_matroska(&mut matroska(48, 2))
            .expect("save second clip");
        assert_eq!(second.clips_removed_for_quota, 1);
        assert_eq!(second.bytes_removed_for_quota, 48);
        assert!(!first.path.exists());
        assert!(second.path.exists());
        assert_eq!(fs::read(&unknown).expect("unknown file retained"), b"user");
        assert!(
            fs::symlink_metadata(&matching_symlink)
                .expect("symlink retained")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&second.path)
                .expect("clip metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(
            fs::read_dir(&fixture.root)
                .expect("scan fixture")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn edited_mp4_preserves_existing_clips_or_rejects_the_commit() {
        let fixture = Fixture::new();
        let store = fixture.store(48, 48);
        let original = store
            .save_matroska(&mut matroska(32, 1))
            .expect("save original clip");

        let error = store
            .save_mp4_preserving_existing(&mut mp4(24, 2))
            .expect_err("edit must not evict its source");

        assert!(error.to_string().contains("without removing"));
        assert!(original.path.exists());
        assert_eq!(store.inventory().expect("inventory").len(), 1);
    }

    #[test]
    fn edited_mp4_commits_without_pruning_when_both_files_fit() {
        let fixture = Fixture::new();
        let store = fixture.store(64, 80);
        let original = store
            .save_matroska(&mut matroska(32, 1))
            .expect("save original clip");

        let edited = store
            .save_mp4_preserving_existing(&mut mp4(24, 2))
            .expect("save edited clip");

        assert_eq!(edited.clips_removed_for_quota, 0);
        assert!(original.path.exists());
        assert!(edited.path.exists());
        assert_eq!(store.inventory().expect("inventory").len(), 2);
    }

    #[test]
    fn inventory_and_delete_are_limited_to_owned_regular_clip_names() {
        let fixture = Fixture::new();
        let store = fixture.store(64, 256);
        let first = store
            .save_matroska(&mut matroska(32, 1))
            .expect("save first clip");
        let second = store
            .save_matroska(&mut matroska(48, 2))
            .expect("save second clip");
        let unknown = fixture.root.join("important.mkv");
        fs::write(&unknown, b"keep").expect("write unknown file");
        let matching_symlink = fixture.root.join("redunar-replay-1-000000001-1.mkv");
        symlink(&unknown, &matching_symlink).expect("create matching symlink");

        let inventory = store.inventory().expect("inventory");
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory.iter().map(|clip| clip.bytes).sum::<u64>(), 80);
        assert!(
            inventory
                .iter()
                .all(|clip| clip.path.starts_with(&fixture.root))
        );
        assert!(
            inventory
                .iter()
                .all(|clip| clip.file_name == clip.path.file_name().unwrap().to_string_lossy())
        );

        assert!(store.delete_clip("../important.mkv").is_err());
        assert!(store.delete_clip("important.mkv").is_err());
        assert!(
            store
                .delete_clip("redunar-replay-1-000000001-1.mkv")
                .is_err()
        );
        assert_eq!(fs::read(&unknown).expect("unknown retained"), b"keep");
        let deleted_name = second.path.file_name().unwrap().to_string_lossy();
        let deleted = store.delete_clip(&deleted_name).expect("delete owned clip");
        assert_eq!(deleted.path, second.path);
        assert_eq!(deleted.bytes, 48);
        assert!(!second.path.exists());
        assert!(first.path.exists());
        assert_eq!(store.inventory().expect("inventory after delete").len(), 1);
    }

    #[test]
    fn archive_moves_owned_clip_under_original_without_inventory_pollution() {
        let fixture = Fixture::new();
        let store = fixture.store(128, 256);
        let clip = store
            .save_matroska(&mut matroska(48, 7))
            .expect("save clip");
        let file_name = clip
            .path
            .file_name()
            .expect("clip name")
            .to_string_lossy()
            .into_owned();

        let archived = store.archive_clip(&file_name).expect("archive clip");
        assert_eq!(archived, fixture.root.join("original").join(&file_name));
        assert!(!clip.path.exists());
        assert!(archived.is_file());
        assert_eq!(fs::read(&archived).expect("archived bytes").len(), 48);
        assert_eq!(store.inventory().expect("inventory after archive").len(), 0);
        assert!(store.archive_clip(&file_name).is_err());
        assert_eq!(
            fs::metadata(fixture.root.join("original"))
                .expect("archive metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn malformed_and_oversized_containers_leave_no_partial_clip() {
        let fixture = Fixture::new();
        let store = fixture.store(16, 32);
        assert!(
            store
                .save_matroska(&mut Cursor::new(b"not-matroska".to_vec()))
                .is_err()
        );
        assert!(store.save_matroska(&mut matroska(17, 3)).is_err());
        let owned = scan_owned_clips(&fixture.root).expect("scan clips");
        assert!(owned.is_empty());
        assert!(
            fs::read_dir(&fixture.root)
                .expect("scan fixture")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn encoded_ring_is_muxed_directly_into_an_atomic_video_only_clip() {
        let fixture = Fixture::new();
        let store = fixture.store(4_096, 8_192);
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(
            EncodedReplayPacket::new(1, 16_666_667, true, vec![0, 0, 0, 2, 0x65, 0x88])
                .expect("packet"),
        )
        .expect("ring");
        let saved = store
            .save_video_only(&h264_stream(), &ring)
            .expect("save generated clip");
        let contents = fs::read(&saved.path).expect("read clip");
        assert_eq!(&contents[..4], &MATROSKA_MAGIC);
        assert!(
            contents
                .windows(b"V_MPEG4/ISO/AVC".len())
                .any(|bytes| bytes == b"V_MPEG4/ISO/AVC")
        );
        assert_eq!(saved.bytes, contents.len() as u64);
    }

    #[test]
    fn mp4_ring_is_committed_and_included_in_owned_inventory() {
        let fixture = Fixture::new();
        let store = fixture.store(4_096, 8_192);
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(
            EncodedReplayPacket::new(1, 16_666_667, true, vec![0, 0, 0, 2, 0x65, 0x88])
                .expect("packet"),
        )
        .expect("ring");
        let saved = store
            .save_video_only_as(&h264_stream(), &ring, ReplayOutputFormat::Mp4)
            .expect("save generated MP4 clip");
        assert_eq!(saved.path.extension().and_then(OsStr::to_str), Some("mp4"));
        let contents = fs::read(&saved.path).expect("read MP4 clip");
        assert_eq!(&contents[4..8], b"ftyp");
        assert_eq!(store.inventory().expect("inventory").len(), 1);
    }

    #[test]
    fn invalid_encoder_framing_leaves_no_generated_clip_or_temporary_file() {
        let fixture = Fixture::new();
        let store = fixture.store(4_096, 8_192);
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(
            EncodedReplayPacket::new(1, 16_666_667, true, vec![0, 0, 0, 9, 1])
                .expect("ring accepts opaque encoded bytes"),
        )
        .expect("ring");
        assert!(store.save_video_only(&h264_stream(), &ring).is_err());
        assert!(
            scan_owned_clips(&fixture.root)
                .expect("scan clips")
                .is_empty()
        );
        assert!(
            fs::read_dir(&fixture.root)
                .expect("scan fixture")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn replaced_or_corrupt_ownership_marker_blocks_cleanup() {
        let fixture = Fixture::new();
        let store = fixture.store(32, 64);
        fs::write(fixture.root.join(STORE_MARKER), b"wrong").expect("corrupt marker");
        assert!(store.enforce_quota().is_err());
    }

    #[test]
    fn symlinked_ownership_marker_blocks_cleanup_without_touching_target() {
        let fixture = Fixture::new();
        let store = fixture.store(32, 64);
        let target = fixture.root.join("important");
        fs::write(&target, b"keep").expect("write target");
        fs::remove_file(fixture.root.join(STORE_MARKER)).expect("remove marker");
        symlink(&target, fixture.root.join(STORE_MARKER)).expect("replace marker with symlink");
        assert!(store.enforce_quota().is_err());
        assert_eq!(fs::read(target).expect("target retained"), b"keep");
    }

    #[test]
    fn quota_never_removes_the_newly_committed_clip_before_older_clips() {
        let fixture = Fixture::new();
        let _store = fixture.store(64, 64);
        let older = fixture.root.join("redunar-replay-1-000000001-1.mkv");
        let protected = fixture.root.join("redunar-replay-2-000000001-2.mkv");
        fs::write(&older, vec![1; 48]).expect("write old clip");
        fs::write(&protected, vec![2; 48]).expect("write protected clip");
        let mut clips = vec![
            OwnedClip {
                path: protected.clone(),
                bytes: 48,
                modified: UNIX_EPOCH,
            },
            OwnedClip {
                path: older.clone(),
                bytes: 48,
                modified: SystemTime::now(),
            },
        ];
        let removed =
            enforce_quota(&fixture.root, &mut clips, 64, Some(&protected)).expect("enforce quota");
        assert_eq!(removed, (1, 48));
        assert!(!older.exists());
        assert!(protected.exists());
    }
}
