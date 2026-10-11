use crate::replay_packet_source::ReplayPacketSource;
use crate::{
    EncodedReplayPacket, ReplayBudget, ReplayClipStore, ReplayOutputFormat, ReplayVideoStream,
    StoredReplayClip,
};
use redunar_core::{ReplayDuration, ReplayFrameRate, ReplayQuality, ReplaySettings};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MARKER: &str = ".redunar-replay-spool-v1";
const MARKER_CONTENTS: &[u8] = b"redunar-replay-spool-v1\n";
const SEGMENT_PREFIX: &str = "redunar-segment-";
const SEGMENT_SUFFIX: &str = ".rseg";
const TEMP_SUFFIX: &str = ".tmp";
const SEGMENT_MAGIC: &[u8; 8] = b"RDNSEG01";
const MIN_SAVABLE_HISTORY_NS: u64 = NANOSECONDS_PER_SECOND;
const TARGET_SEGMENT_NS: u64 = 2_000_000_000;
const MAX_SEGMENT_NS: u64 = 4_000_000_000;
const MAX_SEGMENT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SEGMENTS: usize = 512;
// Encoders can hand packets to the spool in short bursts (especially when a
// hardware encoder flushes several frames at once). Four entries allowed a
// normal burst to overflow immediately, and one dropped packet forced a
// keyframe resync that became a multi-second hole in the replay. Keep the
// queue bounded, but large enough to absorb roughly a second of encoded work
// at 60 FPS without blocking the game thread. Higher frame rates or sustained
// storage stalls can still overflow it and require keyframe recovery.
const COMMAND_CAPACITY: usize = 64;
const MAX_WRITE_LATENCY: Duration = Duration::from_millis(500);
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
const BYTES_PER_MEGABIT: u64 = 1_000_000 / 8;
const RETAINED_DURATION: ReplayDuration = ReplayDuration::Seconds900;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySpoolPhase {
    Running,
    SlowStorage,
    Failed,
    Shutdown,
}

impl ReplaySpoolPhase {
    const fn code(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::SlowStorage => 1,
            Self::Failed => 2,
            Self::Shutdown => 3,
        }
    }

    const fn from_code(value: u8) -> Self {
        match value {
            0 => Self::Running,
            1 => Self::SlowStorage,
            2 => Self::Failed,
            _ => Self::Shutdown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySpoolSegment {
    pub path: PathBuf,
    pub start_timestamp_ns: u64,
    pub end_timestamp_ns: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySpoolSnapshotPlan {
    pub segments: Vec<ReplaySpoolSegment>,
    pub requested_duration: ReplayDuration,
    pub available_duration_ns: u64,
    pub generation: u64,
}

#[derive(Debug)]
pub struct ReplayAssemblyJob {
    worker: Option<JoinHandle<Result<StoredReplayClip, ReplaySpoolError>>>,
    cancel: Arc<AtomicBool>,
}

impl ReplayAssemblyJob {
    /// Wait for the isolated clip assembler. Recording and spool rotation may
    /// continue while this worker owns open handles to the selected segments.
    ///
    /// # Errors
    ///
    /// Returns an assembly, validation, storage, or worker failure.
    pub fn join(mut self) -> Result<StoredReplayClip, ReplaySpoolError> {
        self.worker
            .take()
            .ok_or_else(|| ReplaySpoolError::new("assembly worker already joined"))?
            .join()
            .map_err(|_| ReplaySpoolError::new("Replay clip assembler panicked"))?
    }

    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
}

impl Drop for ReplayAssemblyJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplaySpoolStats {
    pub active_segments: u32,
    pub active_bytes: u64,
    pub buffered_duration_ns: u64,
    /// Total encoded packets durably accepted by this spool epoch.
    pub accepted_packets: u64,
    pub queued_packets: u32,
    pub dropped_queue_full: u64,
    pub dropped_awaiting_keyframe: u64,
    pub recovered_segments: u32,
    /// Packet commits that exceeded the bounded write latency. These remain
    /// in the running phase; sustained slowness degrades through the bounded
    /// queue drop path instead of stopping the spool.
    pub slow_writes: u64,
    /// Times disk history discarded a partial segment after a transient
    /// commit failure and resynchronized at the next keyframe.
    pub segment_resyncs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySpoolError {
    message: String,
    kind: ReplaySpoolErrorKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplaySpoolErrorKind {
    General,
    InsufficientHistory,
}

impl ReplaySpoolError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ReplaySpoolErrorKind::General,
        }
    }

    fn insufficient_history() -> Self {
        Self {
            message: "Replay just started; try saving again in a moment".to_owned(),
            kind: ReplaySpoolErrorKind::InsufficientHistory,
        }
    }

    pub(crate) const fn is_insufficient_history(&self) -> bool {
        matches!(self.kind, ReplaySpoolErrorKind::InsufficientHistory)
    }
}

impl fmt::Display for ReplaySpoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ReplaySpoolError {}

#[derive(Debug)]
pub enum ReplaySpoolSubmitError {
    QueueFull(EncodedReplayPacket),
    NotRunning(EncodedReplayPacket),
}

impl fmt::Display for ReplaySpoolSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::QueueFull(_) => "Replay spool queue is full",
            Self::NotRunning(_) => "Replay spool is not running",
        })
    }
}

impl Error for ReplaySpoolSubmitError {}

enum SpoolCommand {
    Packet(EncodedReplayPacket),
    #[cfg(test)]
    SealSnapshot(mpsc::Sender<Result<(), ReplaySpoolError>>),
    PrepareSnapshot {
        requested: ReplayDuration,
        response: mpsc::Sender<Result<Vec<(ReplaySpoolSegment, File)>, ReplaySpoolError>>,
        cancel: Arc<AtomicBool>,
    },
    ResetEpoch,
    Shutdown,
}

#[derive(Clone, Copy)]
struct SpoolLimits {
    duration_ns: u64,
    bytes: u64,
}

impl SpoolLimits {
    fn new(duration: ReplayDuration, quality: ReplayQuality, frame_rate: ReplayFrameRate) -> Self {
        let seconds = u64::from(duration.seconds());
        let rate_multiplier = if frame_rate == ReplayFrameRate::Variable {
            2
        } else {
            1
        };
        let video_bytes = seconds
            .saturating_mul(u64::from(quality.target_megabits_per_second()))
            .saturating_mul(BYTES_PER_MEGABIT)
            .saturating_mul(rate_multiplier);
        Self {
            duration_ns: seconds.saturating_mul(NANOSECONDS_PER_SECOND),
            bytes: video_bytes.saturating_mul(110).div_ceil(100),
        }
    }
}

#[derive(Default)]
struct SpoolIndex {
    segments: VecDeque<ReplaySpoolSegment>,
    active_bytes: u64,
    generation: u64,
    recovered_segments: u32,
    dropped_awaiting_keyframe: u64,
}

struct SharedStats {
    // Published only after filesystem work; readers never take the disk index lock.
    index_snapshot: Mutex<ReplaySpoolStats>,
    phase: AtomicU8,
    queued_packets: AtomicU32,
    dropped_queue_full: AtomicU64,
    discontinuity: AtomicU64,
    first_packet_timestamp_ns: AtomicU64,
    latest_packet_end_ns: AtomicU64,
    accepted_packets: AtomicU64,
    slow_writes: AtomicU64,
    segment_resyncs: AtomicU64,
}

/// Asynchronous, disk-segment rolling history for already-encoded packets.
///
/// The presentation/encoder caller performs one bounded `try_send`; all file
/// creation, writes, flushes, renames, scans, and cleanup happen on the worker.
/// The queue retains at most four packet payloads (32 MiB worst case under the
/// existing 8 MiB packet contract), independent of the selected 15-second to
/// 15-minute interval. This type has no raw-frame or CPU-encoder input.
pub struct ReplaySegmentSpool {
    sender: SyncSender<SpoolCommand>,
    worker: Option<JoinHandle<()>>,
    index: Arc<Mutex<SpoolIndex>>,
    shared: Arc<SharedStats>,
}

impl ReplaySegmentSpool {
    /// Open an exact owned spool, recover complete segments, remove abandoned
    /// temporary files, enforce current limits, and start the I/O worker.
    ///
    /// # Errors
    ///
    /// Returns [`ReplaySpoolError`] for unsafe paths, ownership mismatch,
    /// excessive entries, malformed owned segments, I/O, or worker startup.
    pub fn open(
        directory: impl Into<PathBuf>,
        settings: ReplaySettings,
    ) -> Result<Self, ReplaySpoolError> {
        let directory = directory.into();
        if !directory.is_absolute() {
            return Err(ReplaySpoolError::new("Replay spool path must be absolute"));
        }
        initialize_spool(&directory)?;
        let limits = SpoolLimits::new(RETAINED_DURATION, settings.quality, settings.frame_rate);
        let mut index = recover_index(&directory)?;
        enforce_limits(&directory, &mut index, limits)?;
        let index = Arc::new(Mutex::new(index));
        let shared = Arc::new(SharedStats {
            index_snapshot: Mutex::new(index_stats(&lock_index(&index))),
            phase: AtomicU8::new(ReplaySpoolPhase::Running.code()),
            queued_packets: AtomicU32::new(0),
            dropped_queue_full: AtomicU64::new(0),
            discontinuity: AtomicU64::new(0),
            first_packet_timestamp_ns: AtomicU64::new(u64::MAX),
            latest_packet_end_ns: AtomicU64::new(0),
            accepted_packets: AtomicU64::new(0),
            slow_writes: AtomicU64::new(0),
            segment_resyncs: AtomicU64::new(0),
        });
        let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let worker_index = Arc::clone(&index);
        let worker_shared = Arc::clone(&shared);
        let worker_directory = directory.clone();
        let worker = thread::Builder::new()
            .name("redunar-replay-spool".to_owned())
            .spawn(move || {
                run_worker(
                    &receiver,
                    &worker_directory,
                    limits,
                    &worker_index,
                    &worker_shared,
                );
            })
            .map_err(|error| {
                ReplaySpoolError::new(format!("could not start Replay spool: {error}"))
            })?;
        Ok(Self {
            sender,
            worker: Some(worker),
            index,
            shared,
        })
    }

    /// Queue one already-encoded packet without waiting for disk I/O.
    ///
    /// # Errors
    ///
    /// Returns the packet to the caller when the fixed queue is full or the
    /// worker is no longer running. Dropping is preferred over game blocking.
    pub fn try_submit(&self, packet: EncodedReplayPacket) -> Result<(), ReplaySpoolSubmitError> {
        if self.phase() != ReplaySpoolPhase::Running {
            return Err(ReplaySpoolSubmitError::NotRunning(packet));
        }
        match self.sender.try_send(SpoolCommand::Packet(packet)) {
            Ok(()) => {
                self.shared.queued_packets.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(SpoolCommand::Packet(packet))) => {
                self.shared
                    .dropped_queue_full
                    .fetch_add(1, Ordering::Relaxed);
                self.shared.discontinuity.fetch_add(1, Ordering::Release);
                Err(ReplaySpoolSubmitError::QueueFull(packet))
            }
            Err(TrySendError::Disconnected(SpoolCommand::Packet(packet))) => {
                Err(ReplaySpoolSubmitError::NotRunning(packet))
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                unreachable!("try_submit sends only packet commands")
            }
        }
    }

    /// Create one immutable keyframe-safe manifest plan from complete owned
    /// segments. This performs no file reads or writes. The assembler must
    /// revalidate every exact path before opening it because rolling cleanup
    /// may advance after this snapshot generation.
    ///
    /// # Errors
    ///
    /// Returns [`ReplaySpoolError`] when the requested interval exceeds the
    /// currently configured history or no complete segment is available.
    pub fn snapshot_plan(
        &self,
        requested: ReplayDuration,
    ) -> Result<ReplaySpoolSnapshotPlan, ReplaySpoolError> {
        if requested.seconds() > RETAINED_DURATION.seconds() {
            return Err(ReplaySpoolError::new(
                "requested Replay interval exceeds retained history",
            ));
        }
        plan_snapshot(&lock_index(&self.index), requested)
    }

    /// Start assembling one keyframe-safe suffix without pausing recording.
    ///
    /// The spool worker seals and pins the selected files at the ordered
    /// command boundary, before processing further packets or retention. Their
    /// descriptors remain valid after unlink. The assembler waits separately
    /// and streams validated packets without copying payloads on the caller.
    ///
    /// # Errors
    ///
    /// Returns an error when the request queue is full or the assembler cannot
    /// start. Snapshot and assembly failures are returned by the job's join.
    pub fn save_async(
        &self,
        requested: ReplayDuration,
        settings: ReplaySettings,
        stream: ReplayVideoStream,
        store: ReplayClipStore,
    ) -> Result<ReplayAssemblyJob, ReplaySpoolError> {
        self.save_async_tracks_as(
            requested,
            settings,
            stream,
            store,
            ReplayOutputFormat::Matroska,
            crate::ReplayAudioTracks {
                system: crate::ReplayAudioSnapshot {
                    stream: redunar_capture_audio::OpusStreamDescription::default(),
                    packets: Vec::new(),
                },
                microphone: None,
            },
        )
    }

    /// Assemble a snapshot into the selected supported output container.
    ///
    /// # Errors
    ///
    /// Returns an error under the same bounded snapshot, ownership, and worker
    /// startup conditions as [`Self::save_async`].
    pub fn save_async_as(
        &self,
        requested: ReplayDuration,
        settings: ReplaySettings,
        stream: ReplayVideoStream,
        store: ReplayClipStore,
        output_format: ReplayOutputFormat,
        audio: crate::ReplayAudioSnapshot,
    ) -> Result<ReplayAssemblyJob, ReplaySpoolError> {
        self.save_async_tracks_as(
            requested,
            settings,
            stream,
            store,
            output_format,
            crate::ReplayAudioTracks {
                system: audio,
                microphone: None,
            },
        )
    }

    /// Save optional independent audio timelines with the selected video window.
    ///
    /// # Errors
    /// Returns an error when the snapshot queue, worker or spool is unavailable.
    pub fn save_async_tracks_as(
        &self,
        requested: ReplayDuration,
        settings: ReplaySettings,
        stream: ReplayVideoStream,
        store: ReplayClipStore,
        output_format: ReplayOutputFormat,
        audio: crate::ReplayAudioTracks,
    ) -> Result<ReplayAssemblyJob, ReplaySpoolError> {
        self.save_async_tracks_with_audio(
            requested,
            settings,
            stream,
            store,
            output_format,
            move |start, end| {
                let mut audio = audio;
                let in_window = |packet: &redunar_capture_audio::EncodedOpusPacket| {
                    packet.timestamp_ns < end
                        && packet.timestamp_ns.saturating_add(packet.duration_ns) > start
                };
                audio.system.packets.retain(in_window);
                if let Some(input) = &mut audio.microphone {
                    input.packets.retain(in_window);
                }
                audio
            },
        )
    }

    pub(crate) fn save_async_tracks_with_audio(
        &self,
        requested: ReplayDuration,
        mut settings: ReplaySettings,
        stream: ReplayVideoStream,
        store: ReplayClipStore,
        output_format: ReplayOutputFormat,
        audio_snapshot: impl FnOnce(u64, u64) -> crate::ReplayAudioTracks + Send + 'static,
    ) -> Result<ReplayAssemblyJob, ReplaySpoolError> {
        let cancel = Arc::new(AtomicBool::new(false));
        if requested.seconds() > RETAINED_DURATION.seconds()
            || self.phase() != ReplaySpoolPhase::Running
        {
            return Err(ReplaySpoolError::new("Replay snapshot is unavailable"));
        }
        let (response, receiver) = mpsc::channel();
        self.sender
            .try_send(SpoolCommand::PrepareSnapshot {
                requested,
                response,
                cancel: Arc::clone(&cancel),
            })
            .map_err(|_| ReplaySpoolError::new("Replay spool is busy; retry saving"))?;
        settings.duration = requested;
        let job_cancel = Arc::clone(&cancel);
        let worker = thread::Builder::new()
            .name("redunar-replay-assembler".to_owned())
            .spawn(move || {
                let started = Instant::now();
                let opened = loop {
                    if cancel.load(Ordering::Acquire) || started.elapsed() >= Duration::from_secs(5)
                    {
                        cancel.store(true, Ordering::Release);
                        return Err(ReplaySpoolError::new(
                            "Replay snapshot cancelled or timed out",
                        ));
                    }
                    match receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok(result) => break result?,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(ReplaySpoolError::new(
                                "Replay spool stopped during snapshot",
                            ));
                        }
                    }
                };
                assemble_snapshot(
                    opened,
                    settings,
                    &stream,
                    &store,
                    output_format,
                    audio_snapshot,
                    &cancel,
                )
            })
            .map_err(|_| ReplaySpoolError::new("could not start Replay clip assembler"))?;
        Ok(ReplayAssemblyJob {
            worker: Some(worker),
            cancel: job_cancel,
        })
    }

    /// Commit the currently writable segment after every packet already
    /// accepted by the bounded queue. This control-path wait does not perform
    /// file I/O on the encoder thread; it only establishes an ordered boundary
    /// so a save includes frames immediately preceding the request.
    #[cfg(test)]
    fn seal_active_tail(&self) -> Result<(), ReplaySpoolError> {
        if self.phase() != ReplaySpoolPhase::Running {
            return Err(ReplaySpoolError::new("Replay spool is not running"));
        }
        let (response_sender, response_receiver) = mpsc::channel();
        self.sender
            .send(SpoolCommand::SealSnapshot(response_sender))
            .map_err(|_| ReplaySpoolError::new("Replay spool stopped before snapshot"))?;
        response_receiver
            .recv()
            .map_err(|_| ReplaySpoolError::new("Replay spool stopped during snapshot"))?
    }

    #[must_use]
    pub fn phase(&self) -> ReplaySpoolPhase {
        ReplaySpoolPhase::from_code(self.shared.phase.load(Ordering::Acquire))
    }

    #[must_use]
    pub fn stats(&self) -> ReplaySpoolStats {
        let index = *self
            .shared
            .index_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ReplaySpoolStats {
            active_segments: index.active_segments,
            active_bytes: index.active_bytes,
            buffered_duration_ns: buffered_duration_ns(&self.shared),
            accepted_packets: self.shared.accepted_packets.load(Ordering::Relaxed),
            queued_packets: self.shared.queued_packets.load(Ordering::Relaxed),
            dropped_queue_full: self.shared.dropped_queue_full.load(Ordering::Relaxed),
            dropped_awaiting_keyframe: index.dropped_awaiting_keyframe,
            recovered_segments: index.recovered_segments,
            slow_writes: self.shared.slow_writes.load(Ordering::Relaxed),
            segment_resyncs: self.shared.segment_resyncs.load(Ordering::Relaxed),
        }
    }

    /// Discard the current codec epoch without pausing the packet producer.
    ///
    /// The command is ordered after packets already accepted by the queue and
    /// before packets accepted afterwards. This prevents a source resize or
    /// encoder reset from producing a clip that joins incompatible streams.
    ///
    /// # Errors
    ///
    /// Returns an error if the worker has already stopped. Reset is a rare
    /// control-path operation, so it may wait for the fixed queue to accept
    /// the ordered command; packet submission never waits this way.
    pub fn reset_epoch(&self) -> Result<(), ReplaySpoolError> {
        if self.phase() != ReplaySpoolPhase::Running {
            return Err(ReplaySpoolError::new("Replay spool is not running"));
        }
        self.sender
            .send(SpoolCommand::ResetEpoch)
            .map_err(|_| ReplaySpoolError::new("Replay spool stopped before reset"))
    }

    /// Stop and join the I/O worker. The current partial segment is discarded;
    /// only atomically committed complete segments survive recovery.
    pub fn shutdown(&mut self) {
        if self.worker.is_none() {
            return;
        }
        let _ = self.sender.send(SpoolCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.shared
            .phase
            .store(ReplaySpoolPhase::Shutdown.code(), Ordering::Release);
    }
}

fn plan_snapshot(
    index: &SpoolIndex,
    requested: ReplayDuration,
) -> Result<ReplaySpoolSnapshotPlan, ReplaySpoolError> {
    let newest = index
        .segments
        .back()
        .ok_or_else(|| ReplaySpoolError::new("no complete Replay segment is available"))?;
    let cutoff = newest
        .end_timestamp_ns
        .saturating_sub(u64::from(requested.seconds()) * NANOSECONDS_PER_SECOND);
    let mut segments = index
        .segments
        .iter()
        .rev()
        .take_while(|segment| segment.end_timestamp_ns > cutoff)
        .cloned()
        .collect::<Vec<_>>();
    segments.reverse();
    let available_duration_ns = segments.first().map_or(0, |first| {
        newest
            .end_timestamp_ns
            .saturating_sub(first.start_timestamp_ns)
    });
    if available_duration_ns < MIN_SAVABLE_HISTORY_NS {
        return Err(ReplaySpoolError::insufficient_history());
    }
    Ok(ReplaySpoolSnapshotPlan {
        segments,
        requested_duration: requested,
        available_duration_ns,
        generation: index.generation,
    })
}

fn open_snapshot_segment(segment: &ReplaySpoolSegment) -> Result<File, ReplaySpoolError> {
    let metadata = fs::symlink_metadata(&segment.path)
        .map_err(|error| io_error("could not inspect Replay snapshot segment", &error))?;
    if !metadata.file_type().is_file() || metadata.len() != segment.bytes {
        return Err(ReplaySpoolError::new(
            "Replay snapshot segment changed before assembly",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&segment.path)
        .map_err(|error| io_error("could not open Replay snapshot segment", &error))?;
    let opened = file
        .metadata()
        .map_err(|error| io_error("could not inspect opened snapshot", &error))?;
    if !opened.is_file()
        || opened.ino() != metadata.ino()
        || opened.dev() != metadata.dev()
        || opened.len() != segment.bytes
    {
        return Err(ReplaySpoolError::new(
            "Replay snapshot changed while opening",
        ));
    }
    Ok(file)
}

fn assemble_snapshot(
    segments: Vec<(ReplaySpoolSegment, File)>,
    settings: ReplaySettings,
    stream: &ReplayVideoStream,
    store: &ReplayClipStore,
    output_format: ReplayOutputFormat,
    audio_snapshot: impl FnOnce(u64, u64) -> crate::ReplayAudioTracks,
    cancel: &Arc<AtomicBool>,
) -> Result<StoredReplayClip, ReplaySpoolError> {
    let mut packets =
        crate::replay_spool_reader::SpoolPackets::scan(segments, settings, Arc::clone(cancel))
            .map_err(|error| io_error("could not inspect Replay snapshot packets", &error))?;
    let window = packets.window();
    packets
        .check_cancel()
        .map_err(|error| io_error("Replay snapshot cancelled", &error))?;
    // Only the selected window is copied under the audio lock. Codec work runs
    // here, after the lock is released, and cancellation is checked per frame.
    let audio = crate::replay_audio_mix::mix_audio_tracks(
        audio_snapshot(window.0, window.1),
        window,
        cancel,
    )
    .map_err(|error| io_error("could not mix Replay snapshot audio", &error))?;
    // Clip duration is selected at save time and can be longer than the
    // legacy duration retained in the recording profile. Re-open the owned
    // store with the requested interval's byte budget so long saves are not
    // capped by that stale profile value.
    let requested_store =
        ReplayClipStore::open_existing(store.directory(), ReplayBudget::from_settings(settings))
            .map_err(|error| {
                ReplaySpoolError::new(format!(
                    "could not prepare Replay storage for the requested interval: {error}"
                ))
            })?;
    requested_store
        .ensure_free_space_for_clip()
        .map_err(|error| {
            ReplaySpoolError::new(format!(
                "could not reserve space for assembled Replay: {error}"
            ))
        })?;
    requested_store
        .save_streaming_tracks(stream, &mut packets, &audio, output_format)
        .map_err(|error| {
            ReplaySpoolError::new(format!("could not store assembled Replay: {error}"))
        })
}

impl Drop for ReplaySegmentSpool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct SegmentWriter {
    temporary_path: PathBuf,
    file: File,
    start_timestamp_ns: u64,
    end_timestamp_ns: u64,
    bytes: u64,
    sequence: u64,
}

fn run_worker(
    receiver: &Receiver<SpoolCommand>,
    directory: &Path,
    limits: SpoolLimits,
    index: &Arc<Mutex<SpoolIndex>>,
    shared: &SharedStats,
) {
    let mut current: Option<SegmentWriter> = None;
    let mut next_sequence = lock_index(index).generation.saturating_add(1);
    let mut observed_discontinuity = shared.discontinuity.load(Ordering::Acquire);
    while let Ok(command) = receiver.recv() {
        match command {
            SpoolCommand::Packet(packet) => {
                shared.queued_packets.fetch_sub(1, Ordering::Relaxed);
                let discontinuity = shared.discontinuity.load(Ordering::Acquire);
                if discontinuity != observed_discontinuity {
                    if let Some(writer) = current.take() {
                        let _ = fs::remove_file(writer.temporary_path);
                    }
                    observed_discontinuity = discontinuity;
                    reset_buffered_duration(shared);
                }
                let started = Instant::now();
                if let Err(error) = process_packet(
                    directory,
                    &packet,
                    &mut current,
                    &mut next_sequence,
                    limits,
                    index,
                ) {
                    // A packet write, rotation commit, or segment-creation
                    // failure invalidates only the partial segment. Rolling
                    // history degrades instead of stopping the recorder: the
                    // temporary file is removed, advertised buffered duration
                    // resets honestly, and the next independently decodable
                    // keyframe starts a fresh segment.
                    crate::log_op!("Redunar Replay: disk spool dropped a segment: {error}");
                    if let Some(writer) = current.take() {
                        let _ = fs::remove_file(writer.temporary_path);
                    }
                    shared.segment_resyncs.fetch_add(1, Ordering::Relaxed);
                    reset_buffered_duration(shared);
                    continue;
                }
                record_buffered_packet(shared, &packet);
                if started.elapsed() > MAX_WRITE_LATENCY {
                    // One slow commit is counted, not terminal. Sustained
                    // slowness drains the bounded queue, and the existing
                    // queue-full discontinuity path drops replay work exactly
                    // as a stall should, without ending the recording session.
                    crate::log_op!(
                        "Redunar Replay: disk spool exceeded the bounded write latency; \
                         disk history is degrading through bounded drops"
                    );
                    shared.slow_writes.fetch_add(1, Ordering::Relaxed);
                }
            }
            SpoolCommand::ResetEpoch => {
                if let Some(writer) = current.take()
                    && let Err(error) = fs::remove_file(writer.temporary_path)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    // The partial file is abandoned rather than retried. It
                    // stays invisible to snapshots because the index is
                    // cleared below, and the next spool open removes leftovers.
                    crate::log_op!(
                        "Redunar Replay: disk spool could not remove the active segment on reset: {error}"
                    );
                }
                if let Err(error) = clear_completed_segments(directory, index) {
                    // A half-cleared index could let a new codec epoch save
                    // old-epoch segments, so this stays terminal. The export
                    // pump re-arms a fresh spool, which retries the reset.
                    crate::log_op!("Redunar Replay: completed spool cleanup failed: {error}");
                    shared
                        .phase
                        .store(ReplaySpoolPhase::Failed.code(), Ordering::Release);
                    break;
                }
                reset_buffered_duration(shared);
            }
            #[cfg(test)]
            SpoolCommand::SealSnapshot(response) => {
                let result = current.take().map_or(Ok(()), |writer| {
                    commit_segment(directory, writer, limits, index)
                });
                if result.is_err() {
                    // The save caller receives this error and degrades to the
                    // completed-segment history; the worker itself stays
                    // available for the next keyframe-safe segment.
                    shared.segment_resyncs.fetch_add(1, Ordering::Relaxed);
                    reset_buffered_duration(shared);
                }
                let _ = response.send(result);
            }
            SpoolCommand::PrepareSnapshot {
                requested,
                response,
                cancel,
            } => {
                if cancel.load(Ordering::Acquire) {
                    continue;
                }
                let result = pin_snapshot(&mut current, directory, limits, index, requested);
                // Snapshot errors do not invalidate a healthy encoder. A failed
                // commit already removes its own temporary output.
                let _ = response.send(result);
            }
            SpoolCommand::Shutdown => break,
        }
        let published = index_stats(&lock_index(index));
        *shared
            .index_snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = published;
    }
    if let Some(writer) = current.take() {
        let _ = fs::remove_file(writer.temporary_path);
    }
    shared.queued_packets.store(0, Ordering::Relaxed);
}

fn index_stats(index: &SpoolIndex) -> ReplaySpoolStats {
    ReplaySpoolStats {
        active_segments: u32::try_from(index.segments.len()).unwrap_or(u32::MAX),
        active_bytes: index.active_bytes,
        recovered_segments: index.recovered_segments,
        dropped_awaiting_keyframe: index.dropped_awaiting_keyframe,
        ..ReplaySpoolStats::default()
    }
}

fn pin_snapshot(
    current: &mut Option<SegmentWriter>,
    directory: &Path,
    limits: SpoolLimits,
    index: &Arc<Mutex<SpoolIndex>>,
    requested: ReplayDuration,
) -> Result<Vec<(ReplaySpoolSegment, File)>, ReplaySpoolError> {
    if let Some(writer) = current.take() {
        commit_segment(directory, writer, limits, index)?;
    }
    let plan = plan_snapshot(&lock_index(index), requested)?;
    plan.segments
        .into_iter()
        .map(|segment| {
            let file = open_snapshot_segment(&segment)?;
            Ok((segment, file))
        })
        .collect()
}

fn record_buffered_packet(shared: &SharedStats, packet: &EncodedReplayPacket) {
    shared.accepted_packets.fetch_add(1, Ordering::Relaxed);
    let timestamp_ns = packet.timestamp_ns();
    let _ = shared.first_packet_timestamp_ns.compare_exchange(
        u64::MAX,
        timestamp_ns,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    shared.latest_packet_end_ns.fetch_max(
        timestamp_ns.saturating_add(packet.duration_ns()),
        Ordering::Release,
    );
}

fn reset_buffered_duration(shared: &SharedStats) {
    shared
        .first_packet_timestamp_ns
        .store(u64::MAX, Ordering::Release);
    shared.latest_packet_end_ns.store(0, Ordering::Release);
    shared.accepted_packets.store(0, Ordering::Release);
}

fn buffered_duration_ns(shared: &SharedStats) -> u64 {
    let first = shared.first_packet_timestamp_ns.load(Ordering::Acquire);
    if first == u64::MAX {
        return 0;
    }
    shared
        .latest_packet_end_ns
        .load(Ordering::Acquire)
        .saturating_sub(first)
}

fn process_packet(
    directory: &Path,
    packet: &EncodedReplayPacket,
    current: &mut Option<SegmentWriter>,
    next_sequence: &mut u64,
    limits: SpoolLimits,
    index: &Arc<Mutex<SpoolIndex>>,
) -> Result<(), ReplaySpoolError> {
    let packet_bytes = u64::try_from(packet.bytes().len()).unwrap_or(u64::MAX);
    let record_bytes = 8_u64 + 8 + 1 + 4 + packet_bytes;
    let should_rotate = current.as_ref().is_some_and(|writer| {
        packet.is_keyframe()
            && (packet
                .timestamp_ns()
                .saturating_sub(writer.start_timestamp_ns)
                >= TARGET_SEGMENT_NS
                || writer.bytes.saturating_add(record_bytes) > MAX_SEGMENT_BYTES)
    });
    if should_rotate {
        let writer = current.take().expect("rotation requires a current segment");
        commit_segment(directory, writer, limits, index)?;
    }
    if current.is_none() {
        if !packet.is_keyframe() {
            let mut index = lock_index(index);
            index.dropped_awaiting_keyframe = index.dropped_awaiting_keyframe.saturating_add(1);
            return Ok(());
        }
        *current = Some(create_segment(
            directory,
            *next_sequence,
            packet.timestamp_ns(),
        )?);
        *next_sequence = next_sequence.saturating_add(1);
    }
    let writer = current.as_mut().expect("segment was created");
    let elapsed = packet
        .timestamp_ns()
        .saturating_add(packet.duration_ns())
        .saturating_sub(writer.start_timestamp_ns);
    if elapsed > MAX_SEGMENT_NS || writer.bytes.saturating_add(record_bytes) > MAX_SEGMENT_BYTES {
        return Err(ReplaySpoolError::new(
            "encoder did not provide a keyframe within the segment bound",
        ));
    }
    writer
        .file
        .write_all(&packet.timestamp_ns().to_le_bytes())
        .and_then(|()| writer.file.write_all(&packet.duration_ns().to_le_bytes()))
        .and_then(|()| writer.file.write_all(&[u8::from(packet.is_keyframe())]))
        .and_then(|()| {
            writer.file.write_all(
                &u32::try_from(packet.bytes().len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            )
        })
        .and_then(|()| writer.file.write_all(packet.bytes()))
        .map_err(|error| io_error("could not write Replay segment", &error))?;
    writer.end_timestamp_ns = packet.timestamp_ns().saturating_add(packet.duration_ns());
    writer.bytes = writer.bytes.saturating_add(record_bytes);
    Ok(())
}

fn clear_completed_segments(
    directory: &Path,
    index: &Arc<Mutex<SpoolIndex>>,
) -> Result<(), ReplaySpoolError> {
    let mut index = lock_index(index);
    let mut removed = false;
    while let Some(segment) = index.segments.pop_front() {
        fs::remove_file(&segment.path)
            .map_err(|error| io_error("could not remove Replay segment from old epoch", &error))?;
        removed = true;
    }
    index.active_bytes = 0;
    index.generation = index.generation.saturating_add(1);
    if removed {
        sync_directory(directory)?;
    }
    Ok(())
}

fn create_segment(
    directory: &Path,
    sequence: u64,
    start_timestamp_ns: u64,
) -> Result<SegmentWriter, ReplaySpoolError> {
    let temporary_path = directory.join(format!(".{SEGMENT_PREFIX}{sequence:016x}{TEMP_SUFFIX}"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary_path)
        // A failed rotation can abandon this exact temporary name. Remove the
        // leftover once and retry so a transient disk error cannot turn into
        // a permanent create collision on the next keyframe.
        .or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
            let _ = fs::remove_file(&temporary_path);
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary_path)
        })
        .map_err(|error| io_error("could not create Replay segment", &error))?;
    file.write_all(SEGMENT_MAGIC)
        .map_err(|error| io_error("could not write Replay segment header", &error))?;
    Ok(SegmentWriter {
        temporary_path,
        file,
        start_timestamp_ns,
        end_timestamp_ns: start_timestamp_ns,
        bytes: u64::try_from(SEGMENT_MAGIC.len()).unwrap_or(u64::MAX),
        sequence,
    })
}

fn commit_segment(
    directory: &Path,
    writer: SegmentWriter,
    limits: SpoolLimits,
    index: &Arc<Mutex<SpoolIndex>>,
) -> Result<(), ReplaySpoolError> {
    writer
        .file
        .sync_data()
        .map_err(|error| io_error("could not flush Replay segment", &error))?;
    drop(writer.file);
    let destination = directory.join(format!(
        "{SEGMENT_PREFIX}{:016x}-{:016x}-{:016x}{SEGMENT_SUFFIX}",
        writer.sequence, writer.start_timestamp_ns, writer.end_timestamp_ns
    ));
    if let Err(error) = fs::rename(&writer.temporary_path, &destination) {
        // The partial segment is invalid history for any later save, so the
        // abandoned temporary file is removed here before the error reaches
        // the worker's drop-and-resynchronize path.
        let _ = fs::remove_file(&writer.temporary_path);
        return Err(io_error("could not commit Replay segment", &error));
    }
    sync_directory(directory)?;
    let metadata = fs::symlink_metadata(&destination)
        .map_err(|error| io_error("could not inspect Replay segment", &error))?;
    let mut index = lock_index(index);
    index.active_bytes = index.active_bytes.saturating_add(metadata.len());
    index.generation = index.generation.saturating_add(1);
    index.segments.push_back(ReplaySpoolSegment {
        path: destination,
        start_timestamp_ns: writer.start_timestamp_ns,
        end_timestamp_ns: writer.end_timestamp_ns,
        bytes: metadata.len(),
    });
    enforce_limits(directory, &mut index, limits)
}

fn enforce_limits(
    directory: &Path,
    index: &mut SpoolIndex,
    limits: SpoolLimits,
) -> Result<(), ReplaySpoolError> {
    let mut removed = false;
    while index.segments.len() > MAX_SEGMENTS
        || index.active_bytes > limits.bytes
        || index
            .segments
            .front()
            .zip(index.segments.back())
            .is_some_and(|(oldest, newest)| {
                newest
                    .end_timestamp_ns
                    .saturating_sub(oldest.start_timestamp_ns)
                    > limits.duration_ns
            })
    {
        let segment = index
            .segments
            .pop_front()
            .ok_or_else(|| ReplaySpoolError::new("Replay spool limits are invalid"))?;
        fs::remove_file(&segment.path)
            .map_err(|error| io_error("could not remove expired Replay segment", &error))?;
        index.active_bytes = index.active_bytes.saturating_sub(segment.bytes);
        removed = true;
    }
    if removed {
        sync_directory(directory)?;
    }
    Ok(())
}

fn initialize_spool(directory: &Path) -> Result<(), ReplaySpoolError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(ReplaySpoolError::new("Replay spool is not a directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(directory)
            .map_err(|error| io_error("could not create Replay spool", &error))?,
        Err(error) => return Err(io_error("could not inspect Replay spool", &error)),
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error("could not secure Replay spool", &error))?;
    let marker = directory.join(MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(_) => validate_marker(&marker),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(directory)
                .map_err(|error| io_error("could not inspect Replay spool", &error))?
                .next()
                .transpose()
                .map_err(|error| io_error("could not inspect Replay spool", &error))?
                .is_some()
            {
                return Err(ReplaySpoolError::new(
                    "refusing to claim a non-empty Replay spool",
                ));
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker)
                .map_err(|error| io_error("could not create Replay spool marker", &error))?;
            file.write_all(MARKER_CONTENTS)
                .and_then(|()| file.sync_all())
                .map_err(|error| io_error("could not write Replay spool marker", &error))?;
            sync_directory(directory)
        }
        Err(error) => Err(io_error("could not inspect Replay spool marker", &error)),
    }
}

fn validate_marker(marker: &Path) -> Result<(), ReplaySpoolError> {
    let metadata = fs::symlink_metadata(marker)
        .map_err(|error| io_error("could not inspect Replay spool marker", &error))?;
    if !metadata.file_type().is_file() || metadata.len() != MARKER_CONTENTS.len() as u64 {
        return Err(ReplaySpoolError::new("Replay spool marker is invalid"));
    }
    let mut contents = Vec::new();
    File::open(marker)
        .and_then(|mut file| file.read_to_end(&mut contents))
        .map_err(|error| io_error("could not read Replay spool marker", &error))?;
    if contents != MARKER_CONTENTS {
        return Err(ReplaySpoolError::new("Replay spool marker is invalid"));
    }
    Ok(())
}

fn recover_index(directory: &Path) -> Result<SpoolIndex, ReplaySpoolError> {
    let mut index = SpoolIndex::default();
    let mut entry_count = 0_usize;
    for entry in
        fs::read_dir(directory).map_err(|error| io_error("could not scan Replay spool", &error))?
    {
        let entry = entry.map_err(|error| io_error("could not scan Replay spool", &error))?;
        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_SEGMENTS + 64 {
            return Err(ReplaySpoolError::new(
                "Replay spool contains too many entries",
            ));
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&format!(".{SEGMENT_PREFIX}")) && name.ends_with(TEMP_SUFFIX) {
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| io_error("could not inspect temporary Replay segment", &error))?;
            if metadata.file_type().is_file() {
                fs::remove_file(entry.path()).map_err(|error| {
                    io_error("could not remove abandoned Replay segment", &error)
                })?;
            }
            continue;
        }
        let Some((sequence, start, end)) = parse_segment_name(name) else {
            continue;
        };
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| io_error("could not inspect Replay segment", &error))?;
        if !metadata.file_type().is_file() || start >= end || metadata.len() > MAX_SEGMENT_BYTES {
            return Err(ReplaySpoolError::new("owned Replay segment is invalid"));
        }
        let mut magic = [0_u8; 8];
        File::open(entry.path())
            .and_then(|mut file| file.read_exact(&mut magic))
            .map_err(|error| io_error("could not read Replay segment", &error))?;
        if &magic != SEGMENT_MAGIC {
            return Err(ReplaySpoolError::new(
                "owned Replay segment header is invalid",
            ));
        }
        index.active_bytes = index.active_bytes.saturating_add(metadata.len());
        index.generation = index.generation.max(sequence);
        index.recovered_segments = index.recovered_segments.saturating_add(1);
        index.segments.push_back(ReplaySpoolSegment {
            path: entry.path(),
            start_timestamp_ns: start,
            end_timestamp_ns: end,
            bytes: metadata.len(),
        });
    }
    index
        .segments
        .make_contiguous()
        .sort_by_key(|segment| segment.start_timestamp_ns);
    Ok(index)
}

fn parse_segment_name(name: &str) -> Option<(u64, u64, u64)> {
    let body = name
        .strip_prefix(SEGMENT_PREFIX)?
        .strip_suffix(SEGMENT_SUFFIX)?;
    let mut parts = body.split('-');
    let sequence = u64::from_str_radix(parts.next()?, 16).ok()?;
    let start = u64::from_str_radix(parts.next()?, 16).ok()?;
    let end = u64::from_str_radix(parts.next()?, 16).ok()?;
    (parts.next().is_none()).then_some((sequence, start, end))
}

fn lock_index(index: &Mutex<SpoolIndex>) -> MutexGuard<'_, SpoolIndex> {
    index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn sync_directory(directory: &Path) -> Result<(), ReplaySpoolError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("could not flush Replay spool directory", &error))
}

fn io_error(context: &str, error: &io::Error) -> ReplaySpoolError {
    ReplaySpoolError::new(format!(
        "{context}: kind={:?} errno={:?} detail={error}",
        error.kind(),
        error.raw_os_error()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReplayBudget, ReplayPacketFormat, ReplayVideoCodec};
    use redunar_core::{ReplayFrameRate, ReplayStorageLimit};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn save_request_and_cancellation_do_not_wait_for_the_spool_index() {
        let root = fixture();
        let configured = settings(ReplayDuration::Seconds30);
        let mut spool = ReplaySegmentSpool::open(root.join("spool"), configured).unwrap();
        let store =
            ReplayClipStore::open(root.join("clips"), ReplayBudget::from_settings(configured))
                .unwrap();
        let guard = lock_index(&spool.index);
        let started = Instant::now();
        assert_eq!(spool.stats().active_segments, 0);
        let job = spool
            .save_async(
                ReplayDuration::Seconds15,
                configured,
                stream(),
                store.clone(),
            )
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(200));
        job.cancellation().store(true, Ordering::Release);
        assert!(job.join().is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(guard);
        spool.shutdown();
        assert!(store.inventory().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture() -> PathBuf {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("redunar-replay-spool-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn settings(duration: ReplayDuration) -> ReplaySettings {
        ReplaySettings {
            duration,
            frame_rate: ReplayFrameRate::Fps60,
            quality: ReplayQuality::Efficient,
            storage_limit: ReplayStorageLimit::GiB5,
        }
    }

    fn packet(timestamp_ns: u64, keyframe: bool) -> EncodedReplayPacket {
        let nal = if keyframe { 0x65 } else { 0x41 };
        EncodedReplayPacket::new(
            timestamp_ns,
            1_000_000_000,
            keyframe,
            vec![0, 0, 0, 2, nal, 0x88],
        )
        .expect("packet")
    }

    fn short_packet(timestamp_ns: u64, keyframe: bool) -> EncodedReplayPacket {
        let nal = if keyframe { 0x65 } else { 0x41 };
        EncodedReplayPacket::new(
            timestamp_ns,
            16_666_667,
            keyframe,
            vec![0, 0, 0, 2, nal, 0x88],
        )
        .expect("short packet")
    }

    fn target_bitrate_packet(timestamp_ns: u64, keyframe: bool) -> EncodedReplayPacket {
        let packet_bytes = usize::try_from(
            u64::from(ReplayQuality::Efficient.target_megabits_per_second()) * BYTES_PER_MEGABIT,
        )
        .expect("one second target payload fits usize");
        packet_with_payload_bytes(timestamp_ns, keyframe, packet_bytes)
    }

    fn packet_with_payload_bytes(
        timestamp_ns: u64,
        keyframe: bool,
        packet_bytes: usize,
    ) -> EncodedReplayPacket {
        let mut payload = vec![0_u8; packet_bytes];
        payload[..4].copy_from_slice(
            &u32::try_from(packet_bytes - 4)
                .expect("bounded payload")
                .to_be_bytes(),
        );
        payload[4] = if keyframe { 0x65 } else { 0x41 };
        EncodedReplayPacket::new(timestamp_ns, NANOSECONDS_PER_SECOND, keyframe, payload)
            .expect("target bitrate packet")
    }

    fn stream() -> ReplayVideoStream {
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
        .expect("stream")
    }

    fn wait_for_segments(spool: &ReplaySegmentSpool, count: u32) {
        for _ in 0..100 {
            if spool.stats().active_segments >= count {
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("Replay spool did not commit {count} segments");
    }

    #[test]
    fn async_spool_keeps_present_path_queue_and_disk_history_bounded() {
        let root = fixture();
        let mut spool = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds15))
            .expect("open spool");
        for second in 0..40_u64 {
            spool
                .try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0))
                .expect("queue no-loss history fixture packet");
            // This history assertion requires no queue-full discontinuity.
            // Wait for worker progress instead of assuming a 1 ms disk write.
            let deadline = Instant::now() + Duration::from_secs(5);
            while spool.stats().accepted_packets < second + 1 {
                assert!(Instant::now() < deadline, "spool fixture worker stalled");
                thread::sleep(Duration::from_millis(1));
            }
        }
        wait_for_segments(&spool, 1);
        spool.shutdown();
        let stats = spool.stats();
        assert!(stats.active_segments <= u32::try_from(MAX_SEGMENTS).unwrap_or(u32::MAX));
        assert!(
            stats.active_bytes
                <= SpoolLimits::new(
                    RETAINED_DURATION,
                    ReplayQuality::Efficient,
                    ReplayFrameRate::Fps60
                )
                .bytes
        );
        assert_eq!(stats.buffered_duration_ns, 40 * NANOSECONDS_PER_SECOND);
        assert_eq!(stats.accepted_packets, 40);
        assert_eq!(stats.dropped_queue_full, 0);
        assert_eq!(COMMAND_CAPACITY, 64);
        assert_eq!(MAX_SEGMENT_BYTES, 32 * 1024 * 1024);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn burst_submission_does_not_drop_a_short_encoded_history() {
        let root = fixture();
        let mut spool = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds15))
            .expect("open spool");
        let packet_count = 32_u64;
        let packet_bytes = usize::try_from(
            u64::from(ReplayQuality::Efficient.target_megabits_per_second()) * BYTES_PER_MEGABIT,
        )
        .expect("bounded packet size");

        for index in 0..packet_count {
            spool
                .try_submit(packet_with_payload_bytes(
                    index * NANOSECONDS_PER_SECOND,
                    index % 2 == 0,
                    packet_bytes,
                ))
                .expect("burst should fit the bounded spool queue");
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        while spool.stats().accepted_packets < packet_count {
            assert!(Instant::now() < deadline, "spool fixture worker stalled");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(spool.stats().dropped_queue_full, 0);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fifteen_minute_spool_budget_is_disk_bounded_without_growing_the_queue() {
        let efficient = SpoolLimits::new(
            ReplayDuration::Seconds900,
            ReplayQuality::Efficient,
            ReplayFrameRate::Fps60,
        );
        let balanced = SpoolLimits::new(
            ReplayDuration::Seconds900,
            ReplayQuality::Balanced,
            ReplayFrameRate::Fps60,
        );
        let high = SpoolLimits::new(
            ReplayDuration::Seconds900,
            ReplayQuality::High,
            ReplayFrameRate::Fps60,
        );
        assert_eq!(efficient.bytes, 1_485_000_000);
        assert_eq!(balanced.bytes, 2_970_000_000);
        assert_eq!(high.bytes, 4_950_000_000);
        assert_eq!(COMMAND_CAPACITY, 64);
        assert_eq!(MAX_SEGMENTS, 512);
        assert_eq!(MAX_SEGMENT_BYTES, 32 * 1024 * 1024);
    }

    #[test]
    fn keyframe_safe_snapshot_can_request_any_retained_duration() {
        let root = fixture();
        let mut spool = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds30))
            .expect("open spool");
        for second in 0..12_u64 {
            loop {
                match spool.try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0)) {
                    Ok(()) => {
                        thread::sleep(Duration::from_millis(1));
                        break;
                    }
                    Err(ReplaySpoolSubmitError::QueueFull(_)) => thread::yield_now(),
                    Err(error) => panic!("submit failed: {error}"),
                }
            }
        }
        wait_for_segments(&spool, 2);
        let plan = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("snapshot plan");
        assert!(!plan.segments.is_empty());
        assert!(
            plan.segments
                .iter()
                .all(|segment| segment.start_timestamp_ns % 2_000_000_000 == 0)
        );
        let early_long_clip = spool
            .snapshot_plan(ReplayDuration::Seconds900)
            .expect("save available history before the full duration exists");
        assert!(
            early_long_clip.available_duration_ns
                < u64::from(ReplayDuration::Seconds900.seconds()) * NANOSECONDS_PER_SECOND
        );
        assert!(!early_long_clip.segments.is_empty());
        let before_repeat = spool.stats();
        let repeated = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("repeated save must not consume history");
        assert!(!repeated.segments.is_empty());
        assert_eq!(spool.stats(), before_repeat);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asynchronous_save_keeps_spooling_and_does_not_consume_history() {
        let root = fixture();
        let spool_root = root.join("spool");
        let clip_root = root.join("clips");
        let configured = settings(ReplayDuration::Seconds30);
        let mut spool = ReplaySegmentSpool::open(&spool_root, configured).expect("open spool");
        for second in 0..12_u64 {
            loop {
                match spool.try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0)) {
                    Ok(()) => {
                        thread::sleep(Duration::from_millis(1));
                        break;
                    }
                    Err(ReplaySpoolSubmitError::QueueFull(_)) => thread::yield_now(),
                    Err(error) => panic!("submit failed: {error}"),
                }
            }
        }
        wait_for_segments(&spool, 2);
        let store = ReplayClipStore::open(&clip_root, ReplayBudget::from_settings(configured))
            .expect("open store");
        let before = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("history");
        let first = spool
            .save_async(
                ReplayDuration::Seconds15,
                configured,
                stream(),
                store.clone(),
            )
            .expect("start save");

        for second in 12..16_u64 {
            loop {
                match spool.try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0)) {
                    Ok(()) => break,
                    Err(ReplaySpoolSubmitError::QueueFull(_)) => thread::yield_now(),
                    Err(error) => panic!("submit during save failed: {error}"),
                }
            }
        }
        let first_clip = first.join().expect("assemble first clip");
        assert!(first_clip.bytes > 4);
        let second_clip = spool
            .save_async(
                ReplayDuration::Seconds15,
                configured,
                stream(),
                store.clone(),
            )
            .expect("start repeated save")
            .join()
            .expect("assemble repeated clip");
        assert_ne!(first_clip.path, second_clip.path);
        assert_eq!(store.inventory().expect("inventory").len(), 2);
        let after = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("history remains");
        assert!(after.generation >= before.generation);
        assert!(!after.segments.is_empty());
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn full_target_bitrate_clip_does_not_double_count_container_headroom() {
        let root = fixture();
        let spool_root = root.join("spool");
        let clip_root = root.join("clips");
        let configured = settings(ReplayDuration::Seconds15);
        let mut spool = ReplaySegmentSpool::open(&spool_root, configured).expect("open spool");
        for second in 0..15_u64 {
            let packet = target_bitrate_packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0);
            spool
                .try_submit(packet)
                .expect("queue budget fixture packet");
            // Drain each keyframe pair so this budget test cannot trigger the
            // queue-full discontinuity policy on a slower storage worker.
            if second % 2 == 1 {
                spool
                    .seal_active_tail()
                    .expect("commit budget fixture pair");
            }
        }
        let store = ReplayClipStore::open(&clip_root, ReplayBudget::from_settings(configured))
            .expect("open store");
        let clip = spool
            .save_async(ReplayDuration::Seconds15, configured, stream(), store)
            .expect("start full target bitrate save")
            .join()
            .expect("full target bitrate history fits its single configured headroom");
        assert!(clip.bytes > 5 * 1024 * 1024);
        assert_eq!(spool.stats().dropped_queue_full, 0);
        assert_eq!(spool.stats().accepted_packets, 15);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn requested_duration_sets_clip_budget_independently_of_legacy_profile_duration() {
        let root = fixture();
        let spool_root = root.join("spool");
        let clip_root = root.join("clips");
        let configured = settings(ReplayDuration::Seconds15);
        let mut spool = ReplaySegmentSpool::open(&spool_root, configured).expect("open spool");
        for second in 0..30_u64 {
            let packet = packet_with_payload_bytes(
                second * NANOSECONDS_PER_SECOND,
                second % 2 == 0,
                2_500_000,
            );
            spool
                .try_submit(packet)
                .expect("queue budget fixture packet");
            // Keep the full history deterministic: retrying QueueFull also
            // signals a discontinuity and can discard the active segment.
            if second % 2 == 1 {
                spool
                    .seal_active_tail()
                    .expect("commit budget fixture pair");
            }
        }
        let legacy_budget = ReplayBudget::from_settings(configured).maximum_ring_bytes;
        let store = ReplayClipStore::open(&clip_root, ReplayBudget::from_settings(configured))
            .expect("open store");
        let clip = spool
            .save_async(ReplayDuration::Seconds30, configured, stream(), store)
            .expect("start save longer than legacy profile duration")
            .join()
            .expect("requested duration supplies the clip byte budget");

        assert!(clip.bytes > legacy_budget);
        assert_eq!(spool.stats().dropped_queue_full, 0);
        assert_eq!(spool.stats().accepted_packets, 30);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asynchronous_save_seals_and_includes_the_active_tail() {
        let root = fixture();
        let spool_root = root.join("spool");
        let clip_root = root.join("clips");
        let configured = settings(ReplayDuration::Seconds30);
        let mut spool = ReplaySegmentSpool::open(&spool_root, configured).expect("open spool");

        // This is shorter than the normal two-second rotation interval, so no
        // completed segment exists before the save request.
        spool.try_submit(packet(0, true)).expect("queue keyframe");
        spool
            .try_submit(packet(NANOSECONDS_PER_SECOND, false))
            .expect("queue tail frame");
        assert_eq!(spool.stats().active_segments, 0);

        let store = ReplayClipStore::open(&clip_root, ReplayBudget::from_settings(configured))
            .expect("open store");
        let clip = spool
            .save_async(
                ReplayDuration::Seconds15,
                configured,
                stream(),
                store.clone(),
            )
            .expect("active tail snapshot starts")
            .join()
            .expect("active tail assembles");

        assert!(clip.bytes > 4);
        assert_eq!(store.inventory().expect("inventory").len(), 1);
        let plan = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("sealed tail is retained");
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].start_timestamp_ns, 0);
        assert_eq!(
            plan.segments[0].end_timestamp_ns,
            2 * NANOSECONDS_PER_SECOND
        );

        // A new segment still observes the keyframe-safe start contract.
        spool
            .try_submit(packet(2 * NANOSECONDS_PER_SECOND, false))
            .expect("queue non-keyframe after snapshot");
        spool
            .try_submit(packet(3 * NANOSECONDS_PER_SECOND, true))
            .expect("queue keyframe after snapshot");
        spool
            .try_submit(packet(5 * NANOSECONDS_PER_SECOND, true))
            .expect("queue rotation keyframe");
        wait_for_segments(&spool, 2);

        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sub_second_history_is_not_committed_as_a_replay_clip() {
        let root = fixture();
        let configured = settings(ReplayDuration::Seconds30);
        let mut spool =
            ReplaySegmentSpool::open(root.join("spool"), configured).expect("open spool");
        spool
            .try_submit(short_packet(0, true))
            .expect("queue keyframe");
        spool
            .try_submit(short_packet(81_000_000, false))
            .expect("queue second frame");
        spool
            .try_submit(short_packet(147_000_000, false))
            .expect("queue third frame");

        let store =
            ReplayClipStore::open(root.join("clips"), ReplayBudget::from_settings(configured))
                .expect("open store");
        let result = spool
            .save_async(
                ReplayDuration::Seconds15,
                configured,
                stream(),
                store.clone(),
            )
            .expect("snapshot request accepted")
            .join();
        let error = result.expect_err("sub-second history must not commit a clip save");

        assert!(error.is_insufficient_history());
        assert!(error.to_string().contains("just started"));
        assert!(store.inventory().expect("inventory").is_empty());
        assert_eq!(spool.phase(), ReplaySpoolPhase::Running);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reset_epoch_discards_completed_history_before_accepting_new_packets() {
        let root = fixture();
        let mut spool = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds30))
            .expect("open spool");
        for second in 0..5_u64 {
            loop {
                match spool.try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0)) {
                    Ok(()) => break,
                    Err(ReplaySpoolSubmitError::QueueFull(_)) => thread::yield_now(),
                    Err(error) => panic!("submit failed: {error}"),
                }
            }
        }
        wait_for_segments(&spool, 1);

        spool.reset_epoch().expect("queue epoch reset");
        for _ in 0..100 {
            if spool.stats().active_segments == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(spool.stats().active_segments, 0);
        assert!(spool.snapshot_plan(ReplayDuration::Seconds15).is_err());

        // Timestamps may restart after a codec epoch reset. A non-keyframe
        // remains ineligible until a new independent segment starts.
        spool
            .try_submit(packet(0, false))
            .expect("queue first new-epoch packet");
        spool
            .try_submit(packet(NANOSECONDS_PER_SECOND, true))
            .expect("queue first new-epoch keyframe");
        spool
            .try_submit(packet(3 * NANOSECONDS_PER_SECOND, true))
            .expect("queue rotation keyframe");
        wait_for_segments(&spool, 1);
        let plan = spool
            .snapshot_plan(ReplayDuration::Seconds15)
            .expect("new epoch history");
        assert_eq!(plan.segments[0].start_timestamp_ns, NANOSECONDS_PER_SECOND);
        spool.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_removes_partial_files_and_retains_only_exact_owned_segments() {
        let root = fixture();
        {
            let mut spool = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds30))
                .expect("open spool");
            for second in 0..5_u64 {
                loop {
                    if spool
                        .try_submit(packet(second * NANOSECONDS_PER_SECOND, second % 2 == 0))
                        .is_ok()
                    {
                        break;
                    }
                    thread::yield_now();
                }
            }
            wait_for_segments(&spool, 1);
            spool.shutdown();
        }
        fs::write(root.join(".redunar-segment-dead.tmp"), b"partial")
            .expect("write abandoned segment");
        fs::write(root.join("user-file"), b"keep").expect("write unknown file");
        let mut recovered = ReplaySegmentSpool::open(&root, settings(ReplayDuration::Seconds30))
            .expect("recover spool");
        assert!(recovered.stats().recovered_segments >= 1);
        assert!(!root.join(".redunar-segment-dead.tmp").exists());
        assert_eq!(
            fs::read(root.join("user-file")).expect("unknown retained"),
            b"keep"
        );
        recovered.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
