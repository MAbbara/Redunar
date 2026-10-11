use crate::capture_session::ReplayExportEndpoint;
use crate::replay_menu::{ReplayMenuAction, ReplayMenuState};
use crate::{
    CaptureLaunchDisposition, CaptureLaunchProcessState, CapturePhase, CaptureSessionError,
    CaptureSessionHandle, DmaBufReplayFrame, HardwareEncoderBackend, MicrophoneMode,
    OverlayRuntimeStatus, ProductionReplayRuntime, ReplayBackendReadiness, ReplayFailure,
    ReplayHardwarePipeline, ReplayPhase, ReplayPreferences, ReplayRuntimeError,
    ReplayRuntimeStatus, SteamActivationState, replay_preferences,
};
use redunar_capture::CaptureApi;
use redunar_core::{GameId, ReplayDuration, ReplaySettings, SystemSnapshot};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

mod saved_notice;

const REPLAY_EXPORT_WAIT: Duration = Duration::from_millis(250);
/// First wait before the pump re-arms a replacement recorder after a
/// recoverable failure. Each consecutive failed re-arm doubles the wait up
/// to the maximum so a persistently broken encoder retries slowly instead of
/// spinning or ending the game session's replay for good.
const REPLAY_REARM_BACKOFF: Duration = Duration::from_millis(500);
const REPLAY_REARM_BACKOFF_MAX: Duration = Duration::from_secs(10);
/// Consecutive frame-source failures tolerated before the running recorder
/// is failed over so the reported health reflects the missing source.
const REPLAY_SOURCE_ERROR_LIMIT: u32 = 10;
const AUDIO_CAPTURE_STALL_TIMEOUT: Duration = Duration::from_secs(3);
const AUDIO_RECONNECT_DELAY: Duration = Duration::from_millis(250);
const AUDIO_ROUTE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// One bounded, daemon-owned view of all work attached to a running game.
///
/// Optional capture and overlay failures are deliberately component-local. A
/// failed optional component never changes the game lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameSessionComponentState {
    Disabled,
    Unavailable,
    Pending,
    Active,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameSessionPhase {
    Idle,
    Launching,
    Running,
    Ending,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameSessionFeatureRequest {
    Disabled,
    Enabled,
}

impl GameSessionFeatureRequest {
    const fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl From<bool> for GameSessionFeatureRequest {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameSessionRequest {
    pub game_id: Option<GameId>,
    pub game_name: String,
    pub metrics: GameSessionFeatureRequest,
    pub overlay: GameSessionFeatureRequest,
    pub replay: GameSessionFeatureRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameSessionStatus {
    pub revision: u64,
    pub phase: GameSessionPhase,
    pub game_id: Option<GameId>,
    pub game_name: Option<String>,
    pub metrics: GameSessionComponentState,
    pub overlay: GameSessionComponentState,
    pub replay: GameSessionComponentState,
    pub failure: Option<String>,
}

impl Default for GameSessionStatus {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: GameSessionPhase::Idle,
            game_id: None,
            game_name: None,
            metrics: GameSessionComponentState::Disabled,
            overlay: GameSessionComponentState::Disabled,
            replay: GameSessionComponentState::Disabled,
            failure: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameSessionError {
    message: String,
}

impl GameSessionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GameSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GameSessionError {}

/// Shared production coordinator cached by [`crate::RedunarService`].
/// Native windows share this owner so view lifetimes cannot discard capture or replay state.
#[derive(Clone)]
pub struct ProductionGameSessionCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    state: Mutex<CoordinatorState>,
    replay: ProductionReplayRuntime,
    replay_menu: Mutex<ReplayMenuState>,
    replay_preferences: Mutex<ReplayPreferences>,
    replay_menu_revision: std::sync::atomic::AtomicU64,
    /// Private state directory holding the clip-to-game attribution ledger.
    clip_games_state: PathBuf,
}

#[derive(Default)]
struct CoordinatorState {
    status: GameSessionStatus,
    request: Option<GameSessionRequest>,
    capture: Option<CaptureSessionHandle>,
    noticed_save_revision: u64,
    /// Completed-save revision last attributed in the clip-games ledger.
    recorded_save_revision: u64,
    pending_replay_releases: VecDeque<u64>,
    replay_pump: Option<ReplayExportPump>,
}

struct ReplayExportPump {
    stop: Arc<AtomicBool>,
    source: Arc<dyn ReplayFrameSource>,
    worker: Option<JoinHandle<Result<(), ReplayRuntimeError>>>,
}

pub(crate) trait ReplayFrameSource: Send + Sync {
    fn take_rejected_exports(&self) -> Vec<u64> {
        Vec::new()
    }
    fn take_source_rejection(&self) -> Option<redunar_capture::ReplaySourceRejection> {
        None
    }
    fn wait_next(
        &self,
        timeout: Duration,
    ) -> Result<Option<(u64, DmaBufReplayFrame)>, ReplayRuntimeError>;
    fn release(&self, sequence: u64) -> Result<(), ReplayRuntimeError>;
    fn drain_and_release(&self) -> Result<(), ReplayRuntimeError>;
    fn wake(&self);
}

impl ReplayFrameSource for ReplayExportEndpoint {
    fn take_rejected_exports(&self) -> Vec<u64> {
        ReplayExportEndpoint::take_rejected_exports(self)
    }
    fn take_source_rejection(&self) -> Option<redunar_capture::ReplaySourceRejection> {
        ReplayExportEndpoint::take_source_rejection(self)
    }
    fn wait_next(
        &self,
        timeout: Duration,
    ) -> Result<Option<(u64, DmaBufReplayFrame)>, ReplayRuntimeError> {
        let Some(export) = ReplayExportEndpoint::wait_next(self, timeout) else {
            return Ok(None);
        };
        match ReplayExportEndpoint::import(self, export) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) => {
                // The endpoint queues any failed safe ACK for this rejected,
                // unsubmitted input; the next pump turn retries its original route.
                Err(ReplayRuntimeError::new(error.to_string()))
            }
        }
    }

    fn release(&self, sequence: u64) -> Result<(), ReplayRuntimeError> {
        ReplayExportEndpoint::release(self, sequence)
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))
    }

    fn drain_and_release(&self) -> Result<(), ReplayRuntimeError> {
        ReplayExportEndpoint::drain_and_release(self)
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))
    }

    fn wake(&self) {
        ReplayExportEndpoint::wake(self);
    }
}

type ReplayPipelineFactory =
    Arc<dyn Fn(&DmaBufReplayFrame) -> Result<ReplayHardwarePipeline, String> + Send + Sync>;
type ReplayBackendFactory = Arc<
    dyn Fn(&DmaBufReplayFrame) -> Result<Box<dyn HardwareEncoderBackend>, String> + Send + Sync,
>;

impl ReplayExportPump {
    fn stop_and_join(&mut self) -> Result<(), ReplayRuntimeError> {
        self.stop.store(true, Ordering::Release);
        self.source.wake();
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| ReplayRuntimeError::new("Replay export worker panicked"))?
    }
}

impl Drop for ReplayExportPump {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

impl ProductionGameSessionCoordinator {
    #[must_use]
    pub(crate) fn new(state_directory: PathBuf) -> Self {
        let preferences = replay_preferences::load(&state_directory).unwrap_or_default();
        let mut replay_menu = ReplayMenuState::default();
        replay_menu.set_initial_duration(preferences.initial_save_duration);
        Self {
            inner: Arc::new(CoordinatorInner {
                state: Mutex::new(CoordinatorState::default()),
                replay: ProductionReplayRuntime::unavailable(ReplaySettings::default()),
                replay_menu: Mutex::new(replay_menu),
                replay_preferences: Mutex::new(preferences),
                replay_menu_revision: std::sync::atomic::AtomicU64::new(2),
                clip_games_state: state_directory,
            }),
        }
    }

    #[must_use]
    pub fn status(&self) -> GameSessionStatus {
        lock_unpoisoned(&self.inner.state).status.clone()
    }

    #[must_use]
    pub fn replay_runtime(&self) -> ProductionReplayRuntime {
        self.inner.replay.clone()
    }

    /// Hand a duration-selected save to the daemon-owned Replay runtime.
    ///
    /// # Errors
    ///
    /// Returns an error when Replay is unavailable, inactive, failed, or
    /// already assembling another clip.
    pub fn save_replay(&self, duration: ReplayDuration) -> Result<(), ReplayRuntimeError> {
        self.inner.replay.save(duration)
    }

    pub(crate) fn toggle_replay_menu(&self, now: Instant) -> Result<bool, ReplayRuntimeError> {
        let can_open = {
            let state = lock_unpoisoned(&self.inner.state);
            replay_menu_has_render_target(&state)
        };
        let currently_visible = lock_unpoisoned(&self.inner.replay_menu).is_visible();
        if !currently_visible && !can_open {
            return Err(ReplayRuntimeError::new(
                "Replay menu requires a connected game capture",
            ));
        }
        let visible = lock_unpoisoned(&self.inner.replay_menu).toggle(now);
        let published = self.try_publish_replay_menu();
        if visible && let Err(error) = published {
            // Do not let the helper grab the mouse for a menu the game
            // cannot receive. A later shortcut press may retry normally.
            lock_unpoisoned(&self.inner.replay_menu).close();
            return Err(error);
        }
        Ok(visible)
    }

    /// Close the Replay menu and publish the corresponding daemon state.
    ///
    /// Native shells call this when their own menu surface is hidden so the
    /// backend cannot retain a stale visible/pressed state between toggles.
    pub fn close_replay_menu(&self) {
        lock_unpoisoned(&self.inner.replay_menu).close();
        self.publish_replay_menu();
    }

    pub(crate) fn move_replay_menu_cursor(&self, dx: i32, dy: i32, now: Instant) {
        lock_unpoisoned(&self.inner.replay_menu).move_cursor(dx, dy, now);
        self.publish_replay_menu();
    }

    pub(crate) fn replay_menu_button(
        &self,
        pressed: bool,
        now: Instant,
    ) -> Result<bool, ReplayRuntimeError> {
        let runtime = self.inner.replay.status();
        let ready = runtime.phase == ReplayPhase::Buffering
            && runtime.buffered_duration_ns >= 1_000_000_000;
        let (save, closed) = {
            let mut menu = lock_unpoisoned(&self.inner.replay_menu);
            let save = menu.button(pressed, ready, now);
            (save, !menu.is_visible())
        };
        if let Some(action) = save {
            match action {
                ReplayMenuAction::Save(duration) => self.save_replay(duration)?,
                ReplayMenuAction::SetOutputFormat(format) => {
                    let preferences =
                        replay_preferences::set_output_format(&self.inner.clip_games_state, format)
                            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?;
                    self.inner.replay.set_output_format(format);
                    *lock_unpoisoned(&self.inner.replay_preferences) = preferences;
                }
            }
        }
        self.publish_replay_menu();
        Ok(closed)
    }

    pub(crate) fn replay_menu_heartbeat(&self, now: Instant) {
        lock_unpoisoned(&self.inner.replay_menu).heartbeat(now);
        self.publish_replay_menu();
    }

    pub(crate) fn replay_menu_watchdog(&self, now: Instant) -> bool {
        let render_target_available = {
            let state = lock_unpoisoned(&self.inner.state);
            replay_menu_has_render_target(&state)
        };
        let closed = {
            let mut menu = lock_unpoisoned(&self.inner.replay_menu);
            if menu.is_visible() && !render_target_available {
                menu.close();
                true
            } else {
                menu.close_if_stale(now)
            }
        };
        if closed {
            self.publish_replay_menu();
        }
        closed
    }

    pub(crate) fn replay_menu_is_visible(&self) -> bool {
        lock_unpoisoned(&self.inner.replay_menu).is_visible()
    }

    #[must_use]
    pub(crate) fn replay_menu_telemetry(
        &self,
        revision: u64,
    ) -> redunar_capture::ReplayMenuTelemetry {
        lock_unpoisoned(&self.inner.replay_menu).telemetry(
            revision,
            self.inner.replay.status(),
            self.inner.replay.output_format(),
            &lock_unpoisoned(&self.inner.replay_preferences),
        )
    }

    pub(crate) fn update_replay_preferences(&self, preferences: ReplayPreferences) {
        lock_unpoisoned(&self.inner.replay_menu)
            .set_initial_duration(preferences.initial_save_duration);
        *lock_unpoisoned(&self.inner.replay_preferences) = preferences;
        self.publish_replay_menu();
    }

    fn publish_replay_menu(&self) {
        let _ = self.try_publish_replay_menu();
    }

    fn try_publish_replay_menu(&self) -> Result<(), ReplayRuntimeError> {
        let revision = self
            .inner
            .replay_menu_revision
            .fetch_add(2, Ordering::Relaxed)
            .saturating_add(2);
        let telemetry = self.replay_menu_telemetry(revision);
        let state = lock_unpoisoned(&self.inner.state);
        let capture = state
            .capture
            .as_ref()
            .ok_or_else(|| ReplayRuntimeError::new("Replay menu has no connected game capture"))?;
        capture
            .update_replay_menu(telemetry)
            .map_err(|_| ReplayRuntimeError::new("Replay menu could not be sent to the game"))
    }

    /// Attribute clips committed since the last poll to the live game.
    ///
    /// The save worker queues each committed clip name under the same mutex
    /// as the completed-save revision bump, so draining names here cannot
    /// race the counter. Attribution is labeling data: any ledger write
    /// failure is swallowed after one warning and never affects the save or
    /// the game session. If two saves complete between polls, both names were
    /// queued, so each clip still gets its own line.
    pub fn flush_completed_clip_attribution(&self) {
        let completed = self.inner.replay.status().completed_save_revision;
        let game_name = {
            let mut state = lock_unpoisoned(&self.inner.state);
            if state.recorded_save_revision >= completed {
                return;
            }
            state.recorded_save_revision = completed;
            state.status.game_name.clone()
        };
        let Some(game_name) = game_name else {
            // A save outside a named Redunar session stays unattributed in
            // the ledger; the inventory's session-window fallback still gets
            // a chance when the session history later records a game.
            let _ = self.inner.replay.take_committed_clip_names();
            return;
        };
        for name in self.inner.replay.take_committed_clip_names() {
            if let Err(error) =
                crate::replay_clip_games::record(&self.inner.clip_games_state, &name, &game_name)
            {
                crate::log_op!("Redunar could not attribute a saved Replay clip: {error}");
            }
        }
    }

    /// Forward a completed-save presentation edge to the active capture
    /// overlay. Replay storage remains authoritative; absence of an overlay
    /// never changes the save result.
    ///
    /// # Errors
    ///
    /// Returns an error if no capture runtime is active or its telemetry file
    /// cannot be updated.
    pub fn publish_replay_saved_notice(&self) -> Result<(), ReplayRuntimeError> {
        let completed = self.inner.replay.status().completed_save_revision;
        let mut state = lock_unpoisoned(&self.inner.state);
        let CoordinatorState {
            capture,
            noticed_save_revision,
            ..
        } = &mut *state;
        saved_notice::publish_completed(completed, noticed_save_revision, || {
            capture
                .as_ref()
                .ok_or_else(|| ReplayRuntimeError::new("the game session has no capture runtime"))?
                .publish_replay_saved_notice()
                .map_err(|error| ReplayRuntimeError::new(error.to_string()))
        })
    }

    /// Start the daemon-owned export consumer independently from desktop UI
    /// polling. The factories are injected by the service so this coordinator
    /// does not own storage paths or Vulkan device selection.
    ///
    /// The pipeline is created lazily from the first exported frame because a
    /// game's actual swapchain dimensions are not known at launch time.
    ///
    /// # Errors
    ///
    /// Returns an error when validation does not authorize activation, the
    /// session no longer requests Replay, capture is absent, another pump is
    /// active, or the worker thread cannot be created safely.
    pub fn start_replay_export_pump<P, B>(
        &self,
        settings: ReplaySettings,
        readiness: ReplayBackendReadiness,
        pipeline_factory: P,
        backend_factory: B,
    ) -> Result<(), ReplayRuntimeError>
    where
        P: Fn(&DmaBufReplayFrame) -> Result<ReplayHardwarePipeline, String> + Send + Sync + 'static,
        B: Fn(&DmaBufReplayFrame) -> Result<Box<dyn HardwareEncoderBackend>, String>
            + Send
            + Sync
            + 'static,
    {
        crate::replay_runtime::validate_production_readiness(readiness)?;
        let endpoint = {
            let state = lock_unpoisoned(&self.inner.state);
            validate_replay_session_state(&state)?;
            if state.replay_pump.is_some() {
                return Err(ReplayRuntimeError::new(
                    "an Instant Replay export worker is already active",
                ));
            }
            state
                .capture
                .as_ref()
                .ok_or_else(|| {
                    ReplayRuntimeError::new(
                        "Instant Replay requires the game frame-transfer runtime",
                    )
                })?
                .replay_export_endpoint()
        };
        self.start_replay_frame_pump(
            settings,
            readiness,
            Arc::new(endpoint),
            pipeline_factory,
            backend_factory,
        )
    }

    pub(crate) fn start_replay_frame_pump<P, B>(
        &self,
        settings: ReplaySettings,
        readiness: ReplayBackendReadiness,
        source: Arc<dyn ReplayFrameSource>,
        pipeline_factory: P,
        backend_factory: B,
    ) -> Result<(), ReplayRuntimeError>
    where
        P: Fn(&DmaBufReplayFrame) -> Result<ReplayHardwarePipeline, String> + Send + Sync + 'static,
        B: Fn(&DmaBufReplayFrame) -> Result<Box<dyn HardwareEncoderBackend>, String>
            + Send
            + Sync
            + 'static,
    {
        crate::replay_runtime::validate_production_readiness(readiness)?;
        {
            let state = lock_unpoisoned(&self.inner.state);
            validate_replay_session_state(&state)?;
            if state.replay_pump.is_some() {
                return Err(ReplayRuntimeError::new(
                    "an Instant Replay export worker is already active",
                ));
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_source = Arc::clone(&source);
        let weak_inner = Arc::downgrade(&self.inner);
        let replay = self.inner.replay.clone();
        let pipeline_factory = Arc::new(pipeline_factory);
        let backend_factory = Arc::new(backend_factory);
        let worker = thread::Builder::new()
            .name("redunar-replay-export".to_owned())
            .spawn(move || {
                run_replay_export_pump(
                    worker_stop,
                    worker_source,
                    replay,
                    weak_inner,
                    settings,
                    readiness,
                    pipeline_factory,
                    backend_factory,
                )
            })
            .map_err(|error| {
                ReplayRuntimeError::new(format!("could not start Replay export worker: {error}"))
            })?;
        let mut state = lock_unpoisoned(&self.inner.state);
        // The session may have ended while the OS created the thread. Refuse
        // attachment and synchronously stop the worker in that rare race.
        if validate_replay_session_state(&state).is_err() || state.replay_pump.is_some() {
            stop.store(true, Ordering::Release);
            source.wake();
            drop(state);
            let _ = worker.join();
            return Err(ReplayRuntimeError::new(
                "the game session ended before Replay could start",
            ));
        }
        state.replay_pump = Some(ReplayExportPump {
            stop,
            source,
            worker: Some(worker),
        });
        Ok(())
    }

    /// Import one validated exported Vulkan frame and submit it to the
    /// daemon-owned hardware pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error when no capture export matches `sequence`, the
    /// producer FD cannot be duplicated, or the encoder rejects the frame.
    pub fn submit_replay_export(&self, sequence: u64) -> Result<(), ReplayRuntimeError> {
        let frame = lock_unpoisoned(&self.inner.state)
            .capture
            .as_ref()
            .ok_or_else(|| ReplayRuntimeError::new("the game session has no capture runtime"))?
            .import_replay_export(sequence)
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?;
        let result = self.inner.replay.submit_frame(sequence, frame);
        let mut state = lock_unpoisoned(&self.inner.state);
        collect_and_release_replay_exports(&mut state, &self.inner.replay);
        result
    }

    /// Publish a launch only after the shell-free child spawn succeeded.
    /// Replacing a live session is refused so two UI windows cannot acquire
    /// unrelated feature lifecycles for different games.
    ///
    /// # Errors
    ///
    /// Returns [`GameSessionError`] when another game session is live.
    pub fn begin(
        &self,
        request: GameSessionRequest,
        capture: Option<CaptureSessionHandle>,
        replay: ReplayRuntimeStatus,
    ) -> Result<(), GameSessionError> {
        // Flush any clip that committed after the end-of-session attribution
        // pass (a save finishing during shutdown) while the previous game
        // name is still published, so it cannot be re-attributed to the
        // session that is about to start.
        self.flush_completed_clip_attribution();
        if replay.backend_readiness.validation_allowed() {
            self.inner
                .replay
                .configure_validation_candidate(replay.settings);
        } else {
            self.inner.replay.configure_unavailable(replay.settings);
        }
        let replay_status = self.inner.replay.status();
        let mut state = lock_unpoisoned(&self.inner.state);
        if !matches!(
            state.status.phase,
            GameSessionPhase::Idle | GameSessionPhase::Ended
        ) {
            return Err(GameSessionError::new(
                "another daemon-owned game session is already active",
            ));
        }
        let capture_attached = capture.is_some();
        state.request = Some(request.clone());
        state.capture = capture;
        state.noticed_save_revision = replay_status.completed_save_revision;
        state.recorded_save_revision = replay_status.completed_save_revision;
        state.pending_replay_releases.clear();
        state.status = GameSessionStatus {
            revision: state.status.revision.saturating_add(1),
            phase: GameSessionPhase::Launching,
            game_id: request.game_id,
            game_name: Some(request.game_name),
            metrics: requested_state(request.metrics.is_enabled(), capture_attached),
            overlay: requested_state(request.overlay.is_enabled(), capture_attached),
            replay: replay_component(replay_status, request.replay.is_enabled()),
            failure: None,
        };
        Ok(())
    }

    /// Mark a shell-free launch as running even when no optional capture
    /// producer was requested.
    pub fn mark_running(&self) {
        let mut state = lock_unpoisoned(&self.inner.state);
        if state.status.phase == GameSessionPhase::Launching {
            state.status.phase = GameSessionPhase::Running;
            bump(&mut state.status);
        }
    }

    #[must_use]
    pub fn capture_snapshot(&self) -> Option<Arc<crate::CaptureSnapshot>> {
        let mut state = lock_unpoisoned(&self.inner.state);
        let snapshot = state.capture.as_ref().map(CaptureSessionHandle::snapshot)?;
        apply_capture_snapshot(&mut state, &snapshot);
        Some(snapshot)
    }

    #[must_use]
    pub fn steam_activation_state(&self) -> Option<SteamActivationState> {
        lock_unpoisoned(&self.inner.state)
            .capture
            .as_ref()
            .and_then(CaptureSessionHandle::steam_activation_state)
    }

    /// # Errors
    ///
    /// Returns [`CaptureSessionError`] when no capture is attached or the
    /// bounded overlay telemetry update fails.
    pub fn update_overlay_hardware(
        &self,
        snapshot: Option<&SystemSnapshot>,
    ) -> Result<(), CaptureSessionError> {
        lock_unpoisoned(&self.inner.state)
            .capture
            .as_ref()
            .ok_or_else(|| {
                CaptureSessionError::owned("the game session has no capture runtime".to_owned())
            })?
            .update_overlay_hardware(snapshot)
    }

    /// Update the active overlay appearance without restarting the game.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureSessionError`] when no capture is attached or the update fails.
    pub fn update_overlay_config(
        &self,
        profile: &redunar_core::EffectiveGameProfile,
    ) -> Result<(), CaptureSessionError> {
        let mut state = lock_unpoisoned(&self.inner.state);
        if !matches!(
            state.status.phase,
            GameSessionPhase::Launching | GameSessionPhase::Running
        ) {
            return Err(CaptureSessionError::owned(
                "the game session has ended".to_owned(),
            ));
        }
        let capture = state.capture.as_ref().ok_or_else(|| {
            CaptureSessionError::owned("the game session has no capture runtime".to_owned())
        })?;
        if matches!(
            capture.snapshot().phase,
            CapturePhase::Completed | CapturePhase::Failed
        ) {
            return Err(CaptureSessionError::owned(
                "the game's overlay runtime is no longer available".to_owned(),
            ));
        }
        capture.update_overlay_config(profile)?;
        let snapshot = capture.snapshot();
        if let Some(request) = state.request.as_mut() {
            request.overlay = profile.overlay_visible.into();
        }
        apply_capture_snapshot(&mut state, &snapshot);
        Ok(())
    }

    #[must_use]
    pub fn observe_launch_process(
        &self,
        process: &CaptureLaunchProcessState,
    ) -> Option<CaptureLaunchDisposition> {
        // The launch supervisor runs even while the desktop window is hidden.
        // Observe only committed saves; accepted requests and failures do not
        // emit success, and a telemetry write failure is retried next poll.
        let _ = self.publish_replay_saved_notice();
        self.flush_completed_clip_attribution();
        let mut state = lock_unpoisoned(&self.inner.state);
        let disposition = state
            .capture
            .as_mut()
            .map(|capture| capture.observe_launch_process(process));
        if let Some(capture) = state.capture.as_ref() {
            let snapshot = capture.snapshot();
            apply_capture_snapshot(&mut state, &snapshot);
        }
        disposition
    }

    /// Release only the fail-open capture/overlay transport while the game
    /// itself remains active (for example, a forwarded Steam launch whose
    /// capture activation expired).
    pub fn stop_capture(&self) {
        self.close_replay_menu();
        let mut replay_pump = lock_unpoisoned(&self.inner.state).replay_pump.take();
        if let Some(pump) = replay_pump.as_mut() {
            let _ = pump.stop_and_join();
        } else {
            let _ = self.inner.replay.shutdown();
        }
        let capture = {
            let mut state = lock_unpoisoned(&self.inner.state);
            collect_and_release_replay_exports(&mut state, &self.inner.replay);
            state.status.metrics = stop_optional(state.status.metrics);
            state.status.overlay = stop_optional(state.status.overlay);
            state.status.replay = stop_optional(state.status.replay);
            bump(&mut state.status);
            state.capture.take()
        };
        drop(capture);
    }

    /// Stop all optional session resources and publish a terminal state.
    ///
    /// # Errors
    ///
    /// Returns [`GameSessionError`] when Replay cleanup cannot complete.
    pub fn end(&self) -> Result<(), GameSessionError> {
        // One final attribution pass before the session stops so a save that
        // completed since the last supervisor poll is still labeled with this
        // game. Later drains fall back to session-history window matching.
        self.flush_completed_clip_attribution();
        self.close_replay_menu();
        let mut replay_pump = {
            let mut state = lock_unpoisoned(&self.inner.state);
            if matches!(
                state.status.phase,
                GameSessionPhase::Idle | GameSessionPhase::Ended
            ) {
                return Ok(());
            }
            state.status.phase = GameSessionPhase::Ending;
            state.status.metrics = stop_optional(state.status.metrics);
            state.status.overlay = stop_optional(state.status.overlay);
            state.status.replay = stop_optional(state.status.replay);
            bump(&mut state.status);
            state.replay_pump.take()
        };

        let replay_cleanup = replay_pump.as_mut().map_or_else(
            || self.inner.replay.shutdown(),
            ReplayExportPump::stop_and_join,
        );
        let capture = {
            let mut state = lock_unpoisoned(&self.inner.state);
            collect_and_release_replay_exports(&mut state, &self.inner.replay);
            state.capture.take()
        };
        drop(capture);

        let mut state = lock_unpoisoned(&self.inner.state);
        state.status.phase = GameSessionPhase::Ended;
        match replay_cleanup {
            Ok(()) => {
                state.status.failure = None;
                bump(&mut state.status);
                Ok(())
            }
            Err(error) => {
                state.status.failure =
                    Some(format!("Game-session cleanup did not complete: {error}"));
                bump(&mut state.status);
                Err(GameSessionError::new(error.to_string()))
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the worker keeps each injected ownership boundary explicit"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the worker thread owns these handles for its complete lifetime"
)]
#[allow(
    clippy::too_many_lines,
    reason = "the source-independent worker keeps video, optional audio, completion, and shutdown ordering explicit"
)]
fn run_replay_export_pump(
    stop: Arc<AtomicBool>,
    source: Arc<dyn ReplayFrameSource>,
    replay: ProductionReplayRuntime,
    coordinator: Weak<CoordinatorInner>,
    settings: ReplaySettings,
    readiness: ReplayBackendReadiness,
    pipeline_factory: ReplayPipelineFactory,
    backend_factory: ReplayBackendFactory,
) -> Result<(), ReplayRuntimeError> {
    let mut dimensions = None;
    // One recording failure is recoverable: the pump reports the failure,
    // releases producer ownership of every in-flight export, and then re-arms
    // a fresh validated pipeline after a bounded backoff. Failures therefore
    // cost a short window of disk history instead of the whole game session.
    let mut rearm_deadline: Option<Instant> = None;
    let mut rearm_backoff = REPLAY_REARM_BACKOFF;
    let mut consecutive_source_errors = 0_u32;
    let mut last_source_error_log: Option<Instant> = None;
    let mut audio_worker = None;
    let mut microphone_worker = None;
    let mut pending_releases = VecDeque::new();
    let mut diagnostic_timeline = crate::replay_logging::VideoTimeline::new(settings.frame_rate);
    while !stop.load(Ordering::Acquire) {
        diagnostic_timeline.tick(&replay);
        let _ = release_sequences(
            &mut pending_releases,
            source.take_rejected_exports(),
            |sequence| source.release(sequence),
        );
        if let Some(reason) = source.take_source_rejection() {
            crate::diagnostic_log::log(&format!("Replay event=source_rejected reason={reason:?}"));
            enter_replay_recovery(
                &replay,
                &coordinator,
                ReplayFailure::FrameSourceLost,
                &mut dimensions,
                &mut rearm_deadline,
                &mut rearm_backoff,
            );
        }
        // Retry ownership acknowledgements even when a full producer has no
        // free slot to export another frame. Never log sequence identifiers.
        let _ = release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
        // Retained unfinished GPU ownership is terminal. Do not construct
        // replacement encoders while its producer inputs remain leased.
        if replay.teardown_incomplete() {
            stop.store(true, Ordering::Release);
            break;
        }
        let frame = match source.wait_next(REPLAY_EXPORT_WAIT) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(_) => {
                consecutive_source_errors = consecutive_source_errors.saturating_add(1);
                // Malformed exports can arrive at frame rate. Log at most
                // once per interval and never echo a path from transport
                // diagnostics into the normal application log.
                if last_source_error_log.is_none_or(|at| at.elapsed() >= Duration::from_secs(5)) {
                    crate::log_op!(
                        "Redunar Replay: frame source failed ({consecutive_source_errors} consecutive)"
                    );
                    last_source_error_log = Some(Instant::now());
                }
                // A single stale or malformed export must not end recording;
                // a running recorder keeps its last frames and waits for the
                // next good one. Only a sustained source outage fails the
                // recorder over so the reported health reflects reality.
                let limit = if dimensions.is_some() {
                    REPLAY_SOURCE_ERROR_LIMIT
                } else {
                    1
                };
                if rearm_deadline.is_none() && consecutive_source_errors >= limit {
                    enter_replay_recovery(
                        &replay,
                        &coordinator,
                        ReplayFailure::FrameSourceLost,
                        &mut dimensions,
                        &mut rearm_deadline,
                        &mut rearm_backoff,
                    );
                }
                continue;
            }
        };
        consecutive_source_errors = 0;
        let (sequence, frame) = frame;
        if let Some(reason) = source.take_source_rejection() {
            // A rejection can wake wait_next while this descriptor is popped.
            // Do not submit it into the epoch that has just been invalidated.
            let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                source.release(completed)
            });
            crate::diagnostic_log::log(&format!("Replay event=source_rejected reason={reason:?}"));
            enter_replay_recovery(
                &replay,
                &coordinator,
                ReplayFailure::FrameSourceLost,
                &mut dimensions,
                &mut rearm_deadline,
                &mut rearm_backoff,
            );
            continue;
        }
        diagnostic_timeline.source(frame.timestamp_ns, frame.duration_ns);
        if let Some(deadline) = rearm_deadline {
            if Instant::now() < deadline {
                let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                    source.release(completed)
                });
                continue;
            }
            // The backoff elapsed, so this frame attempts the re-arm.
            rearm_deadline = None;
            // Retire the failed pipeline's finished GPU fences before the
            // replacement records, so the producer's staging contexts return.
            let _ = release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
        }
        let frame_dimensions = (frame.width, frame.height);
        if dimensions.is_none() {
            let Ok(pipeline) = pipeline_factory(&frame) else {
                crate::log_op!("Redunar Replay: hardware encoder pipeline creation failed");
                let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                    source.release(completed)
                });
                enter_replay_recovery(
                    &replay,
                    &coordinator,
                    ReplayFailure::EncoderFailed,
                    &mut dimensions,
                    &mut rearm_deadline,
                    &mut rearm_backoff,
                );
                continue;
            };
            if let Err(error) = replay.start_validated_pipeline(settings, pipeline, readiness) {
                crate::log_op!("Redunar Replay: recorder activation failed: {error}");
                let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                    source.release(completed)
                });
                enter_replay_recovery(
                    &replay,
                    &coordinator,
                    ReplayFailure::EncoderFailed,
                    &mut dimensions,
                    &mut rearm_deadline,
                    &mut rearm_backoff,
                );
                continue;
            }
            dimensions = Some(frame_dimensions);
            rearm_backoff = REPLAY_REARM_BACKOFF;
            crate::log_op!(
                "Redunar Replay: rolling recorder active at {}x{}",
                frame_dimensions.0,
                frame_dimensions.1
            );
            publish_replay_component(&coordinator, GameSessionComponentState::Active);
            // One default-output audio worker serves the whole game session.
            // It survives recorder re-arms, so re-arming never spawns a second
            // system-audio consumer.
            if audio_worker.is_none() {
                audio_worker =
                    start_game_audio_worker(Arc::clone(&stop), replay.clone(), frame.timestamp_ns);
            }
            if microphone_worker.is_none() {
                let microphone = coordinator
                    .upgrade()
                    .map(|inner| {
                        lock_unpoisoned(&inner.replay_preferences)
                            .microphone
                            .clone()
                    })
                    .unwrap_or(MicrophoneMode::Off);
                if !matches!(microphone, MicrophoneMode::Off) {
                    replay.enable_microphone_recording();
                    microphone_worker = start_microphone_audio_worker(
                        Arc::clone(&stop),
                        replay.clone(),
                        frame.timestamp_ns,
                        microphone,
                    );
                }
            }
        } else if dimensions != Some(frame_dimensions) {
            let Ok(replacement) = backend_factory(&frame) else {
                crate::log_op!(
                    "Redunar Replay: replacement encoder creation failed after a source resize"
                );
                let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                    source.release(completed)
                });
                let _ =
                    release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
                enter_replay_recovery(
                    &replay,
                    &coordinator,
                    ReplayFailure::EncoderFailed,
                    &mut dimensions,
                    &mut rearm_deadline,
                    &mut rearm_backoff,
                );
                continue;
            };
            if let Err(error) = replay.reset_backend(replacement) {
                crate::log_op!(
                    "Redunar Replay: recorder reset failed after a source resize: {error}"
                );
                let _ = release_sequences(&mut pending_releases, [sequence], |completed| {
                    source.release(completed)
                });
                let _ =
                    release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
                enter_replay_recovery(
                    &replay,
                    &coordinator,
                    ReplayFailure::EncoderFailed,
                    &mut dimensions,
                    &mut rearm_deadline,
                    &mut rearm_backoff,
                );
                continue;
            }
            dimensions = Some(frame_dimensions);
            rearm_backoff = REPLAY_REARM_BACKOFF;
            crate::log_op!(
                "Redunar Replay: rolling recorder reset for {}x{}",
                frame_dimensions.0,
                frame_dimensions.1
            );
            let _ = release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
        }
        let submitted_at = Instant::now();
        let submitted = replay.submit_frame(sequence, frame);
        diagnostic_timeline.submitted(submitted_at.elapsed(), submitted.is_err());
        if let Err(error) = submitted {
            crate::log_op!("Redunar Replay: hardware frame submission failed: {error}");
            // A failed submission can still have queued GPU reads. Only the
            // pipeline's completion/teardown proof authorizes buffer reuse.
            let _ = release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
            enter_replay_recovery(
                &replay,
                &coordinator,
                ReplayFailure::EncoderFailed,
                &mut dimensions,
                &mut rearm_deadline,
                &mut rearm_backoff,
            );
            continue;
        }
        if release_runtime_completions(source.as_ref(), &replay, &mut pending_releases).is_err() {
            enter_replay_recovery(
                &replay,
                &coordinator,
                ReplayFailure::EncoderFailed,
                &mut dimensions,
                &mut rearm_deadline,
                &mut rearm_backoff,
            );
        }
    }

    if let Some(worker) = audio_worker {
        let _ = worker.join();
    }
    if let Some(worker) = microphone_worker {
        let _ = worker.join();
    }
    diagnostic_timeline.finish(&replay);
    let shutdown = replay.shutdown();
    let releases = release_runtime_completions(source.as_ref(), &replay, &mut pending_releases);
    let queued_releases = source.drain_and_release();
    if releases.is_err() || queued_releases.is_err() {
        crate::diagnostic_log::log(
            "Replay event=shutdown_failed stage=export_release reason=acknowledgement_failed",
        );
    }
    let result = shutdown.and(releases).and(queued_releases);
    crate::diagnostic_log::log(shutdown_event(&result));
    result
}

fn shutdown_event(result: &Result<(), ReplayRuntimeError>) -> &'static str {
    if result.is_ok() {
        "Replay event=shutdown_complete"
    } else {
        "Replay event=shutdown_failed"
    }
}

/// Publish one recoverable recorder failure and schedule a re-arm attempt.
/// The exponential backoff bounds retry churn while a broken encoder or
/// storage path keeps rejecting new pipelines.
fn enter_replay_recovery(
    replay: &ProductionReplayRuntime,
    coordinator: &Weak<CoordinatorInner>,
    failure: ReplayFailure,
    dimensions: &mut Option<(u32, u32)>,
    rearm_deadline: &mut Option<Instant>,
    rearm_backoff: &mut Duration,
) {
    crate::diagnostic_log::log(&format!(
        "Replay event=recovery failure={failure:?} backoff_ms={}",
        rearm_backoff.as_millis()
    ));
    replay.fail(failure);
    publish_replay_component(coordinator, GameSessionComponentState::Failed);
    *dimensions = None;
    *rearm_deadline = Some(Instant::now() + *rearm_backoff);
    *rearm_backoff = rearm_backoff.mul_f32(2.0).min(REPLAY_REARM_BACKOFF_MAX);
}

fn start_microphone_audio_worker(
    stop: Arc<AtomicBool>,
    replay: ProductionReplayRuntime,
    first_frame_timestamp_ns: u64,
    mode: MicrophoneMode,
) -> Option<JoinHandle<()>> {
    let timeline_started = Instant::now();
    let timeline_timestamp_ns =
        redunar_capture_audio::monotonic_now_ns().unwrap_or(first_frame_timestamp_ns);
    thread::Builder::new()
        .name("redunar-replay-microphone".to_owned())
        .spawn(move || {
            run_microphone_audio_worker(
                &stop,
                &replay,
                timeline_timestamp_ns,
                timeline_started,
                mode,
            )
        })
        .map_err(|error| {
            crate::log_op!("Redunar Replay: could not start microphone worker: {error}")
        })
        .ok()
}

fn run_microphone_audio_worker(
    stop: &Arc<AtomicBool>,
    replay: &ProductionReplayRuntime,
    timeline_timestamp_ns: u64,
    timeline_started: Instant,
    mode: MicrophoneMode,
) {
    while !stop.load(Ordering::Acquire) {
        let sources = redunar_capture_audio::discover_microphone_audio_sources();
        let source = match &mode {
            MicrophoneMode::DefaultInput => sources.first().cloned(),
            MicrophoneMode::SelectedInput(name) => sources
                .into_iter()
                .find(|candidate| candidate.name() == name),
            MicrophoneMode::Off => None,
        };
        let Some(source) = source else {
            crate::log_op!("Redunar Replay: microphone input unavailable; retrying");
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        let Ok(mut capture) = redunar_capture_audio::MicrophoneAudioCapture::start(
            &source,
            timeline_timestamp_ns,
            timeline_started,
        ) else {
            crate::log_op!(
                "Redunar Replay: microphone capture unavailable via {}; retrying",
                source.backend_name()
            );
            thread::sleep(Duration::from_millis(250));
            continue;
        };
        crate::log_op!(
            "Redunar Replay: microphone capture active via {}",
            source.backend_name()
        );
        while !stop.load(Ordering::Acquire) {
            match capture.next_packet(Duration::from_millis(100)) {
                Ok(Some(packet)) => {
                    let _ = replay.submit_microphone_audio(packet);
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn start_game_audio_worker(
    stop: Arc<AtomicBool>,
    replay: ProductionReplayRuntime,
    first_frame_timestamp_ns: u64,
) -> Option<JoinHandle<()>> {
    let timeline_started = Instant::now();
    // A frame may wait for encoder initialization before this worker starts.
    // Both tracks use CLOCK_MONOTONIC, so anchor audio to the current clock,
    // not to that older frame's presentation time.
    let timeline_timestamp_ns =
        redunar_capture_audio::monotonic_now_ns().unwrap_or(first_frame_timestamp_ns);
    let startup_lag_ms = timeline_timestamp_ns.saturating_sub(first_frame_timestamp_ns) / 1_000_000;
    if startup_lag_ms > 250 {
        crate::log_op!("Redunar Replay: audio startup lag {startup_lag_ms} ms");
    }
    thread::Builder::new()
        .name("redunar-replay-audio".to_owned())
        .spawn(move || {
            run_game_audio_worker(&stop, &replay, timeline_timestamp_ns, timeline_started);
        })
        .map_err(|error| {
            crate::log_op!("Redunar Replay: could not start game audio worker: {error}");
        })
        .ok()
}

fn run_game_audio_worker(
    stop: &Arc<AtomicBool>,
    replay: &ProductionReplayRuntime,
    timeline_timestamp_ns: u64,
    timeline_started: Instant,
) {
    // Keep the worker alive and reconnect after route or sound-server failures
    // so audio does not silently disappear for the rest of the session.
    let mut backend_attempt = 0_usize;
    let (route_sender, route_receiver) = mpsc::sync_channel(1);
    let route_stop = Arc::clone(stop);
    let route_watcher = thread::Builder::new()
        .name("redunar-audio-route".to_owned())
        .spawn(move || watch_game_audio_route(&route_stop, &route_sender));
    while !stop.load(Ordering::Acquire) {
        let Ok(sources) = redunar_capture_audio::discover_system_audio_sources() else {
            // Sound-server diagnostics can include user-named devices.
            crate::log_op!("Redunar Replay: game audio discovery failed; retrying");
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        let source = sources[backend_attempt % sources.len()].clone();
        let Ok(mut capture) = redunar_capture_audio::SystemAudioCapture::start(
            &source,
            timeline_timestamp_ns,
            timeline_started,
        ) else {
            crate::log_op!(
                "Redunar Replay: game audio capture unavailable via {}; retrying",
                source.backend_name()
            );
            backend_attempt = backend_attempt.saturating_add(1);
            thread::sleep(Duration::from_secs(1));
            continue;
        };
        crate::log_op!(
            "Redunar Replay: audio capture active from the default output via {}",
            source.backend_name()
        );
        let mut reconnect = false;
        let mut backend_failed = false;
        let mut recorder_pause_logged = false;
        let mut last_packet_at = Instant::now();
        let mut received_packet = false;
        let mut audio_timeline = crate::replay_logging::AudioTimeline::new();
        while !stop.load(Ordering::Acquire) {
            match capture.next_packet(Duration::from_millis(100)) {
                Ok(Some(packet)) => {
                    audio_timeline.packet(packet.timestamp_ns, packet.duration_ns);
                    if !received_packet {
                        backend_attempt = 0;
                        received_packet = true;
                    }
                    last_packet_at = Instant::now();
                    if let Err(_error) = replay.submit_audio(packet) {
                        if matches!(
                            replay.status().phase,
                            ReplayPhase::Buffering | ReplayPhase::Saving
                        ) {
                            crate::log_op!(
                                "Redunar Replay: game audio buffering stopped; retrying"
                            );
                            reconnect = true;
                            break;
                        }
                        // The recorder itself is stopped (failed, re-arming,
                        // or inactive), which is not a broken audio
                        // transport. Keep this audio consumer attached and
                        // drop packets until the pump re-arms a pipeline;
                        // reconnecting here would respawn the capture helper
                        // about once per second for the rest of the session.
                        if !recorder_pause_logged {
                            crate::log_op!("Redunar Replay: game audio paused with the recorder");
                            recorder_pause_logged = true;
                        }
                    } else {
                        recorder_pause_logged = false;
                    }
                }
                Ok(None) => {
                    if last_packet_at.elapsed() > AUDIO_CAPTURE_STALL_TIMEOUT {
                        crate::log_op!(
                            "Redunar Replay: audio capture stopped producing samples; reconnecting"
                        );
                        reconnect = true;
                        backend_failed = true;
                        break;
                    }
                }
                Err(_) => {
                    crate::log_op!("Redunar Replay: game audio capture stopped; retrying");
                    reconnect = true;
                    backend_failed = true;
                    break;
                }
            }
            if game_audio_route_changed(&route_receiver, &source) {
                crate::log_op!(
                    "Redunar Replay: audio output changed on {}; reconnecting",
                    source.backend_name()
                );
                reconnect = true;
                break;
            }
        }
        if reconnect {
            // Try the next discovered backend after a transport failure. A
            // default-device change reconnects the same backend instead.
            if backend_failed {
                backend_attempt = backend_attempt.saturating_add(1);
            }
            thread::sleep(AUDIO_RECONNECT_DELAY);
        }
    }
    if let Ok(route_watcher) = route_watcher {
        let _ = route_watcher.join();
    }
}

fn game_audio_route_changed(
    receiver: &Receiver<Vec<redunar_capture_audio::AudioCaptureSource>>,
    source: &redunar_capture_audio::AudioCaptureSource,
) -> bool {
    receiver.try_recv().ok().is_some_and(|updated| {
        // Discovery can temporarily omit one backend under load. A failed
        // lookup is not evidence that the current sink changed; the active
        // capture's stall/error path handles a genuinely removed backend.
        updated
            .iter()
            .find(|candidate| candidate.backend_name() == source.backend_name())
            .is_some_and(|candidate| candidate != source)
    })
}

fn watch_game_audio_route(
    stop: &AtomicBool,
    sender: &SyncSender<Vec<redunar_capture_audio::AudioCaptureSource>>,
) {
    while !stop.load(Ordering::Acquire) {
        let next_check = Instant::now() + AUDIO_ROUTE_CHECK_INTERVAL;
        while Instant::now() < next_check && !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(100));
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let started = Instant::now();
        if let Ok(sources) = redunar_capture_audio::discover_system_audio_sources() {
            let _ = sender.try_send(sources);
        }
        if started.elapsed() > Duration::from_millis(250) {
            crate::log_op!(
                "Redunar Replay: audio route check took {} ms",
                started.elapsed().as_millis()
            );
        }
    }
}

fn release_runtime_completions(
    source: &dyn ReplayFrameSource,
    replay: &ProductionReplayRuntime,
    pending: &mut VecDeque<u64>,
) -> Result<(), ReplayRuntimeError> {
    release_sequences(pending, replay.take_completed_exports(), |sequence| {
        source.release(sequence)
    })
}

fn release_sequences(
    pending: &mut VecDeque<u64>,
    completed: impl IntoIterator<Item = u64>,
    mut release: impl FnMut(u64) -> Result<(), ReplayRuntimeError>,
) -> Result<(), ReplayRuntimeError> {
    // The producer's bounded slot pool limits outstanding acknowledgements.
    // Keep every failure for the next pump turn instead of discarding the
    // remainder when one acknowledgement fails.
    pending.extend(completed);
    let attempts = pending.len();
    let mut first_error = None;
    for _ in 0..attempts {
        let sequence = pending.pop_front().expect("attempt count is bounded");
        if let Err(error) = release(sequence) {
            pending.push_back(sequence);
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn publish_replay_component(
    coordinator: &Weak<CoordinatorInner>,
    component: GameSessionComponentState,
) {
    let Some(coordinator) = coordinator.upgrade() else {
        return;
    };
    let mut state = lock_unpoisoned(&coordinator.state);
    if matches!(
        state.status.phase,
        GameSessionPhase::Launching | GameSessionPhase::Running
    ) && state.status.replay != component
    {
        state.status.replay = component;
        bump(&mut state.status);
    }
}

fn validate_replay_session_state(state: &CoordinatorState) -> Result<(), ReplayRuntimeError> {
    if !matches!(
        state.status.phase,
        GameSessionPhase::Launching | GameSessionPhase::Running
    ) {
        return Err(ReplayRuntimeError::new(
            "Instant Replay requires an active game session",
        ));
    }
    if !state
        .request
        .as_ref()
        .is_some_and(|request| request.replay.is_enabled())
    {
        return Err(ReplayRuntimeError::new(
            "Instant Replay was not requested for this game session",
        ));
    }
    if state.capture.is_none() {
        return Err(ReplayRuntimeError::new(
            "Instant Replay requires the game frame-transfer runtime",
        ));
    }
    Ok(())
}

fn collect_and_release_replay_exports(
    state: &mut CoordinatorState,
    replay: &ProductionReplayRuntime,
) {
    for sequence in replay.take_completed_exports() {
        if !state.pending_replay_releases.contains(&sequence) {
            state.pending_replay_releases.push_back(sequence);
        }
    }
    let Some(capture) = state.capture.as_ref() else {
        return;
    };
    while let Some(sequence) = state.pending_replay_releases.front().copied() {
        if capture.release_replay_export(sequence).is_err() {
            break;
        }
        state.pending_replay_releases.pop_front();
    }
}

fn requested_state(requested: bool, available: bool) -> GameSessionComponentState {
    match (requested, available) {
        (false, _) => GameSessionComponentState::Disabled,
        (true, true) => GameSessionComponentState::Pending,
        (true, false) => GameSessionComponentState::Unavailable,
    }
}

fn replay_component(status: ReplayRuntimeStatus, requested: bool) -> GameSessionComponentState {
    if !requested {
        return GameSessionComponentState::Disabled;
    }
    match status.phase {
        ReplayPhase::Unavailable => GameSessionComponentState::Unavailable,
        ReplayPhase::Inactive => GameSessionComponentState::Pending,
        ReplayPhase::Buffering | ReplayPhase::Saving => GameSessionComponentState::Active,
        ReplayPhase::Failed => GameSessionComponentState::Failed,
    }
}

fn replay_menu_has_render_target(state: &CoordinatorState) -> bool {
    state.status.phase == GameSessionPhase::Running
        && state.capture.as_ref().is_some_and(|capture| {
            let snapshot = capture.snapshot();
            snapshot.phase == CapturePhase::Capturing
                && matches!(
                    snapshot.capture_api,
                    Some(CaptureApi::Vulkan | CaptureApi::OpenGl)
                )
        })
}

fn stop_optional(state: GameSessionComponentState) -> GameSessionComponentState {
    match state {
        GameSessionComponentState::Pending | GameSessionComponentState::Active => {
            GameSessionComponentState::Stopped
        }
        other => other,
    }
}

fn apply_capture_snapshot(state: &mut CoordinatorState, snapshot: &crate::CaptureSnapshot) {
    let request = state.request.as_ref();
    let metrics_requested = request.is_some_and(|request| request.metrics.is_enabled());
    let overlay_requested = request.is_some_and(|request| request.overlay.is_enabled());
    let metrics = if metrics_requested {
        match snapshot.phase {
            CapturePhase::Armed => GameSessionComponentState::Pending,
            CapturePhase::Capturing => GameSessionComponentState::Active,
            CapturePhase::Completed => GameSessionComponentState::Stopped,
            CapturePhase::Failed => GameSessionComponentState::Failed,
        }
    } else {
        GameSessionComponentState::Disabled
    };
    let overlay = if overlay_requested {
        match snapshot.overlay_status {
            None | Some(OverlayRuntimeStatus::Requested) => GameSessionComponentState::Pending,
            Some(OverlayRuntimeStatus::Active) => GameSessionComponentState::Active,
            Some(OverlayRuntimeStatus::Error(_)) => GameSessionComponentState::Failed,
        }
    } else {
        GameSessionComponentState::Disabled
    };
    let phase = if snapshot.phase == CapturePhase::Armed {
        GameSessionPhase::Launching
    } else {
        GameSessionPhase::Running
    };
    // Replay state flows from the replay component itself; the GL producer
    // now announces the same source/export contract as Vulkan, so the capture
    // API alone must not override an in-progress replay state.
    let replay = state.status.replay;
    if state.status.metrics != metrics
        || state.status.overlay != overlay
        || state.status.replay != replay
        || state.status.phase != phase
    {
        state.status.metrics = metrics;
        state.status.overlay = overlay;
        state.status.replay = replay;
        state.status.phase = phase;
        bump(&mut state.status);
    }
}

fn bump(status: &mut GameSessionStatus) {
    status.revision = status.revision.saturating_add(1);
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redunar_capture::{CaptureMessage, CaptureSessionId};
    use redunar_core::ReplaySettings;
    use std::time::Instant;

    fn request() -> GameSessionRequest {
        GameSessionRequest {
            game_id: GameId::new(9).ok(),
            game_name: "Fixture Game".to_owned(),
            metrics: GameSessionFeatureRequest::Enabled,
            overlay: GameSessionFeatureRequest::Enabled,
            replay: GameSessionFeatureRequest::Enabled,
        }
    }

    #[test]
    fn requested_components_share_one_launch_state() {
        let mut state = CoordinatorState::default();
        let request = request();
        state.request = Some(request.clone());
        state.status = GameSessionStatus {
            phase: GameSessionPhase::Launching,
            game_id: request.game_id,
            game_name: Some(request.game_name),
            metrics: requested_state(request.metrics.is_enabled(), true),
            overlay: requested_state(request.overlay.is_enabled(), true),
            replay: replay_component(
                ReplayRuntimeStatus::unavailable(ReplaySettings::default()),
                request.replay.is_enabled(),
            ),
            ..GameSessionStatus::default()
        };
        assert_eq!(state.status.metrics, GameSessionComponentState::Pending);
        assert_eq!(state.status.overlay, GameSessionComponentState::Pending);
        assert_eq!(state.status.replay, GameSessionComponentState::Unavailable);
    }

    #[test]
    fn opengl_capture_keeps_metrics_and_overlay_while_replay_keeps_its_component_state() {
        let request = request();
        let mut state = CoordinatorState {
            request: Some(request.clone()),
            status: GameSessionStatus {
                phase: GameSessionPhase::Launching,
                game_id: request.game_id,
                game_name: Some(request.game_name),
                metrics: GameSessionComponentState::Pending,
                overlay: GameSessionComponentState::Pending,
                replay: GameSessionComponentState::Pending,
                ..GameSessionStatus::default()
            },
            ..CoordinatorState::default()
        };
        let session_id = CaptureSessionId::new([8; 16]).expect("session ID");
        let mut capture = crate::CaptureSessionModel::new(session_id);
        capture
            .accept(CaptureMessage::Hello {
                session_id,
                process_id: 42,
                api: CaptureApi::OpenGl,
                producer_started_monotonic_ns: 1,
            })
            .expect("OpenGL producer");

        apply_capture_snapshot(&mut state, &capture.snapshot());

        assert_eq!(state.status.metrics, GameSessionComponentState::Active);
        assert_eq!(state.status.overlay, GameSessionComponentState::Pending);
        // The fixture started replay as Pending; the OpenGL backend no longer
        // forces it Unavailable, so the component state is preserved.
        assert_eq!(state.status.replay, GameSessionComponentState::Pending);
        assert_eq!(state.status.phase, GameSessionPhase::Running);

        capture
            .accept(CaptureMessage::OverlayStatus {
                session_id,
                status: OverlayRuntimeStatus::Requested,
            })
            .expect("OpenGL overlay requested");
        capture
            .accept(CaptureMessage::OverlayStatus {
                session_id,
                status: OverlayRuntimeStatus::Active,
            })
            .expect("OpenGL overlay active");
        apply_capture_snapshot(&mut state, &capture.snapshot());
        assert_eq!(state.status.overlay, GameSessionComponentState::Active);
    }

    #[test]
    fn replay_start_requires_active_requested_session_and_capture() {
        let idle = CoordinatorState::default();
        assert!(
            validate_replay_session_state(&idle)
                .expect_err("idle session")
                .to_string()
                .contains("active game session")
        );

        let without_capture = CoordinatorState {
            request: Some(request()),
            status: GameSessionStatus {
                phase: GameSessionPhase::Running,
                ..GameSessionStatus::default()
            },
            ..CoordinatorState::default()
        };
        assert!(
            validate_replay_session_state(&without_capture)
                .expect_err("missing capture")
                .to_string()
                .contains("frame-transfer")
        );

        let mut disabled = without_capture;
        disabled.request.as_mut().expect("request").replay = GameSessionFeatureRequest::Disabled;
        assert!(
            validate_replay_session_state(&disabled)
                .expect_err("Replay disabled")
                .to_string()
                .contains("not requested")
        );
    }

    #[test]
    fn replay_menu_refuses_mouse_capture_without_a_live_game_render_target() {
        assert!(!replay_menu_has_render_target(&CoordinatorState::default()));
    }

    #[test]
    fn lost_render_target_closes_an_open_menu_before_the_next_pointer_ping() {
        let coordinator = ProductionGameSessionCoordinator::new(
            std::env::temp_dir().join("redunar-menu-target-loss-unused"),
        );
        let now = Instant::now();
        lock_unpoisoned(&coordinator.inner.replay_menu).toggle(now);
        assert!(coordinator.replay_menu_is_visible());
        assert!(coordinator.replay_menu_watchdog(now));
        assert!(!coordinator.replay_menu_is_visible());
    }

    #[test]
    fn optional_cleanup_is_explicit_and_idempotent() {
        for state in [
            GameSessionComponentState::Pending,
            GameSessionComponentState::Active,
        ] {
            assert_eq!(stop_optional(state), GameSessionComponentState::Stopped);
            assert_eq!(
                stop_optional(stop_optional(state)),
                GameSessionComponentState::Stopped
            );
        }
        assert_eq!(
            stop_optional(GameSessionComponentState::Failed),
            GameSessionComponentState::Failed
        );
    }

    #[test]
    fn failed_final_acknowledgement_reports_failed_cleanup() {
        let mut pending = VecDeque::new();
        let releases = release_sequences(&mut pending, [1], |_| {
            Err(ReplayRuntimeError::new("fixture ACK failed"))
        });
        let result = Ok(()).and(releases);
        assert!(result.is_err());
        assert_eq!(pending, VecDeque::from([1]));
        assert_eq!(shutdown_event(&result), "Replay event=shutdown_failed");
    }

    #[test]
    fn replay_pump_startup_failure_is_component_local_and_stop_is_idempotent() {
        let (endpoint, acknowledgements) = ReplayExportEndpoint::new_for_test();
        let stop = Arc::new(AtomicBool::new(false));
        let runtime = ProductionReplayRuntime::unavailable(ReplaySettings::default());
        let worker = {
            let worker_stop = Arc::clone(&stop);
            let worker_endpoint = endpoint.clone();
            let worker_runtime = runtime.clone();
            thread::spawn(move || {
                run_replay_export_pump(
                    worker_stop,
                    Arc::new(worker_endpoint),
                    worker_runtime,
                    Weak::<CoordinatorInner>::new(),
                    ReplaySettings::default(),
                    ReplayBackendReadiness::fully_verified(),
                    Arc::new(|_| Err("fixture backend initialization failed".to_owned())),
                    Arc::new(|_| Err("replacement must not be requested".to_owned())),
                )
            })
        };
        let mut pump = ReplayExportPump {
            stop,
            source: Arc::new(endpoint.clone()),
            worker: Some(worker),
        };
        endpoint.enqueue_for_test(crate::capture_session::ReplayExportMetadata {
            sequence: 1,
            fd: std::fs::File::open("/dev/null")
                .expect("open fixture fd")
                .into(),
            source: redunar_capture::ReplaySourceCandidate {
                gpu_identity: None,
                width: 320,
                height: 240,
                pixel_format: redunar_capture::ReplayPixelFormat::Bgra8Unorm,
                target_frames_per_second: 60,
            },
            offset: 0,
            stride: 1_280,
            modifier: 0,
            timestamp_ns: 1,
            duration_ns: 16_666_667,
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.status().phase != ReplayPhase::Failed && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(runtime.status().phase, ReplayPhase::Failed);
        pump.stop_and_join().expect("stop failed pump");
        pump.stop_and_join().expect("second stop is idempotent");
        assert_eq!(*lock_unpoisoned(&acknowledgements), vec![1]);
    }

    #[test]
    fn rejection_wakes_an_idle_pump_and_clears_candidate_without_retrying_an_encoder() {
        let (endpoint, acknowledgements) = ReplayExportEndpoint::new_for_test();
        let stop = Arc::new(AtomicBool::new(false));
        let runtime = ProductionReplayRuntime::unavailable(ReplaySettings::default());
        runtime.configure_validation_candidate(ReplaySettings::default());
        let worker_stop = Arc::clone(&stop);
        let worker_endpoint = endpoint.clone();
        let worker_runtime = runtime.clone();
        let worker = thread::spawn(move || {
            run_replay_export_pump(
                worker_stop,
                Arc::new(worker_endpoint),
                worker_runtime,
                Weak::<CoordinatorInner>::new(),
                ReplaySettings::default(),
                ReplayBackendReadiness::fully_verified(),
                Arc::new(|_| panic!("no encoder without fresh export")),
                Arc::new(|_| panic!("no replacement without fresh export")),
            )
        });
        endpoint.reject_source_for_test(
            redunar_capture::ReplaySourceRejection::ExternalMemoryUnsupported,
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.status().phase != ReplayPhase::Failed && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(runtime.status().phase, ReplayPhase::Failed);
        assert_eq!(
            runtime.status().last_failure,
            Some(ReplayFailure::FrameSourceLost)
        );
        let mut pump = ReplayExportPump {
            stop,
            source: Arc::new(endpoint),
            worker: Some(worker),
        };
        pump.stop_and_join().unwrap();
        assert!(lock_unpoisoned(&acknowledgements).is_empty());
    }
}
