use crate::{
    DmaBufReplayFrame, HardwareEncoderBackend, ReplayAudioBuffer, ReplayBackendReadiness,
    ReplayBudget, ReplayCapability, ReplayFailure, ReplayHardwarePipeline, ReplayOutputFormat,
    ReplayPhase, ReplayPipelineError, ReplayRecorderHealth, ReplayRuntimeStatus,
};
use redunar_core::{ReplayDuration, ReplaySettings};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const MAX_COMPLETED_EXPORTS: usize = 8;
/// Clip names kept between the save worker and the next game-session poll.
/// The bound only guards pathological bursts; an overflow degrades to the
/// session-history fallback in the clip inventory, never to a lost clip.
const MAX_COMMITTED_CLIP_NAMES: usize = 32;
const RECORDER_STALL_TIMEOUT: Duration = Duration::from_secs(5);
const AUDIO_STALL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct ProductionReplayRuntime {
    inner: Arc<Mutex<RuntimeState>>,
    audio: Arc<Mutex<ProductionAudioState>>,
}

struct ProductionAudioState {
    recording: bool,
    buffer: ReplayAudioBuffer,
    last_progress: Option<Instant>,
    microphone_buffer: ReplayAudioBuffer,
    microphone_recording: bool,
    microphone_muted: bool,
}

struct RuntimeState {
    status: ReplayRuntimeStatus,
    pipeline: Option<ReplayHardwarePipeline>,
    completed_exports: VecDeque<u64>,
    save_worker: Option<thread::JoinHandle<()>>,
    save_cancel: Option<Arc<AtomicBool>>,
    stopping: bool,
    /// File names of clips durably committed since the coordinator last
    /// drained them. The save worker pushes under the same mutex that bumps
    /// the completed-save revision, so the two always move together.
    committed_clips: VecDeque<String>,
    received_frame_count: u64,
    observed_packet_count: u64,
    last_encoded_progress: Option<Instant>,
    output_format: ReplayOutputFormat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRuntimeError {
    message: String,
}

impl ReplayRuntimeError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ReplayRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ReplayRuntimeError {}

impl ProductionReplayRuntime {
    #[must_use]
    pub fn unavailable(settings: ReplaySettings) -> Self {
        Self {
            audio: Arc::new(Mutex::new(ProductionAudioState {
                recording: false,
                last_progress: None,
                buffer: ReplayAudioBuffer::new(
                    ReplayDuration::Seconds900,
                    redunar_capture_audio::OpusStreamDescription::default(),
                ),
                microphone_buffer: ReplayAudioBuffer::new(
                    ReplayDuration::Seconds900,
                    redunar_capture_audio::OpusStreamDescription::default(),
                ),
                microphone_recording: false,
                microphone_muted: false,
            })),
            inner: Arc::new(Mutex::new(RuntimeState {
                status: ReplayRuntimeStatus::unavailable(settings),
                pipeline: None,
                completed_exports: VecDeque::with_capacity(MAX_COMPLETED_EXPORTS),
                save_worker: None,
                save_cancel: None,
                stopping: false,
                committed_clips: VecDeque::new(),
                received_frame_count: 0,
                observed_packet_count: 0,
                last_encoded_progress: None,
                output_format: ReplayOutputFormat::Matroska,
            })),
        }
    }

    #[must_use]
    pub fn status(&self) -> ReplayRuntimeStatus {
        let mut state = lock_unpoisoned(&self.inner);
        let mut status = state.status;
        let spool_stats = state
            .pipeline
            .as_ref()
            .and_then(ReplayHardwarePipeline::spool_stats);
        let audio_stats = if matches!(status.phase, ReplayPhase::Buffering | ReplayPhase::Saving) {
            let audio = lock_unpoisoned(&self.audio);
            (
                audio.buffer.packet_count(),
                audio.buffer.byte_count(),
                audio.last_progress.map(|progress| progress.elapsed()),
            )
        } else {
            (0, 0, None)
        };
        status.buffered_duration_ns = spool_stats.map_or(0, |stats| stats.buffered_duration_ns);
        status.received_frame_count = state.received_frame_count;
        status.encoded_packet_count = spool_stats.map_or(0, |stats| stats.accepted_packets);
        status.audio_packet_count = audio_stats.0;
        status.audio_byte_count = audio_stats.1;
        status.audio_active = audio_is_active(status.phase, audio_stats.2);
        if status.encoded_packet_count > state.observed_packet_count {
            state.observed_packet_count = status.encoded_packet_count;
            state.last_encoded_progress = Some(Instant::now());
        }
        status.recorder_health = recorder_health(
            status.phase,
            status.encoded_packet_count,
            state
                .last_encoded_progress
                .map(|progress| progress.elapsed()),
        );
        status
    }

    /// Diagnostic aggregates only; never infer drops by subtracting counters
    /// belonging to different epochs or to in-flight work.
    pub(crate) fn diagnostic_counters(&self) -> (usize, Option<crate::ReplaySpoolStats>) {
        let state = lock_unpoisoned(&self.inner);
        state.pipeline.as_ref().map_or((0, None), |pipeline| {
            (pipeline.in_flight_count(), pipeline.spool_stats())
        })
    }

    pub(crate) fn teardown_incomplete(&self) -> bool {
        lock_unpoisoned(&self.inner)
            .pipeline
            .as_ref()
            .is_some_and(ReplayHardwarePipeline::teardown_incomplete)
    }

    pub fn set_output_format(&self, output_format: ReplayOutputFormat) {
        lock_unpoisoned(&self.inner).output_format = output_format;
    }

    #[must_use]
    pub(crate) fn output_format(&self) -> ReplayOutputFormat {
        lock_unpoisoned(&self.inner).output_format
    }

    pub(crate) fn configure_unavailable(&self, settings: ReplaySettings) {
        let mut state = lock_unpoisoned(&self.inner);
        if state.pipeline.is_none() {
            state.status = ReplayRuntimeStatus::unavailable(settings);
        }
    }

    pub(crate) fn configure_validation_candidate(&self, settings: ReplaySettings) {
        let mut state = lock_unpoisoned(&self.inner);
        if state.pipeline.is_none() {
            state.status = ReplayRuntimeStatus::validation_candidate(settings);
        }
    }

    /// Attach an end-to-end validated backend. The readiness attestation is
    /// supplied by the validation owner rather than manufactured here, so a
    /// successful device probe or encoder initialization cannot open Replay.
    pub(crate) fn start_validated_pipeline(
        &self,
        settings: ReplaySettings,
        pipeline: ReplayHardwarePipeline,
        readiness: ReplayBackendReadiness,
    ) -> Result<(), ReplayRuntimeError> {
        self.reap_finished_save();
        validate_production_readiness(readiness)?;
        let mut state = lock_unpoisoned(&self.inner);
        if state.pipeline.is_some() {
            return Err(ReplayRuntimeError::new(
                "an Instant Replay backend is already active",
            ));
        }
        // A terminal failure normally clears the pipeline and reports Failed,
        // which the export worker may re-arm within the same game session.
        // While a clip assembly is still finishing, replacing the pipeline
        // would let a second save race the pending completion worker, so the
        // retry waits for that worker to restore the recording phase.
        if state.status.phase == ReplayPhase::Saving || state.save_worker.is_some() {
            return Err(ReplayRuntimeError::new(
                "an Instant Replay clip is still being saved",
            ));
        }
        if !pipeline.has_spool() {
            return Err(ReplayRuntimeError::new(
                "an Instant Replay backend must include the bounded disk spool",
            ));
        }
        state.status = ReplayRuntimeStatus {
            phase: ReplayPhase::Buffering,
            capability: if readiness.activation_allowed() {
                ReplayCapability::Available
            } else {
                ReplayCapability::ValidationCandidate
            },
            backend_readiness: readiness,
            last_failure: None,
            completed_save_revision: state.status.completed_save_revision,
            buffered_duration_ns: 0,
            received_frame_count: 0,
            encoded_packet_count: 0,
            audio_packet_count: 0,
            audio_byte_count: 0,
            audio_active: false,
            recorder_health: ReplayRecorderHealth::Starting,
            settings,
            budget: ReplayBudget::from_settings(settings),
        };
        state.received_frame_count = 0;
        state.stopping = false;
        state.observed_packet_count = 0;
        state.last_encoded_progress = None;
        let mut audio = lock_unpoisoned(&self.audio);
        audio.buffer.clear();
        audio.last_progress = None;
        audio.recording = true;
        state.pipeline = Some(pipeline);
        Ok(())
    }

    /// Submit one already-exported DMA-BUF to the active hardware backend.
    /// Failure is terminal for this session and clears the advertised buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when no verified backend is recording or when DMA-BUF
    /// import, encoding, or bounded packet collection fails.
    pub fn submit_frame(
        &self,
        input_sequence: u64,
        frame: DmaBufReplayFrame,
    ) -> Result<(), ReplayRuntimeError> {
        let mut state = lock_unpoisoned(&self.inner);
        state.received_frame_count = state.received_frame_count.saturating_add(1);
        let pipeline = state
            .pipeline
            .as_mut()
            .ok_or_else(|| ReplayRuntimeError::new("Instant Replay is not recording"))?;
        let result = pipeline.submit_frame(input_sequence, frame);
        let completed = pipeline.take_completed_inputs();
        if enqueue_completed(&mut state, completed).is_err() {
            mark_failed(&mut state, &self.audio, ReplayFailure::EncoderFailed);
            return Err(ReplayRuntimeError::new(
                "Instant Replay completion queue exceeded its fixed bound",
            ));
        }
        if let Err(error) = result {
            mark_failed(&mut state, &self.audio, ReplayFailure::EncoderFailed);
            return Err(ReplayRuntimeError::new(error.to_string()));
        }
        Ok(())
    }

    /// Add one encoded game-audio packet without making audio availability an
    /// authority over the already working video recorder.
    ///
    /// # Errors
    ///
    /// Returns an error when Replay is not buffering or the audio packet is
    /// malformed. Callers stop only the audio worker; video remains active.
    pub(crate) fn submit_audio(
        &self,
        packet: redunar_capture_audio::EncodedOpusPacket,
    ) -> Result<(), ReplayRuntimeError> {
        // Video encoding and save drain can block the runtime mutex for long
        // enough to starve 20 ms audio packets. Keep ingestion independent.
        let mut audio = lock_unpoisoned(&self.audio);
        if !audio.recording {
            return Err(ReplayRuntimeError::new("Instant Replay is not recording"));
        }
        audio
            .buffer
            .push(packet)
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?;
        audio.last_progress = Some(Instant::now());
        Ok(())
    }

    /// Add one encoded microphone packet without affecting system audio or video.
    pub(crate) fn submit_microphone_audio(
        &self,
        packet: redunar_capture_audio::EncodedOpusPacket,
    ) -> Result<(), ReplayRuntimeError> {
        let mut audio = lock_unpoisoned(&self.audio);
        if !audio.recording || !audio.microphone_recording || audio.microphone_muted {
            return Err(ReplayRuntimeError::new("microphone capture is inactive"));
        }
        audio
            .microphone_buffer
            .push(packet)
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))
    }

    pub(crate) fn set_microphone_muted(&self, muted: bool) {
        lock_unpoisoned(&self.audio).microphone_muted = muted;
    }

    /// Replace the active encoder after the capture source changes size.
    /// Old codec history is discarded by the pipeline before the replacement
    /// can accept a frame, so dimensions never mix in one saved clip.
    pub(crate) fn reset_backend(
        &self,
        replacement: Box<dyn HardwareEncoderBackend>,
    ) -> Result<(), ReplayRuntimeError> {
        let mut state = lock_unpoisoned(&self.inner);
        let pipeline = state
            .pipeline
            .as_mut()
            .ok_or_else(|| ReplayRuntimeError::new("Instant Replay is not recording"))?;
        let result = pipeline.reset(replacement);
        let completed = pipeline.take_completed_inputs();
        if enqueue_completed(&mut state, completed).is_err() {
            mark_failed(&mut state, &self.audio, ReplayFailure::EncoderFailed);
            return Err(ReplayRuntimeError::new(
                "Instant Replay completion queue exceeded its fixed bound",
            ));
        }
        if let Err(error) = result {
            mark_failed(&mut state, &self.audio, ReplayFailure::EncoderFailed);
            return Err(ReplayRuntimeError::new(error.to_string()));
        }
        let mut audio = lock_unpoisoned(&self.audio);
        audio.buffer.clear();
        audio.last_progress = None;
        Ok(())
    }

    /// Publish a component-local startup/transport failure. This never ends
    /// the owning game or capture session.
    pub(crate) fn fail(&self, failure: ReplayFailure) {
        mark_failed(&mut lock_unpoisoned(&self.inner), &self.audio, failure);
    }

    /// Take only producer export sequences whose GPU input ownership has
    /// ended at a completion fence or safe backend shutdown.
    pub(crate) fn take_completed_exports(&self) -> Vec<u64> {
        lock_unpoisoned(&self.inner)
            .completed_exports
            .drain(..)
            .collect()
    }

    /// Take the clip file names committed since the previous drain, oldest
    /// first. Pairs with changes in the completed-save revision so the game
    /// session can attribute each saved clip to the game that recorded it.
    pub(crate) fn take_committed_clip_names(&self) -> Vec<String> {
        lock_unpoisoned(&self.inner)
            .committed_clips
            .drain(..)
            .collect()
    }

    /// Hand one duration-selected save to the isolated spool assembler.
    /// Recording continues while the worker commits the clip.
    ///
    /// # Errors
    ///
    /// Returns an error when Replay is not buffering, a save is already in
    /// flight, the spool cannot form a snapshot, or the status worker fails.
    pub fn save(&self, duration: ReplayDuration) -> Result<(), ReplayRuntimeError> {
        self.reap_finished_save();
        let mut state = lock_unpoisoned(&self.inner);
        if state.stopping || state.save_worker.is_some() {
            return Err(ReplayRuntimeError::new(
                "Instant Replay is stopping or finishing a save",
            ));
        }
        let job = self.prepare_save(&mut state, duration)?;
        crate::diagnostic_log::log(&format!(
            "Replay event=save_requested seconds={}",
            duration.seconds()
        ));
        state.save_cancel = Some(job.cancellation());

        let runtime = self.clone();
        let worker = thread::Builder::new()
            .name("redunar-replay-save-status".to_owned())
            .spawn(move || {
                let result = job.join();
                let mut state = lock_unpoisoned(&runtime.inner);
                match result {
                    Ok(stored) if state.status.phase == ReplayPhase::Saving => {
                        crate::diagnostic_log::log("Replay event=save_committed");
                        // Remember the committed file name beside the revision
                        // bump so the game-session coordinator can attribute
                        // the clip to the game that recorded it. Only the name
                        // is kept; the clip itself is already durable.
                        if let Some(name) = stored.path.file_name().and_then(|name| name.to_str()) {
                            if state.committed_clips.len() >= MAX_COMMITTED_CLIP_NAMES {
                                state.committed_clips.pop_front();
                            }
                            state.committed_clips.push_back(name.to_owned());
                        }
                        state.status.phase = ReplayPhase::Buffering;
                        state.status.last_failure = None;
                        state.status.completed_save_revision =
                            state.status.completed_save_revision.saturating_add(1);
                    }
                    Err(error) if state.status.phase == ReplayPhase::Saving => {
                        // Clip assembly is isolated from the active encoder and
                        // spool. A storage failure must not discard otherwise
                        // healthy rolling history or force the counter to zero.
                        crate::log_op!("Redunar could not save the Replay clip: {error}");
                        state.status.phase = ReplayPhase::Buffering;
                        state.status.last_failure = Some(ReplayFailure::StorageFailed);
                    }
                    Ok(_) | Err(_) => {}
                }
            });
        match worker {
            Ok(worker) => {
                state.save_worker = Some(worker);
                Ok(())
            }
            Err(error) => {
                if state.status.phase == ReplayPhase::Saving {
                    state.status.phase = ReplayPhase::Buffering;
                }
                Err(ReplayRuntimeError::new(format!(
                    "could not track Replay save: {error}"
                )))
            }
        }
    }

    fn prepare_save(
        &self,
        state: &mut RuntimeState,
        duration: ReplayDuration,
    ) -> Result<crate::ReplayAssemblyJob, ReplayRuntimeError> {
        if state.status.phase != ReplayPhase::Buffering {
            return Err(ReplayRuntimeError::new(match state.status.phase {
                ReplayPhase::Saving => "an Instant Replay clip is already being saved",
                ReplayPhase::Failed => "Instant Replay stopped after a recording error",
                _ => "Instant Replay is not recording",
            }));
        }
        let output_format = state.output_format;
        let pipeline = state
            .pipeline
            .as_mut()
            .ok_or_else(|| ReplayRuntimeError::new("Instant Replay is not recording"))?;
        if pipeline
            .spool_stats()
            .is_none_or(|stats| stats.buffered_duration_ns < 1_000_000_000)
        {
            return Err(ReplayRuntimeError::new(
                "Instant Replay is still buffering; wait at least one second before saving",
            ));
        }
        let result = pipeline.save_spooled_replay_as_with_audio(duration, output_format, || {
            lock_unpoisoned(&self.audio)
                .buffer
                .snapshot_between(0, u64::MAX)
        });
        let completed = pipeline.take_completed_inputs();
        if enqueue_completed(state, completed).is_err() {
            mark_failed(state, &self.audio, ReplayFailure::EncoderFailed);
            return Err(ReplayRuntimeError::new(
                "Instant Replay completion queue exceeded its fixed bound",
            ));
        }
        match result {
            Ok(job) => {
                state.status.phase = ReplayPhase::Saving;
                state.status.last_failure = None;
                Ok(job)
            }
            Err(error) => {
                if matches!(error, ReplayPipelineError::InsufficientHistory) {
                    return Err(ReplayRuntimeError::new(error.to_string()));
                }
                if matches!(
                    error,
                    ReplayPipelineError::Spool(_) | ReplayPipelineError::Store(_)
                ) {
                    state.status.last_failure = Some(ReplayFailure::StorageFailed);
                } else {
                    mark_failed(state, &self.audio, replay_failure(&error));
                }
                Err(ReplayRuntimeError::new(error.to_string()))
            }
        }
    }

    fn reap_finished_save(&self) {
        let finished_worker = {
            let mut state = lock_unpoisoned(&self.inner);
            if state
                .save_worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
            {
                state.save_worker.take()
            } else {
                None
            }
        };
        if let Some(worker) = finished_worker {
            let _ = worker.join();
        }
    }

    /// Stop encoding, discard buffered history, and release backend resources.
    ///
    /// # Errors
    ///
    /// Returns an error if the hardware backend cannot shut down cleanly.
    pub fn shutdown(&self) -> Result<(), ReplayRuntimeError> {
        let save_worker = {
            let mut state = lock_unpoisoned(&self.inner);
            state.stopping = true;
            if let Some(cancel) = state.save_cancel.take() {
                cancel.store(true, Ordering::Release);
            }
            state.save_worker.take()
        };
        if let Some(worker) = save_worker {
            let _ = worker.join();
        }
        let mut state = lock_unpoisoned(&self.inner);
        let result = state
            .pipeline
            .as_mut()
            .map_or(Ok(()), ReplayHardwarePipeline::shutdown);
        let completed = state
            .pipeline
            .as_mut()
            .map_or_else(Vec::new, ReplayHardwarePipeline::take_completed_inputs);
        let completion_result = enqueue_completed(&mut state, completed);
        if result.is_ok() {
            state.pipeline = None;
        }
        let mut audio = lock_unpoisoned(&self.audio);
        audio.recording = false;
        audio.buffer.clear();
        audio.microphone_buffer.clear();
        audio.microphone_recording = false;
        state.status.phase = if result.is_err() {
            state.status.last_failure = Some(ReplayFailure::EncoderFailed);
            ReplayPhase::Failed
        } else if state.status.backend_readiness.validation_allowed() {
            ReplayPhase::Inactive
        } else {
            ReplayPhase::Unavailable
        };
        result
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))
            .and(completion_result)
    }
}

pub(crate) fn validate_production_readiness(
    readiness: ReplayBackendReadiness,
) -> Result<(), ReplayRuntimeError> {
    if !readiness.validation_allowed() {
        return Err(ReplayRuntimeError::new(
            "Instant Replay has not passed live transfer, hardware encode, and playable output validation",
        ));
    }
    Ok(())
}

fn replay_failure(error: &ReplayPipelineError) -> ReplayFailure {
    match error {
        ReplayPipelineError::Store(_) | ReplayPipelineError::Spool(_) => {
            ReplayFailure::StorageFailed
        }
        _ => ReplayFailure::EncoderFailed,
    }
}

fn mark_failed(
    state: &mut RuntimeState,
    audio: &Mutex<ProductionAudioState>,
    failure: ReplayFailure,
) {
    let shutdown_ok = if let Some(pipeline) = state.pipeline.as_mut() {
        let shutdown_ok = pipeline.shutdown().is_ok();
        let completed = pipeline.take_completed_inputs();
        let _ = enqueue_completed(state, completed);
        shutdown_ok
    } else {
        true
    };
    if shutdown_ok {
        state.pipeline = None;
    }
    let mut audio = lock_unpoisoned(audio);
    audio.recording = false;
    audio.buffer.clear();
    state.status.phase = ReplayPhase::Failed;
    state.status.last_failure = Some(failure);
}

fn enqueue_completed(
    state: &mut RuntimeState,
    completed: Vec<u64>,
) -> Result<(), ReplayRuntimeError> {
    if state
        .completed_exports
        .len()
        .saturating_add(completed.len())
        > MAX_COMPLETED_EXPORTS
    {
        return Err(ReplayRuntimeError::new(
            "Instant Replay completion queue exceeded its fixed bound",
        ));
    }
    state.completed_exports.extend(completed);
    Ok(())
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn recorder_health(
    phase: ReplayPhase,
    encoded_packet_count: u64,
    progress_age: Option<Duration>,
) -> ReplayRecorderHealth {
    match phase {
        ReplayPhase::Failed => ReplayRecorderHealth::Failed,
        ReplayPhase::Buffering | ReplayPhase::Saving if encoded_packet_count == 0 => {
            ReplayRecorderHealth::Starting
        }
        ReplayPhase::Buffering | ReplayPhase::Saving
            if progress_age.is_some_and(|age| age > RECORDER_STALL_TIMEOUT) =>
        {
            ReplayRecorderHealth::Stalled
        }
        ReplayPhase::Buffering | ReplayPhase::Saving => ReplayRecorderHealth::Healthy,
        ReplayPhase::Unavailable | ReplayPhase::Inactive => ReplayRecorderHealth::Inactive,
    }
}

fn audio_is_active(phase: ReplayPhase, progress_age: Option<Duration>) -> bool {
    matches!(phase, ReplayPhase::Buffering | ReplayPhase::Saving)
        && progress_age.is_some_and(|age| age <= AUDIO_STALL_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_progress_survives_full_buffer_and_reset_without_polling() {
        let runtime = ProductionReplayRuntime::unavailable(ReplaySettings::default());
        lock_unpoisoned(&runtime.inner).status.phase = ReplayPhase::Buffering;
        lock_unpoisoned(&runtime.audio).recording = true;
        let packet = |index: u64| redunar_capture_audio::EncodedOpusPacket {
            timestamp_ns: index * 20_000_000,
            duration_ns: 20_000_000,
            bytes: vec![1, 2, 3],
        };
        for index in 0..45_000 {
            runtime.submit_audio(packet(index)).unwrap();
        }
        assert_eq!(runtime.status().audio_packet_count, 45_000);
        lock_unpoisoned(&runtime.audio).last_progress =
            Some(Instant::now().checked_sub(Duration::from_secs(4)).unwrap());
        assert!(!runtime.status().audio_active);
        runtime.submit_audio(packet(45_000)).unwrap();
        assert!(runtime.status().audio_active);
        assert_eq!(runtime.status().audio_packet_count, 45_000);
        {
            let mut audio = lock_unpoisoned(&runtime.audio);
            audio.buffer.clear();
            audio.last_progress = None;
        }
        assert!(!runtime.status().audio_active);
        runtime.submit_audio(packet(0)).unwrap();
        assert!(runtime.status().audio_active);
    }

    #[test]
    fn unavailable_runtime_refuses_frame_and_save_without_mutating_capability() {
        let runtime = ProductionReplayRuntime::unavailable(ReplaySettings::default());
        assert!(runtime.save(ReplayDuration::Seconds15).is_err());
        let status = runtime.status();
        assert_eq!(status.phase, ReplayPhase::Unavailable);
        assert!(!status.backend_readiness.activation_allowed());
    }

    #[test]
    fn unavailable_runtime_shutdown_is_idempotent() {
        let runtime = ProductionReplayRuntime::unavailable(ReplaySettings::default());
        runtime.shutdown().expect("first shutdown");
        runtime.shutdown().expect("second shutdown");
        assert_eq!(runtime.status().phase, ReplayPhase::Unavailable);
    }

    #[test]
    fn headless_foundation_cannot_open_the_production_gate() {
        let error = validate_production_readiness(ReplayBackendReadiness::headless_foundation())
            .expect_err("headless readiness must remain unavailable");
        assert!(error.to_string().contains("has not passed"));
    }

    #[test]
    fn complete_internal_attestation_passes_the_activation_preflight() {
        validate_production_readiness(ReplayBackendReadiness::fully_verified())
            .expect("complete internal attestation");
    }

    #[test]
    fn recorder_health_requires_recent_encoded_progress() {
        assert_eq!(
            recorder_health(ReplayPhase::Buffering, 0, None),
            ReplayRecorderHealth::Starting
        );
        assert_eq!(
            recorder_health(ReplayPhase::Buffering, 10, Some(RECORDER_STALL_TIMEOUT)),
            ReplayRecorderHealth::Healthy
        );
        assert_eq!(
            recorder_health(
                ReplayPhase::Buffering,
                10,
                Some(RECORDER_STALL_TIMEOUT + Duration::from_millis(1))
            ),
            ReplayRecorderHealth::Stalled
        );
    }

    #[test]
    fn audio_activity_requires_recent_packet_progress() {
        assert!(!audio_is_active(ReplayPhase::Buffering, None));
        assert!(audio_is_active(
            ReplayPhase::Buffering,
            Some(AUDIO_STALL_TIMEOUT)
        ));
        assert!(!audio_is_active(
            ReplayPhase::Buffering,
            Some(AUDIO_STALL_TIMEOUT + Duration::from_millis(1))
        ));
        assert!(!audio_is_active(
            ReplayPhase::Inactive,
            Some(Duration::ZERO)
        ));
    }
}
