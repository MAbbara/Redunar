use crate::replay_encoder::MAX_HARDWARE_INPUTS_IN_FLIGHT;
use crate::{
    DmaBufReplayFrame, HardwareEncoderApi, HardwareEncoderBackend, ReplayAssemblyJob,
    ReplayAudioBuffer, ReplayAudioError, ReplayAudioSnapshot, ReplayAudioTracks, ReplayClipStore,
    ReplayEncoderError, ReplayOutputFormat, ReplayPacketFlow, ReplayPacketFlowPhase,
    ReplayRingStats, ReplaySegmentSpool, ReplaySpoolError, ReplaySpoolPhase, ReplaySpoolStats,
    ReplaySpoolSubmitError, ReplayStoreError, ReplayVideoStream, StoredReplayClip,
};
use redunar_core::{ReplayDuration, ReplayFrameRate, ReplaySettings};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

const MAX_IN_MEMORY_HISTORY: ReplayDuration = ReplayDuration::Seconds120;
const MAX_TRACKED_INPUTS: usize = MAX_HARDWARE_INPUTS_IN_FLIGHT;
const MAX_COMPLETED_INPUTS: usize = 8;
const KEYFRAME_REQUEST_INTERVAL_NS: u64 = 2_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayPipelinePhase {
    Buffering,
    Saving,
    Resetting,
    Failed,
    Shutdown,
}

#[derive(Debug)]
pub enum ReplayPipelineError {
    InvalidLifecycle,
    BackendStreamMismatch,
    FrameFormatChanged,
    FrameTimestampNotMonotonic,
    NoEncodedFrames,
    InsufficientHistory,
    Encoder(ReplayEncoderError),
    Spool(ReplaySpoolError),
    Store(ReplayStoreError),
    Audio(ReplayAudioError),
}

impl fmt::Display for ReplayPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLifecycle => formatter.write_str("replay pipeline lifecycle is invalid"),
            Self::BackendStreamMismatch => {
                formatter.write_str("hardware encoder stream does not match replay settings")
            }
            Self::FrameFormatChanged => formatter.write_str(
                "exported DMA-BUF dimensions changed before the replay pipeline was reset",
            ),
            Self::FrameTimestampNotMonotonic => {
                formatter.write_str("exported DMA-BUF timestamp is not monotonic")
            }
            Self::NoEncodedFrames => {
                formatter.write_str("the rolling replay buffer has no decodable encoded frames")
            }
            Self::InsufficientHistory => formatter.write_str(
                "Instant Replay is still buffering; wait at least one second before saving",
            ),
            Self::Encoder(error) => write!(formatter, "replay encoder pipeline failed: {error}"),
            Self::Spool(error) => write!(formatter, "replay disk history failed: {error}"),
            Self::Store(error) => write!(formatter, "replay clip save failed: {error}"),
            Self::Audio(error) => write!(formatter, "replay audio failed: {error}"),
        }
    }
}

impl Error for ReplayPipelineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encoder(error) => Some(error),
            Self::Spool(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Audio(error) => Some(error),
            _ => None,
        }
    }
}

/// Daemon-owned hardware replay pipeline.
///
/// This type connects the GPU-exported DMA-BUF boundary to exactly one
/// hardware encoder, the bounded encoded ring, and the private atomic clip
/// store. It has no host-pixel input and no software-encoder fallback.
///
/// Construction is an integration boundary, not production-readiness proof.
/// Normal [`crate::RedunarService`] status remains unavailable until a real
/// backend passes the separate end-to-end hardware and performance gates.
pub struct ReplayHardwarePipeline {
    settings: ReplaySettings,
    backend: Option<Box<dyn HardwareEncoderBackend>>,
    shutdown_error: Option<ReplayEncoderError>,
    flow: ReplayPacketFlow,
    store: ReplayClipStore,
    spool: Option<ReplaySegmentSpool>,
    phase: ReplayPipelinePhase,
    last_frame_timestamp_ns: Option<u64>,
    last_keyframe_request_timestamp_ns: Option<u64>,
    in_flight_inputs: VecDeque<u64>,
    completed_inputs: VecDeque<u64>,
    audio: ReplayAudioBuffer,
}

impl ReplayHardwarePipeline {
    /// Connect one already-created hardware backend to bounded daemon state.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] when the backend output cadence does
    /// not match the selected replay settings. The rejected backend is shut
    /// down before returning.
    pub fn new(
        settings: ReplaySettings,
        store: ReplayClipStore,
        mut backend: Box<dyn HardwareEncoderBackend>,
    ) -> Result<Self, ReplayPipelineError> {
        if !stream_matches_settings(backend.stream(), settings) {
            let _ = backend.shutdown();
            return Err(ReplayPipelineError::BackendStreamMismatch);
        }
        let stream = backend.stream().clone();
        Ok(Self {
            settings,
            backend: Some(backend),
            shutdown_error: None,
            // Long Replay selections are served from the disk spool. Keeping
            // this compatibility window capped prevents a 15-minute shortcut
            // from silently allocating a 15-minute RAM ring.
            flow: ReplayPacketFlow::new(in_memory_settings(settings), stream),
            store,
            spool: None,
            phase: ReplayPipelinePhase::Buffering,
            last_frame_timestamp_ns: None,
            last_keyframe_request_timestamp_ns: None,
            in_flight_inputs: VecDeque::with_capacity(MAX_TRACKED_INPUTS),
            completed_inputs: VecDeque::with_capacity(MAX_COMPLETED_INPUTS),
            audio: ReplayAudioBuffer::new(
                // Video history always retains the full duration-selectable
                // spool. Keep audio on the same horizon so a 15-minute save
                // cannot silently contain only the profile's shorter tail.
                ReplayDuration::Seconds900,
                redunar_capture_audio::OpusStreamDescription::default(),
            ),
        })
    }

    /// Add one encoded game-audio packet to the same Replay timeline.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] when the pipeline is not buffering or
    /// the bounded audio timeline rejects the packet.
    pub fn submit_audio(
        &mut self,
        packet: redunar_capture_audio::EncodedOpusPacket,
    ) -> Result<(), ReplayPipelineError> {
        if self.phase != ReplayPipelinePhase::Buffering {
            return Err(ReplayPipelineError::InvalidLifecycle);
        }
        self.audio.push(packet).map_err(ReplayPipelineError::Audio)
    }

    /// Connect the hardware packet path to the fixed-memory disk spool.
    ///
    /// A saturated spool drops its current segment until the next keyframe;
    /// the encoder/ring path never blocks on disk. A failed spool is terminal
    /// because continuing would advertise history that can no longer be saved.
    #[must_use]
    pub fn with_spool(mut self, spool: ReplaySegmentSpool) -> Self {
        self.spool = Some(spool);
        self
    }

    #[must_use]
    pub const fn phase(&self) -> ReplayPipelinePhase {
        self.phase
    }

    #[must_use]
    pub fn audio_stats(&self) -> (u64, u64) {
        (self.audio.packet_count(), self.audio.byte_count())
    }

    #[must_use]
    pub fn encoder_api(&self) -> Option<HardwareEncoderApi> {
        self.backend.as_ref().map(|backend| backend.api())
    }

    #[must_use]
    pub fn stream(&self) -> &ReplayVideoStream {
        self.flow.stream()
    }

    #[must_use]
    pub fn ring_stats(&self) -> ReplayRingStats {
        self.flow.ring().stats()
    }

    #[must_use]
    pub const fn has_spool(&self) -> bool {
        self.spool.is_some()
    }

    #[must_use]
    pub fn spool_stats(&self) -> Option<ReplaySpoolStats> {
        self.spool.as_ref().map(ReplaySegmentSpool::stats)
    }

    pub(crate) fn in_flight_count(&self) -> usize {
        self.in_flight_inputs.len()
    }

    pub(crate) fn teardown_incomplete(&self) -> bool {
        self.shutdown_error.is_some()
    }

    /// Submit one GPU-exported frame to the hardware-only encoder.
    ///
    /// A keyframe is requested for the first frame and at least every two
    /// seconds thereafter. Dimension or timestamp changes without an explicit
    /// reset fail closed and release the encoder.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] for an invalid phase, changed export
    /// shape, non-monotonic presentation time, backend error, malformed output,
    /// or ring-bound violation.
    pub fn submit_frame(
        &mut self,
        input_sequence: u64,
        frame: DmaBufReplayFrame,
    ) -> Result<(), ReplayPipelineError> {
        if self.phase != ReplayPipelinePhase::Buffering {
            return Err(ReplayPipelineError::InvalidLifecycle);
        }
        if input_sequence == 0 || self.in_flight_inputs.contains(&input_sequence) {
            self.fail_closed();
            return Err(ReplayPipelineError::Encoder(
                ReplayEncoderError::InvalidInputCompletion,
            ));
        }
        // Track even an input rejected before encode so fail_closed can
        // return it after safe teardown, together with earlier GPU reads.
        self.in_flight_inputs.push_back(input_sequence);
        if frame.width != self.flow.stream().width() || frame.height != self.flow.stream().height()
        {
            self.fail_closed();
            return Err(ReplayPipelineError::FrameFormatChanged);
        }
        if self
            .last_frame_timestamp_ns
            .is_some_and(|timestamp| frame.timestamp_ns <= timestamp)
        {
            self.fail_closed();
            return Err(ReplayPipelineError::FrameTimestampNotMonotonic);
        }
        let timestamp_ns = frame.timestamp_ns;
        // Presentation time is authoritative. The capture path may
        // intentionally subsample or drop work under load, so a submitted
        // frame count can postpone the next IDR beyond the spool bound.
        let force_keyframe = self
            .last_keyframe_request_timestamp_ns
            .is_none_or(|last| timestamp_ns.saturating_sub(last) >= KEYFRAME_REQUEST_INTERVAL_NS);
        let encoded = match self
            .backend
            .as_mut()
            .ok_or(ReplayPipelineError::InvalidLifecycle)?
            .encode(input_sequence, frame, force_keyframe)
        {
            Ok(encoded) => encoded,
            Err(error) => {
                self.fail_closed();
                return Err(ReplayPipelineError::Encoder(error));
            }
        };
        if let Err(error) = self.accept_encoded(encoded) {
            self.fail_closed();
            return Err(ReplayPipelineError::Encoder(error));
        }
        self.last_frame_timestamp_ns = Some(timestamp_ns);
        if force_keyframe {
            self.last_keyframe_request_timestamp_ns = Some(timestamp_ns);
        }
        if self.in_flight_inputs.len() > MAX_TRACKED_INPUTS {
            self.fail_closed();
            return Err(ReplayPipelineError::Encoder(
                ReplayEncoderError::InvalidInputCompletion,
            ));
        }
        Ok(())
    }

    /// Drain submitted hardware output and durably save the current bounded
    /// window without consuming it.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] when no decodable frame is buffered or
    /// when draining, muxing, atomic commit, or quota enforcement fails. A
    /// backend or storage failure is terminal and releases encoder resources.
    pub fn save_replay(&mut self) -> Result<StoredReplayClip, ReplayPipelineError> {
        if self.phase != ReplayPipelinePhase::Buffering {
            return Err(ReplayPipelineError::InvalidLifecycle);
        }
        let drained = match self
            .backend
            .as_mut()
            .ok_or(ReplayPipelineError::InvalidLifecycle)?
            .drain()
        {
            Ok(drained) => drained,
            Err(error) => {
                self.fail_closed();
                return Err(ReplayPipelineError::Encoder(error));
            }
        };
        if let Err(error) = self.accept_encoded(drained) {
            self.fail_closed();
            return Err(ReplayPipelineError::Encoder(error));
        }
        if self.flow.ring().stats().stored_packets == 0 {
            return Err(ReplayPipelineError::NoEncodedFrames);
        }

        self.phase = ReplayPipelinePhase::Saving;
        let audio = self.audio_snapshot();
        match self.store.save_with_audio_as(
            self.flow.stream(),
            self.flow.ring(),
            &audio,
            ReplayOutputFormat::Matroska,
        ) {
            Ok(clip) => {
                self.phase = ReplayPipelinePhase::Buffering;
                Ok(clip)
            }
            Err(error) => {
                self.fail_closed();
                Err(ReplayPipelineError::Store(error))
            }
        }
    }

    /// Drain current encoder output and assemble a duration-selected disk
    /// history snapshot on an isolated worker while recording continues.
    ///
    /// The spool first seals every packet accepted before the save request, so
    /// the active tail participates as an atomically completed segment. New
    /// packets continue into a fresh keyframe-safe segment.
    ///
    /// # Errors
    ///
    /// Returns an invalid lifecycle, encoder drain, packet validation, spool
    /// snapshot, or assembler-start failure. Assembly/storage errors are
    /// reported later by [`ReplayAssemblyJob::join`].
    pub fn save_spooled_replay(
        &mut self,
        duration: ReplayDuration,
    ) -> Result<ReplayAssemblyJob, ReplayPipelineError> {
        self.save_spooled_replay_as(duration, ReplayOutputFormat::Matroska)
    }

    /// Save the duration-selected spool suffix in the requested container.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid lifecycle, encoder drain, spool snapshot,
    /// or assembler startup failure.
    pub fn save_spooled_replay_as(
        &mut self,
        duration: ReplayDuration,
        output_format: ReplayOutputFormat,
    ) -> Result<ReplayAssemblyJob, ReplayPipelineError> {
        let audio = self.audio.snapshot_between(0, u64::MAX);
        self.save_spooled_replay_as_with_audio(duration, output_format, || audio)
    }

    /// Production audio lives behind its own short-held lock. Read it after
    /// draining video so a slow encoder cannot truncate the clip's audio tail.
    pub(crate) fn save_spooled_replay_as_with_audio(
        &mut self,
        duration: ReplayDuration,
        output_format: ReplayOutputFormat,
        audio_snapshot: impl FnOnce() -> ReplayAudioSnapshot + Send + 'static,
    ) -> Result<ReplayAssemblyJob, ReplayPipelineError> {
        self.save_spooled_replay_as_with_audio_tracks(duration, output_format, move |_, _| {
            ReplayAudioTracks {
                system: audio_snapshot(),
                microphone: None,
            }
        })
    }

    pub(crate) fn save_spooled_replay_as_with_audio_tracks(
        &mut self,
        duration: ReplayDuration,
        output_format: ReplayOutputFormat,
        audio_snapshot: impl FnOnce(u64, u64) -> ReplayAudioTracks + Send + 'static,
    ) -> Result<ReplayAssemblyJob, ReplayPipelineError> {
        if self.phase != ReplayPipelinePhase::Buffering {
            return Err(ReplayPipelineError::InvalidLifecycle);
        }
        let drained = match self
            .backend
            .as_mut()
            .ok_or(ReplayPipelineError::InvalidLifecycle)?
            .drain()
        {
            Ok(drained) => drained,
            Err(error) => {
                self.fail_closed();
                return Err(ReplayPipelineError::Encoder(error));
            }
        };
        if let Err(error) = self.accept_encoded(drained) {
            self.fail_closed();
            return Err(ReplayPipelineError::Encoder(error));
        }
        let spool = self
            .spool
            .as_ref()
            .ok_or(ReplayPipelineError::InvalidLifecycle)?;
        // An accepted snapshot may seal the active segment even if assembler
        // startup fails. The next frame must still resume at a keyframe.
        self.last_keyframe_request_timestamp_ns = None;
        let job = spool
            .save_async_tracks_with_audio(
                duration,
                self.settings,
                self.flow.stream().clone(),
                self.store.clone(),
                output_format,
                audio_snapshot,
            )
            .map_err(|error| {
                if error.is_insufficient_history() {
                    ReplayPipelineError::InsufficientHistory
                } else {
                    ReplayPipelineError::Spool(error)
                }
            })?;
        // Sealing ends the old segment. Force the next submitted frame to be
        // independently decodable so recording resumes without waiting for
        // the normal two-second keyframe cadence.
        self.last_keyframe_request_timestamp_ns = None;
        Ok(job)
    }

    fn audio_snapshot(&self) -> ReplayAudioSnapshot {
        let packets = self.flow.ring().packets().collect::<Vec<_>>();
        let Some(first) = packets.first() else {
            return self.audio.snapshot_between(0, 0);
        };
        let last = packets.last().expect("non-empty Replay packet ring");
        let start = first.timestamp_ns();
        let end = last.timestamp_ns().saturating_add(last.duration_ns());
        self.audio.snapshot_between(start, end)
    }

    /// Replace the encoder after a source resize or hardware reset.
    ///
    /// The entire old codec epoch is discarded before the replacement can
    /// accept a frame. Shutdown failures and invalid replacement streams fail
    /// closed; encoded packets from different dimensions are never combined.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] for an invalid lifecycle, backend
    /// shutdown failure, or replacement stream mismatch.
    pub fn reset(
        &mut self,
        mut replacement: Box<dyn HardwareEncoderBackend>,
    ) -> Result<(), ReplayPipelineError> {
        if self.phase != ReplayPipelinePhase::Buffering {
            let _ = replacement.shutdown();
            return Err(ReplayPipelineError::InvalidLifecycle);
        }
        self.phase = ReplayPipelinePhase::Resetting;
        self.flow.begin_reset();
        if let Some(spool) = self.spool.as_ref()
            && let Err(error) = spool.reset_epoch()
        {
            let _ = replacement.shutdown();
            self.fail_closed();
            return Err(ReplayPipelineError::Spool(error));
        }
        if let Err(error) = self.shutdown_backend() {
            let _ = replacement.shutdown();
            self.flow.fail();
            self.phase = ReplayPipelinePhase::Failed;
            return Err(ReplayPipelineError::Encoder(error));
        }
        if !stream_matches_settings(replacement.stream(), self.settings) {
            let _ = replacement.shutdown();
            self.flow.fail();
            self.phase = ReplayPipelinePhase::Failed;
            return Err(ReplayPipelineError::BackendStreamMismatch);
        }
        let stream = replacement.stream().clone();
        if let Err(error) = self.flow.complete_reset(stream) {
            let _ = replacement.shutdown();
            self.flow.fail();
            self.phase = ReplayPipelinePhase::Failed;
            return Err(ReplayPipelineError::Encoder(error));
        }
        self.backend = Some(replacement);
        self.audio.clear();
        self.last_frame_timestamp_ns = None;
        self.last_keyframe_request_timestamp_ns = None;
        self.phase = ReplayPipelinePhase::Buffering;
        Ok(())
    }
    /// Release the hardware backend and discard the rolling buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPipelineError`] if backend shutdown reports failure.
    pub fn shutdown(&mut self) -> Result<(), ReplayPipelineError> {
        if matches!(self.phase, ReplayPipelinePhase::Shutdown) {
            return self
                .shutdown_error
                .clone()
                .map_or(Ok(()), |error| Err(ReplayPipelineError::Encoder(error)));
        }
        let result = self.shutdown_backend();
        self.flow.shutdown();
        if let Some(mut spool) = self.spool.take() {
            spool.shutdown();
        }
        self.phase = ReplayPipelinePhase::Shutdown;
        result.map_err(ReplayPipelineError::Encoder)
    }

    fn fail_closed(&mut self) {
        let _ = self.shutdown_backend();
        if self.flow.phase() != ReplayPacketFlowPhase::Shutdown {
            self.flow.fail();
        }
        self.phase = ReplayPipelinePhase::Failed;
    }

    fn push_encoded(&mut self, batch: crate::EncodedPacketBatch) -> Result<(), ReplayEncoderError> {
        for packet in batch.packets() {
            self.flow.stream().validate_packet(packet.bytes())?;
        }
        // `EncodedReplayPacket` shares its immutable payload through `Arc`.
        // This bounded metadata copy lets the ring and asynchronous spool own
        // the same encoded packet without copying video bytes.
        let spool_packets = self.spool.as_ref().map(|_| batch.packets().to_vec());
        self.flow.push_batch(batch)?;
        if let (Some(spool), Some(spool_packets)) = (self.spool.as_ref(), spool_packets) {
            for packet in spool_packets {
                match spool.try_submit(packet) {
                    Ok(()) | Err(ReplaySpoolSubmitError::QueueFull(_)) => {}
                    Err(ReplaySpoolSubmitError::NotRunning(_)) => {
                        return Err(ReplayEncoderError::BackendFailed(format!(
                            "Replay spool stopped in phase {:?}",
                            spool.phase()
                        )));
                    }
                }
            }
            if spool.phase() != ReplaySpoolPhase::Running {
                return Err(ReplayEncoderError::BackendFailed(format!(
                    "Replay spool stopped in phase {:?}",
                    spool.phase()
                )));
            }
        }
        Ok(())
    }

    fn accept_encoded(
        &mut self,
        output: crate::HardwareEncodeOutput,
    ) -> Result<(), ReplayEncoderError> {
        let (batch, completed_inputs) = output.into_parts();
        for sequence in completed_inputs {
            let Some(index) = self
                .in_flight_inputs
                .iter()
                .position(|pending| *pending == sequence)
            else {
                return Err(ReplayEncoderError::InvalidInputCompletion);
            };
            self.in_flight_inputs.remove(index);
            if self.completed_inputs.len() >= MAX_COMPLETED_INPUTS {
                return Err(ReplayEncoderError::InvalidInputCompletion);
            }
            self.completed_inputs.push_back(sequence);
        }
        self.push_encoded(batch)
    }

    /// Take producer exports whose encode completion fence has signaled, or
    /// whose backend completed a safe device-idle shutdown.
    pub fn take_completed_inputs(&mut self) -> Vec<u64> {
        self.completed_inputs.drain(..).collect()
    }

    fn shutdown_backend(&mut self) -> Result<(), ReplayEncoderError> {
        if let Some(error) = self.shutdown_error.as_ref() {
            return Err(error.clone());
        }
        if let Some(backend) = self.backend.as_mut()
            && let Err(error) = backend.shutdown()
        {
            // Preserve both the owner and the failed attestation. Repeated
            // cleanup must never convert unfinished GPU reads into ACKs.
            self.shutdown_error = Some(error.clone());
            return Err(error);
        }
        self.backend = None;
        while let Some(sequence) = self.in_flight_inputs.pop_front() {
            if self.completed_inputs.len() < MAX_COMPLETED_INPUTS {
                self.completed_inputs.push_back(sequence);
            }
        }
        Ok(())
    }
}

impl Drop for ReplayHardwarePipeline {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn stream_matches_settings(stream: &ReplayVideoStream, settings: ReplaySettings) -> bool {
    if settings.frame_rate == ReplayFrameRate::Variable {
        stream.codec() == crate::ReplayVideoCodec::H264
            && stream.frames_per_second()
                == redunar_capture_vulkan::replay_video::maximum_variable_frame_rate(
                    stream.width(),
                    stream.height(),
                )
    } else {
        u16::from(stream.frames_per_second()) == settings.frame_rate.frames_per_second()
    }
}

fn in_memory_settings(mut settings: ReplaySettings) -> ReplaySettings {
    if settings.duration.seconds() > MAX_IN_MEMORY_HISTORY.seconds() {
        settings.duration = MAX_IN_MEMORY_HISTORY;
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DmaBufPlane, EncodedPacketBatch, EncodedReplayPacket, HardwareEncodeOutput, ReplayBudget,
        ReplayEncoderError, ReplayPacketFormat, ReplayVideoCodec,
    };
    use std::fs::{self, File};
    use std::os::fd::OwnedFd;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use std::time::Instant;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "redunar-replay-pipeline-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            Self { root }
        }

        fn store(&self) -> ReplayClipStore {
            ReplayClipStore::open(
                &self.root,
                ReplayBudget::from_settings(ReplaySettings::default()),
            )
            .expect("open store")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct FakeBackend {
        stream: ReplayVideoStream,
        shutdowns: Arc<AtomicUsize>,
        fail_encode: bool,
        fail_drain: bool,
        fail_shutdown: bool,
        completion_delay: usize,
        pending: VecDeque<(u64, u64, u64, bool)>,
    }

    impl FakeBackend {
        fn new(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                stream: ReplayVideoStream::new(
                    ReplayVideoCodec::H264,
                    ReplayPacketFormat::H264LengthPrefixed4,
                    width,
                    height,
                    60,
                    vec![
                        1, 66, 0, 30, 0xff, 0xe1, 0, 2, 0x67, 0x42, 1, 0, 2, 0x68, 0xce,
                    ],
                )
                .expect("stream"),
                shutdowns,
                fail_encode: false,
                fail_drain: false,
                fail_shutdown: false,
                completion_delay: 0,
                pending: VecDeque::new(),
            }
        }

        fn failing(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                fail_encode: true,
                ..Self::new(width, height, shutdowns)
            }
        }

        fn drain_failing(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                fail_drain: true,
                ..Self::new(width, height, shutdowns)
            }
        }

        fn shutdown_failing(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                fail_shutdown: true,
                completion_delay: MAX_HARDWARE_INPUTS_IN_FLIGHT,
                ..Self::new(width, height, shutdowns)
            }
        }

        fn delayed(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                completion_delay: 1,
                ..Self::new(width, height, shutdowns)
            }
        }

        fn four_slots(width: u32, height: u32, shutdowns: Arc<AtomicUsize>) -> Self {
            Self {
                completion_delay: MAX_HARDWARE_INPUTS_IN_FLIGHT,
                ..Self::new(width, height, shutdowns)
            }
        }

        fn output(
            completed: impl IntoIterator<Item = (u64, u64, u64, bool)>,
        ) -> Result<HardwareEncodeOutput, ReplayEncoderError> {
            let completed: Vec<_> = completed.into_iter().collect();
            let packets = completed
                .iter()
                .map(|(_, timestamp_ns, duration_ns, keyframe)| {
                    EncodedReplayPacket::new(
                        *timestamp_ns,
                        *duration_ns,
                        *keyframe,
                        vec![0, 0, 0, 2, if *keyframe { 0x65 } else { 0x41 }, 0x88],
                    )
                    .expect("fake packet")
                })
                .collect();
            HardwareEncodeOutput::new(
                EncodedPacketBatch::new(packets)?,
                completed
                    .into_iter()
                    .map(|(sequence, _, _, _)| sequence)
                    .collect(),
            )
        }
    }

    impl HardwareEncoderBackend for FakeBackend {
        fn api(&self) -> HardwareEncoderApi {
            HardwareEncoderApi::VaApi
        }

        fn stream(&self) -> &ReplayVideoStream {
            &self.stream
        }

        fn encode(
            &mut self,
            input_sequence: u64,
            frame: DmaBufReplayFrame,
            force_keyframe: bool,
        ) -> Result<HardwareEncodeOutput, ReplayEncoderError> {
            if self.fail_encode {
                return Err(ReplayEncoderError::BackendFailed("injected".to_owned()));
            }
            let current = (
                input_sequence,
                frame.timestamp_ns,
                frame.duration_ns,
                force_keyframe,
            );
            if self.completion_delay > 0 {
                let completed = (self.pending.len() >= self.completion_delay)
                    .then(|| self.pending.pop_front())
                    .flatten();
                self.pending.push_back(current);
                Self::output(completed)
            } else {
                Self::output(Some(current))
            }
        }

        fn drain(&mut self) -> Result<HardwareEncodeOutput, ReplayEncoderError> {
            if self.fail_drain {
                return Err(ReplayEncoderError::BackendFailed(
                    "injected drain".to_owned(),
                ));
            }
            Self::output(self.pending.drain(..).collect::<Vec<_>>())
        }

        fn shutdown(&mut self) -> Result<(), ReplayEncoderError> {
            self.shutdowns.fetch_add(1, Ordering::Relaxed);
            if self.fail_shutdown {
                return Err(ReplayEncoderError::BackendFailed(
                    "injected shutdown failure".to_owned(),
                ));
            }
            Ok(())
        }
    }

    fn frame(root: &std::path::Path, sequence: u64, width: u32, height: u32) -> DmaBufReplayFrame {
        frame_at(
            root,
            sequence,
            width,
            height,
            sequence.saturating_mul(16_666_667),
        )
    }

    fn frame_at(
        root: &std::path::Path,
        sequence: u64,
        width: u32,
        height: u32,
        timestamp_ns: u64,
    ) -> DmaBufReplayFrame {
        let path = root.join(format!("frame-{sequence}"));
        let fd: OwnedFd = File::create(path).expect("frame fd").into();
        DmaBufReplayFrame::new(
            fd,
            width,
            height,
            0x3432_4152,
            0,
            timestamp_ns,
            16_666_667,
            vec![DmaBufPlane {
                offset: 0,
                stride: width.saturating_mul(4),
            }],
        )
        .expect("DMA-BUF frame")
    }

    #[test]
    fn sparse_capture_requests_keyframes_by_presentation_time() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, shutdowns)),
        )
        .expect("pipeline");

        pipeline
            .submit_frame(1, frame_at(&fixture.root, 1, 1_920, 1_080, 1))
            .expect("initial keyframe");
        pipeline
            .submit_frame(
                2,
                frame_at(
                    &fixture.root,
                    2,
                    1_920,
                    1_080,
                    KEYFRAME_REQUEST_INTERVAL_NS + 1,
                ),
            )
            .expect("time-based keyframe");

        let packets = pipeline.flow.ring().packets().collect::<Vec<_>>();
        assert_eq!(packets.len(), 2);
        assert!(packets[0].is_keyframe());
        assert!(packets[1].is_keyframe());
    }

    #[test]
    fn dma_buf_frames_flow_through_hardware_backend_ring_and_atomic_save() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("first frame");
        pipeline
            .submit_frame(2, frame(&fixture.root, 2, 1_920, 1_080))
            .expect("second frame");
        let clip = pipeline.save_replay().expect("save replay");
        assert!(clip.path.exists());
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Buffering);
        assert_eq!(pipeline.ring_stats().stored_packets, 2);
        assert_eq!(pipeline.store.inventory().expect("inventory").len(), 1);
        pipeline.shutdown().expect("shutdown");
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn game_audio_is_muxed_without_changing_video_encoder_ownership() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("video frame");
        pipeline
            .submit_audio(redunar_capture_audio::EncodedOpusPacket {
                timestamp_ns: 16_666_667,
                duration_ns: 20_000_000,
                bytes: vec![0xf8, 0xff, 0xfe],
            })
            .expect("audio packet");
        pipeline
            .submit_frame(2, frame(&fixture.root, 2, 1_920, 1_080))
            .expect("video frame");
        let clip = pipeline.save_replay().expect("audio/video save");
        let bytes = fs::read(clip.path).expect("saved clip");
        assert!(bytes.windows(6).any(|window| window == b"A_OPUS"));
        assert_eq!(shutdowns.load(Ordering::Relaxed), 0);
        pipeline.shutdown().expect("shutdown");
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn producer_inputs_are_released_only_after_backend_completion_or_safe_shutdown() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::delayed(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");

        pipeline
            .submit_frame(11, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("first frame remains in flight");
        assert!(pipeline.take_completed_inputs().is_empty());

        pipeline
            .submit_frame(12, frame(&fixture.root, 2, 1_920, 1_080))
            .expect("second frame completes the first");
        assert_eq!(pipeline.take_completed_inputs(), vec![11]);

        pipeline.shutdown().expect("safe backend shutdown");
        assert_eq!(pipeline.take_completed_inputs(), vec![12]);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn four_slot_backend_does_not_trip_the_input_completion_guard() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::four_slots(
                1_920,
                1_080,
                Arc::clone(&shutdowns),
            )),
        )
        .expect("pipeline");

        for sequence in 1..=5 {
            pipeline
                .submit_frame(sequence, frame(&fixture.root, sequence, 1_920, 1_080))
                .expect("four-slot submission remains bounded");
        }
        assert_eq!(pipeline.take_completed_inputs(), vec![1]);
        pipeline.save_replay().expect("four-slot drain is valid");
        assert_eq!(pipeline.take_completed_inputs(), vec![2, 3, 4, 5]);
    }

    #[test]
    fn resize_discards_old_epoch_and_accepts_restarted_timestamps() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        pipeline
            .submit_frame(10, frame(&fixture.root, 10, 1_920, 1_080))
            .expect("old epoch");
        pipeline
            .reset(Box::new(FakeBackend::new(
                1_280,
                720,
                Arc::clone(&shutdowns),
            )))
            .expect("reset");
        assert_eq!(pipeline.ring_stats().stored_packets, 0);
        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_280, 720))
            .expect("new epoch");
        assert_eq!(pipeline.stream().width(), 1_280);
        assert_eq!(pipeline.ring_stats().stored_packets, 1);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn format_change_and_backend_error_fail_closed_and_release_resources() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut changed = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        assert!(matches!(
            changed.submit_frame(1, frame(&fixture.root, 1, 1_280, 720)),
            Err(ReplayPipelineError::FrameFormatChanged)
        ));
        assert_eq!(changed.phase(), ReplayPipelinePhase::Failed);
        assert_eq!(changed.ring_stats().stored_packets, 0);

        let mut failed = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::failing(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        assert!(matches!(
            failed.submit_frame(2, frame(&fixture.root, 2, 1_920, 1_080)),
            Err(ReplayPipelineError::Encoder(_))
        ));
        assert_eq!(failed.phase(), ReplayPipelinePhase::Failed);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn empty_save_is_non_terminal_but_bad_frame_timestamp_is_terminal() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        assert!(matches!(
            pipeline.save_replay(),
            Err(ReplayPipelineError::NoEncodedFrames)
        ));
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Buffering);
        pipeline
            .submit_frame(2, frame(&fixture.root, 2, 1_920, 1_080))
            .expect("first timestamp");
        assert!(matches!(
            pipeline.submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080)),
            Err(ReplayPipelineError::FrameTimestampNotMonotonic)
        ));
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Failed);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn storage_replacement_during_save_fails_closed_and_discards_the_ring() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline");
        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("encoded frame");
        fs::write(fixture.root.join(".redunar-replay-store-v1"), b"replaced")
            .expect("replace marker contents");

        assert!(matches!(
            pipeline.save_replay(),
            Err(ReplayPipelineError::Store(_))
        ));
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Failed);
        assert_eq!(pipeline.ring_stats().stored_packets, 0);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn spooled_save_drain_failure_fails_closed_like_the_normal_save_path() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let store = fixture.store();
        let spool = ReplaySegmentSpool::open(fixture.root.join("spool"), ReplaySettings::default())
            .expect("open spool");
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            store,
            Box::new(FakeBackend::drain_failing(
                1_920,
                1_080,
                Arc::clone(&shutdowns),
            )),
        )
        .expect("pipeline")
        .with_spool(spool);

        assert!(matches!(
            pipeline.save_spooled_replay(ReplayDuration::Seconds15),
            Err(ReplayPipelineError::Encoder(_))
        ));
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Failed);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn spooled_save_includes_the_tail_and_forces_a_new_keyframe() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let settings = ReplaySettings::default();
        let store = fixture.store();
        let spool =
            ReplaySegmentSpool::open(fixture.root.join("spool"), settings).expect("open spool");
        let mut pipeline = ReplayHardwarePipeline::new(
            settings,
            store,
            Box::new(FakeBackend::new(1_920, 1_080, Arc::clone(&shutdowns))),
        )
        .expect("pipeline")
        .with_spool(spool);

        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("active-tail frame");
        pipeline
            .submit_frame(60, frame(&fixture.root, 60, 1_920, 1_080))
            .expect("one-second active-tail frame");
        let clip = pipeline
            .save_spooled_replay(ReplayDuration::Seconds15)
            .expect("start tail save")
            .join()
            .expect("assemble tail save");
        assert!(clip.path.exists());

        pipeline
            .submit_frame(61, frame(&fixture.root, 61, 1_920, 1_080))
            .expect("first post-save frame");
        assert!(
            pipeline
                .flow
                .ring()
                .packets()
                .last()
                .expect("post-save packet")
                .is_keyframe()
        );
        pipeline.shutdown().expect("shutdown");
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn audio_snapshot_runs_in_assembler_with_selected_window_and_without_capture_lock() {
        let fixture = Fixture::new();
        let settings = ReplaySettings::default();
        let store = fixture.store();
        let spool = ReplaySegmentSpool::open(fixture.root.join("spool"), settings).unwrap();
        let mut pipeline = ReplayHardwarePipeline::new(
            settings,
            store,
            Box::new(FakeBackend::new(1920, 1080, Arc::new(AtomicUsize::new(0)))),
        )
        .unwrap()
        .with_spool(spool);
        for (position, index) in [
            1, 60, 120, 180, 240, 300, 360, 420, 480, 540, 600, 660, 720, 780, 840, 900, 960, 1020,
            1080, 1140, 1200,
        ]
        .into_iter()
        .enumerate()
        {
            pipeline
                .submit_frame(index, frame(&fixture.root, index, 1920, 1080))
                .unwrap();
            let _ = pipeline.take_completed_inputs();
            // The four-item spool queue intentionally rejects burst submissions;
            // wait for each fixture packet to be accounted before the next one.
            let deadline = Instant::now() + std::time::Duration::from_secs(2);
            while pipeline.spool_stats().unwrap().accepted_packets
                < u64::try_from(position + 1).unwrap()
            {
                assert!(Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let (entered, window) = std::sync::mpsc::channel();
        let (release, continue_mix) = std::sync::mpsc::channel();
        let caller = std::thread::current().id();
        let job = pipeline
            .save_spooled_replay_as_with_audio_tracks(
                ReplayDuration::Seconds15,
                ReplayOutputFormat::Matroska,
                move |start, end| {
                    assert_ne!(std::thread::current().id(), caller);
                    entered.send((start, end)).unwrap();
                    continue_mix
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                    ReplayAudioTracks {
                        system: ReplayAudioSnapshot {
                            stream: redunar_capture_audio::OpusStreamDescription::default(),
                            packets: Vec::new(),
                        },
                        microphone: None,
                    }
                },
            )
            .unwrap();
        let (start, end) = window
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(start > 0);
        assert!(end - start <= 15_000_000_000);
        // Assembly remains deliberately blocked here; subsequent frames must
        // still be accepted by the same active encoder and disk spool.
        pipeline
            .submit_frame(1201, frame(&fixture.root, 1201, 1920, 1080))
            .unwrap();
        release.send(()).unwrap();
        assert!(job.join().unwrap().path.is_file());
        pipeline.shutdown().unwrap();
    }

    #[test]
    fn runtime_resize_discards_both_audio_epochs_and_retains_microphone_mode() {
        let fixture = Fixture::new();
        let settings = ReplaySettings::default();
        let store = fixture.store();
        let spool = ReplaySegmentSpool::open(fixture.root.join("spool"), settings).unwrap();
        let pipeline = ReplayHardwarePipeline::new(
            settings,
            store,
            Box::new(FakeBackend::new(1920, 1080, Arc::new(AtomicUsize::new(0)))),
        )
        .unwrap()
        .with_spool(spool);
        let runtime = crate::ProductionReplayRuntime::unavailable(settings);
        runtime
            .start_validated_pipeline(
                settings,
                pipeline,
                crate::ReplayBackendReadiness::fully_verified(),
            )
            .unwrap();
        runtime.enable_microphone_recording();
        let packet = redunar_capture_audio::EncodedOpusPacket {
            timestamp_ns: 0,
            duration_ns: 20_000_000,
            bytes: vec![1, 2, 3],
        };
        runtime.submit_audio(packet.clone()).unwrap();
        runtime.submit_microphone_audio(packet.clone()).unwrap();
        runtime
            .reset_backend(Box::new(FakeBackend::new(
                1280,
                720,
                Arc::new(AtomicUsize::new(0)),
            )))
            .unwrap();
        runtime
            .submit_audio(packet.clone())
            .expect("system timestamp restarts in new epoch");
        runtime
            .submit_microphone_audio(packet)
            .expect("microphone timestamp restarts in new epoch");
        runtime.shutdown().unwrap();
    }

    #[test]
    fn shutdown_failure_keeps_unfinished_export_ownership_and_is_idempotent() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(FakeBackend::shutdown_failing(
                1_920,
                1_080,
                Arc::clone(&shutdowns),
            )),
        )
        .expect("pipeline");
        pipeline
            .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
            .expect("in-flight frame");
        assert!(pipeline.shutdown().is_err());
        assert_eq!(pipeline.phase(), ReplayPipelinePhase::Shutdown);
        // A failed device-idle shutdown cannot prove GPU ownership ended.
        assert!(pipeline.take_completed_inputs().is_empty());
        assert!(pipeline.shutdown().is_err());
        assert!(pipeline.take_completed_inputs().is_empty());
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn submission_failure_and_repeated_runtime_cleanup_never_ack_unfinished_inputs() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let backend = FakeBackend {
            fail_encode: true,
            ..FakeBackend::shutdown_failing(1_920, 1_080, Arc::clone(&shutdowns))
        };
        let pipeline = ReplayHardwarePipeline::new(
            ReplaySettings::default(),
            fixture.store(),
            Box::new(backend),
        )
        .expect("pipeline")
        .with_spool(
            ReplaySegmentSpool::open(fixture.root.join("spool"), ReplaySettings::default())
                .expect("spool"),
        );
        let runtime = crate::ProductionReplayRuntime::unavailable(ReplaySettings::default());
        runtime
            .start_validated_pipeline(
                ReplaySettings::default(),
                pipeline,
                crate::ReplayBackendReadiness::fully_verified(),
            )
            .expect("activation");
        assert!(
            runtime
                .submit_frame(1, frame(&fixture.root, 1, 1_920, 1_080))
                .is_err()
        );
        runtime.fail(crate::ReplayFailure::EncoderFailed);
        assert!(runtime.take_completed_exports().is_empty());
        assert!(runtime.shutdown().is_err());
        assert!(runtime.shutdown().is_err());
        assert!(runtime.take_completed_exports().is_empty());
        assert_eq!(runtime.status().phase, crate::ReplayPhase::Failed);
        assert_eq!(shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn long_replay_setting_keeps_the_hardware_ring_at_the_two_minute_cap() {
        let fixture = Fixture::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let settings = ReplaySettings {
            duration: ReplayDuration::Seconds900,
            ..ReplaySettings::default()
        };
        let pipeline = ReplayHardwarePipeline::new(
            settings,
            fixture.store(),
            Box::new(FakeBackend::new(1_920, 1_080, shutdowns)),
        )
        .expect("pipeline");

        let expected = crate::ReplayBudget::from_settings(in_memory_settings(settings));
        assert_eq!(
            pipeline.flow.ring().packet_capacity(),
            expected.frame_capacity
        );
        assert_eq!(
            pipeline.flow.ring().byte_capacity(),
            expected.maximum_ring_bytes
        );
    }
}
