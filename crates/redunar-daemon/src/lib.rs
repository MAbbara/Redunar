use redunar_core::{GameLaunchConfig, HardwareProbe, ProbeError, SystemSnapshot};
use redunar_platform::{
    DesktopGameDiscovery, LinuxHardwareProbe, SteamGameDiscovery, capture_launch_support,
    game_launch_process_ownership,
};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

mod app_preferences;
mod capture;
mod capture_lifecycle;
mod capture_session;
pub mod diagnostic_log;
mod game_catalog;
mod game_session;
mod kms_replay_source;
mod module_state;
mod monitor;
mod replay;
mod replay_audio;
mod replay_clip_games;
mod replay_control;
mod replay_diagnostics;
mod replay_encoder;
mod replay_h264;
mod replay_inventory_cache;
mod replay_logging;
mod replay_matroska;
mod replay_menu;
mod replay_mp4;
mod replay_nvidia_diagnostics;
mod replay_packet_source;
mod replay_pipeline;
mod replay_preferences;
mod replay_ring;
mod replay_runtime;
mod replay_spool;
mod replay_spool_reader;
mod replay_store;
#[cfg(test)]
mod replay_streaming_tests;
mod replay_vulkan_video;
mod session_history;
mod session_ownership;
pub use crate::session_history::{SessionRecord, SessionTelemetrySample};

/// Current wall-clock timestamp used for local history records.
#[must_use]
pub fn session_started_unix() -> u64 {
    session_history::now_unix()
}

pub use app_preferences::{AppPreferences, AppPreferencesError};
pub use capture::{
    CaptureFrameMetrics, CapturePhase, CaptureSessionModel, CaptureSnapshot, capture_gpu,
};
pub use capture_lifecycle::{
    ARMED_STARTUP_GRACE, CaptureLaunchDisposition, CaptureLaunchProcessState, ProcessIdentity,
};
pub use capture_session::{
    CaptureRuntimeStatus, CaptureSessionConfig, CaptureSessionError, CaptureSessionHandle,
    SteamBridgeSetup, SteamBridgeSetupStatus,
};
pub use game_catalog::{AddGameRequest, GameCatalogError, ResolvedCatalogGame};
pub use game_session::{
    GameSessionComponentState, GameSessionError, GameSessionFeatureRequest, GameSessionPhase,
    GameSessionRequest, GameSessionStatus, ProductionGameSessionCoordinator,
};
pub use kms_replay_source::KmsReplayLiveDiagnostic;
pub use module_state::{AppModule, ModuleStatus, ModuleStatuses};
pub use monitor::{
    MonitorConfig, MonitorConfigError, MonitorDiagnostics, MonitorHandle, MonitorReader,
    MonitorSnapshot,
};
pub use redunar_capture::{OverlayFailureReason, OverlayRuntimeStatus, ReplaySourceRejection};
pub use redunar_capture_kms::{KmsCapturePlan, KmsProbe, KmsProbeBlocker};
pub use redunar_platform::{
    CaptureLaunchSupport, CaptureLaunchUnavailableReason, DEFAULT_STEAM_ACTIVATION_TTL,
    GameLaunchProcessOwnership, SteamActivationState, SteamAppId, SteamLaunchOptionsReason,
    SteamLaunchOptionsStatus,
};
pub use redunar_platform::{DiscoveredGame, GameDiscoveryError, GameDiscoverySource};
pub use replay::{
    ReplayBackendComponent, ReplayBackendReadiness, ReplayBudget, ReplayCapability, ReplayFailure,
    ReplayPhase, ReplayRecorderHealth, ReplayRuntimeStatus, ReplayStartError,
    ReplayStorageAccessError, ReplayTransitionError, ReplayUnavailableReason,
    begin_save as begin_replay_save, complete_save as complete_replay_save, fail as fail_replay,
    request_start as request_replay_start, stop as stop_replay,
};
pub use replay_audio::{
    ReplayAudioBuffer, ReplayAudioError, ReplayAudioSnapshot, ReplayAudioTracks,
};
pub use replay_diagnostics::{
    ReplayFoundationSelfTestError, ReplayFoundationSelfTestReport, ReplayFoundationSelfTestStage,
    run_replay_foundation_self_test,
};
pub use replay_encoder::{
    DmaBufImagePlane, DmaBufPlane, DmaBufReplayFrame, EncodedPacketBatch, HardwareEncodeOutput,
    HardwareEncoderApi, HardwareEncoderBackend, HardwareEncoderDeviceCandidate,
    HardwareEncoderProbe, HardwareEncoderProbeBlocker, ReplayEncoderError, ReplayPacketFlow,
    ReplayPacketFlowPhase, ReplayPacketFormat, ReplayVideoCodec, ReplayVideoStream,
};
pub use replay_h264::{H264ParameterSets, h264_annex_b_access_unit};
pub use replay_matroska::{
    MatroskaVideoError, MatroskaVideoSummary, write_audio_video_matroska,
    write_audio_video_matroska_tracks, write_video_only_matroska,
};
pub use replay_mp4::{
    Mp4VideoError, Mp4VideoSummary, write_audio_video_mp4, write_audio_video_mp4_tracks,
    write_video_only_mp4,
};
pub use replay_pipeline::{ReplayHardwarePipeline, ReplayPipelineError, ReplayPipelinePhase};
pub use replay_preferences::{
    MAX_REPLAY_SHORTCUTS, MicrophoneMode, ReplayOutputFormat, ReplayPreferences,
    ReplayPreferencesError, ReplayShortcutBinding,
};
pub use replay_ring::{
    EncodedReplayPacket, ReplayPacketError, ReplayPushOutcome, ReplayRing, ReplayRingError,
    ReplayRingStats,
};
pub use replay_runtime::{ProductionReplayRuntime, ReplayRuntimeError};
pub use replay_spool::{
    ReplayAssemblyJob, ReplaySegmentSpool, ReplaySpoolError, ReplaySpoolPhase, ReplaySpoolSegment,
    ReplaySpoolSnapshotPlan, ReplaySpoolStats, ReplaySpoolSubmitError,
};
pub use replay_store::{
    ReplayClipEntry, ReplayClipStore, ReplayStorageStatus, ReplayStoreError, StoredReplayClip,
};
pub use replay_vulkan_video::VulkanVideoH264Backend;

/// In-process application service used by the native desktop host.
#[derive(Clone)]
pub struct RedunarService {
    probe: LinuxHardwareProbe,
    state_directory: PathBuf,
    replay_home_directory: Option<PathBuf>,
    replay_control_path: Option<PathBuf>,
    allow_validation_candidate: bool,
    beta_access_at_start: bool,
    read_only: bool,
    runtime: Arc<ServiceRuntime>,
}

/// Read-only display evidence used to explain whether the selected 120 FPS
/// Replay mode has a compatible connected mode. It never opens a DRM device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayDisplayCapability {
    pub connected_outputs: usize,
    pub compatible_120_modes: usize,
    pub maximum_width: Option<u32>,
    pub maximum_height: Option<u32>,
    pub error: Option<String>,
}

#[derive(Default)]
struct ServiceRuntime {
    inventory: std::sync::Mutex<replay_inventory_cache::InventoryCache>,
    game_session: OnceLock<ProductionGameSessionCoordinator>,
    replay_control: OnceLock<replay_control::ReplayControlServer>,
    // Drop after the coordinator and listener, including all service clones.
    _ownership: Option<session_ownership::SessionLease>,
}

impl fmt::Debug for RedunarService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedunarService")
            .field("probe", &self.probe)
            .field("state_directory", &self.state_directory)
            .field("replay_home_directory", &self.replay_home_directory)
            .field("replay_control_path", &self.replay_control_path)
            .field(
                "allow_validation_candidate",
                &self.allow_validation_candidate,
            )
            .field("beta_access_at_start", &self.beta_access_at_start)
            .field("read_only", &self.read_only)
            .field(
                "game_session_initialized",
                &self.runtime.game_session.get().is_some(),
            )
            .finish()
    }
}

impl Default for RedunarService {
    fn default() -> Self {
        let state_directory = default_state_directory();
        let beta_access_at_start = app_preferences::load(&state_directory)
            .is_ok_and(|preferences| preferences.beta_access);
        Self {
            probe: LinuxHardwareProbe::default().with_nvidia_beta_enabled(beta_access_at_start),
            state_directory,
            replay_home_directory: absolute_home_directory(),
            replay_control_path: replay_control::replay_control_socket_path(),
            allow_validation_candidate: true,
            beta_access_at_start,
            read_only: false,
            runtime: Arc::new(ServiceRuntime::default()),
        }
    }
}

impl RedunarService {
    #[must_use]
    pub fn with_state_directory(state_directory: impl Into<PathBuf>) -> Self {
        let state_directory = state_directory.into();
        let beta_access_at_start = app_preferences::load(&state_directory)
            .is_ok_and(|preferences| preferences.beta_access);
        Self {
            probe: LinuxHardwareProbe::default().with_nvidia_beta_enabled(beta_access_at_start),
            state_directory,
            // Test/embedded callers that substitute daemon state must not
            // accidentally initialize the real user's Videos directory.
            replay_home_directory: None,
            // Isolated state must not claim the login session's Replay socket.
            replay_control_path: None,
            // Tests and embedded callers must not infer production Replay
            // readiness from whatever GPU happens to be installed on the host.
            allow_validation_candidate: false,
            beta_access_at_start,
            read_only: false,
            runtime: Arc::new(ServiceRuntime::default()),
        }
    }

    /// Isolated hardware probe with production Replay capability checks.
    /// Unlike the ordinary test constructor, this permits the real encoder
    /// path while retaining private state, output, and a private control socket
    /// that cannot collide with the login-session socket.
    #[must_use]
    pub fn for_isolated_replay_probe(state_directory: impl Into<PathBuf>) -> Self {
        let mut service = Self::with_state_directory(state_directory);
        service.allow_validation_candidate = true;
        service.replay_control_path = Some(service.state_directory.join("replay-control-v1.sock"));
        service
    }

    /// Construct the service used by the Tauri shell.
    /// # Errors
    /// Fails closed when process ownership cannot be safely established.
    pub fn for_tauri() -> std::io::Result<Self> {
        let lease = session_ownership::SessionLease::acquire(&default_state_directory())?;
        let mut service = Self::default();
        // Older installed builds do not take this lock. Retain their live
        // socket as an additional compatibility guard, never as acquisition.
        let legacy_owner = service
            .replay_control_path
            .as_ref()
            .is_some_and(|path| std::os::unix::net::UnixStream::connect(path).is_ok());
        service.read_only = lease.is_none() || legacy_owner;
        if service.read_only {
            service.replay_control_path = None;
        }
        service.runtime = Arc::new(ServiceRuntime {
            _ownership: lease,
            ..ServiceRuntime::default()
        });
        Ok(service)
    }

    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// A second Tauri process may show a read-only window while another app
    /// owns the login session. It must not compete for that app's Replay socket.
    #[must_use]
    pub fn for_tauri_read_only() -> Self {
        Self {
            replay_control_path: None,
            read_only: true,
            ..Self::default()
        }
    }

    /// Capture the current read-only hardware state.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError`] when a required hardware source cannot be read.
    pub fn snapshot(&self) -> Result<SystemSnapshot, ProbeError> {
        self.probe.snapshot()
    }

    /// Load daemon-owned application behavior. Missing state keeps normal
    /// desktop close semantics and performs no write.
    ///
    /// # Errors
    ///
    /// Returns [`AppPreferencesError`] for unsafe, malformed, or unreadable
    /// local state.
    pub fn app_preferences(&self) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::load(&self.state_directory)
    }

    /// Persist close-to-tray intent atomically without claiming that a desktop
    /// tray host is currently available.
    ///
    /// # Errors
    ///
    /// Returns [`AppPreferencesError`] if private local state cannot be saved.
    pub fn set_close_to_tray_enabled(
        &self,
        enabled: bool,
    ) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::set_close_to_tray(&self.state_directory, enabled)
    }

    /// Persist whether Redunar should check signed release metadata
    /// automatically. Installation remains a separate user-confirmed action.
    ///
    /// # Errors
    ///
    /// Returns [`AppPreferencesError`] if private local state cannot be saved.
    pub fn set_automatic_updates_enabled(
        &self,
        enabled: bool,
    ) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::set_automatic_updates(&self.state_directory, enabled)
    }

    /// Persist the opt-in diagnostic logging switch. Enabling takes effect on
    /// the next app start; the running process keeps its current stream.
    ///
    /// # Errors
    ///
    /// Propagates preference persistence failures.
    pub fn set_diagnostic_log_enabled(
        &self,
        enabled: bool,
    ) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::set_diagnostic_log(&self.state_directory, enabled)
    }

    /// Persist voluntary access to beta features in installed builds. Feature
    /// owners still check their own native runtime capabilities before use.
    ///
    /// # Errors
    ///
    /// Propagates preference persistence failures.
    pub fn set_beta_access_enabled(
        &self,
        enabled: bool,
    ) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::set_beta_access(&self.state_directory, enabled)
    }

    /// Start the bounded diagnostic log tee when the saved preference enables
    /// it. Silent on failure: diagnostics must never block startup.
    pub fn start_diagnostic_log_if_enabled(&self) {
        if self.read_only {
            return;
        }
        if let Ok(preferences) = app_preferences::load(&self.state_directory)
            && preferences.diagnostic_log
        {
            diagnostic_log::start(&self.state_directory);
            diagnostic_log::log(&format!(
                "app event=start beta_active={} debug_active={} architecture={}",
                self.beta_access_at_start,
                diagnostic_log::enabled(),
                std::env::consts::ARCH,
            ));
            if self.allow_validation_candidate && diagnostic_log::enabled() {
                let probe = self.replay_hardware_encoder_probe();
                diagnostic_log::log(&format!(
                    "Replay event=render_candidates count={} accessible={} blockers={:?}",
                    probe.candidates().len(),
                    probe
                        .candidates()
                        .iter()
                        .filter(|candidate| candidate.is_accessible())
                        .count(),
                    probe.blockers(),
                ));
            }
        }
    }

    /// The bounded diagnostic log file path, for the settings folder action.
    #[must_use]
    pub fn diagnostic_log_path(&self) -> PathBuf {
        diagnostic_log::log_path(&self.state_directory)
    }

    /// Persist the last usable window size and maximized state.
    ///
    /// # Errors
    ///
    /// Returns [`AppPreferencesError`] when the dimensions are unsafe or the
    /// private preference file cannot be updated atomically.
    pub fn set_window_state(
        &self,
        width: i32,
        height: i32,
        maximized: bool,
    ) -> Result<AppPreferences, AppPreferencesError> {
        app_preferences::set_window_state(&self.state_directory, width, height, maximized)
    }

    /// Return the one daemon-owned running-game coordinator shared by every
    /// native window and background controller created from this service.
    #[must_use]
    pub fn game_session_coordinator(&self) -> ProductionGameSessionCoordinator {
        let coordinator = self
            .runtime
            .game_session
            .get_or_init(|| ProductionGameSessionCoordinator::new(self.state_directory.clone()))
            .clone();
        self.runtime.replay_control.get_or_init(|| {
            replay_control::ReplayControlServer::start(
                coordinator.clone(),
                self.replay_control_path.clone(),
            )
        });
        coordinator
    }

    /// Start the daemon-owned, read-only live monitor.
    ///
    /// Hardware paths are discovered once by the worker. Dropping the handle
    /// interrupts and joins that worker, so monitoring cannot outlive its
    /// owner or delay application shutdown for a full sampling interval.
    #[must_use]
    pub fn start_monitor(&self) -> MonitorHandle {
        MonitorHandle::start(MonitorConfig::default(), self.beta_access_at_start)
    }

    /// Start live monitoring with explicit sampling intervals.
    ///
    /// This is primarily useful for tests and future user-selectable low-rate
    /// monitoring. [`MonitorConfig`] enforces Redunar's minimum interval.
    #[must_use]
    pub fn start_monitor_with_config(&self, config: MonitorConfig) -> MonitorHandle {
        MonitorHandle::start(config, self.beta_access_at_start)
    }

    /// Report whether the required Vulkan capture layer is available beside
    /// the current executable and a private runtime directory exists. The
    /// optional OpenGL observer is discovered separately by the launch plan.
    #[must_use]
    pub fn capture_runtime_status(&self) -> CaptureRuntimeStatus {
        Self::raw_capture_runtime_status()
    }

    fn raw_capture_runtime_status() -> CaptureRuntimeStatus {
        CaptureSessionConfig::for_current_build().map_or_else(
            |error| CaptureRuntimeStatus::Unavailable(error.to_string()),
            |_| CaptureRuntimeStatus::Available,
        )
    }

    /// Sweep capture-session directories left by a previous Redunar instance
    /// that died before cleanup ran. The caller (the app shell) must have
    /// confirmed that no other live instance owns the backend session; the
    /// sweep never removes paths another uid owns. Returns the removed count
    /// for diagnostics; leftovers are never fatal to startup.
    #[must_use]
    pub fn sweep_stale_capture_sessions(&self) -> usize {
        CaptureSessionConfig::for_current_build()
            .map_or(0, |config| config.sweep_stale_session_directories())
    }

    /// Verify the packaged Steam bridge and return the exact Launch Options
    /// snippet for one typed app ID. This does not edit Steam configuration or
    /// make native Steam capture generally available.
    #[must_use]
    pub fn steam_bridge_setup_status(&self, app_id: SteamAppId) -> SteamBridgeSetupStatus {
        CaptureSessionConfig::for_current_build().map_or_else(
            |error| SteamBridgeSetupStatus::Unavailable(error.to_string()),
            |config| config.steam_bridge_setup_status(app_id),
        )
    }

    /// Report whether Redunar can safely attach its host-side Vulkan layer to
    /// this exact launch configuration.
    ///
    /// Flatpak launches currently cross a filesystem boundary that hides the
    /// private manifest, socket, and layer library. Native `steam -applaunch`
    /// commands also forward only arguments when Steam is already running, so
    /// their games cannot inherit the private per-launch environment. Both
    /// remain launchable without metrics or overlay until separately verified
    /// integrations exist.
    #[must_use]
    pub fn capture_available_for_launch(&self, launch: &GameLaunchConfig) -> bool {
        self.capture_support_for_launch(launch).is_supported()
            && matches!(
                self.capture_runtime_status(),
                CaptureRuntimeStatus::Available
            )
    }

    /// Explain whether the exact command can pass the private capture state to
    /// its game. Runtime installation is reported separately by
    /// [`Self::capture_runtime_status`].
    #[must_use]
    pub fn capture_support_for_launch(&self, launch: &GameLaunchConfig) -> CaptureLaunchSupport {
        capture_launch_support(&launch.executable, &launch.arguments)
    }

    /// Describe whether the spawned child is the game lifetime authority.
    /// A forwarded Steam helper exit confirms only command delivery.
    #[must_use]
    pub fn process_ownership_for_launch(
        &self,
        launch: &GameLaunchConfig,
    ) -> GameLaunchProcessOwnership {
        game_launch_process_ownership(&launch.executable, &launch.arguments)
    }

    /// Report the daemon-owned replay configuration and resource budget.
    /// Installed encoder names do not make replay available; the capability
    /// stays unavailable until an end-to-end backend is verified.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when the persisted replay settings cannot
    /// be loaded safely.
    pub fn replay_runtime_status(&self) -> Result<ReplayRuntimeStatus, GameCatalogError> {
        let settings = self.load_game_catalog()?.global_profile.replay;
        let runtime = self.game_session_coordinator().replay_runtime();
        if self.replay_validation_candidate_available() {
            runtime.configure_validation_candidate(settings);
        } else {
            runtime.configure_unavailable(settings);
        }
        Ok(runtime.status())
    }

    /// Start the daemon-owned hardware Replay export worker for the active
    /// requested game session. Encoder creation is deferred until the first
    /// exported frame supplies the real swapchain dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayRuntimeError`] when validation capability, settings,
    /// storage, or active-session ownership cannot be established.
    pub fn start_replay_validation_candidate(&self) -> Result<(), ReplayRuntimeError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?
            .global_profile
            .replay;
        let readiness = self.replay_backend_readiness();
        if !readiness.validation_allowed() {
            return Err(ReplayRuntimeError::new(
                "Instant Replay hardware validation is unavailable on this system",
            ));
        }
        let save_directory = self
            .replay_save_directory()
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?;
        let output_format = self
            .replay_preferences()
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?
            .output_format;
        let spool_directory = self.state_directory.join("replay-spool-v1");
        let pipeline_settings = settings;
        let pipeline_save_directory = save_directory.clone();
        let pipeline_spool_directory = spool_directory.clone();
        // Beta admission requires an accessible, identifiable NVIDIA device.
        // The first game export independently selects and matches its UUIDs.
        let allow_nvidia_beta = self.beta_access_at_start
            && self
                .replay_hardware_encoder_probe()
                .candidates()
                .iter()
                .any(|candidate| candidate.driver() == "nvidia" && candidate.is_accessible());
        let coordinator = self.game_session_coordinator();
        coordinator
            .replay_runtime()
            .set_output_format(output_format);
        let result = coordinator.start_replay_export_pump(
            settings,
            readiness,
            move |frame| {
                let backend = vulkan_replay_backend_with_nvidia_beta(
                    frame,
                    pipeline_settings,
                    allow_nvidia_beta,
                )?;
                let store = ReplayClipStore::open(
                    pipeline_save_directory.clone(),
                    ReplayBudget::from_settings(pipeline_settings),
                )
                .map_err(|error| error.to_string())?;
                let spool = ReplaySegmentSpool::open(&pipeline_spool_directory, pipeline_settings)
                    .map_err(|error| error.to_string())?;
                // Recovery validates crash-left segments, but a new game
                // session must never save history from an earlier producer.
                // Queue the epoch reset before the first new encoded packet.
                spool.reset_epoch().map_err(|error| error.to_string())?;
                ReplayHardwarePipeline::new(pipeline_settings, store, backend)
                    .map(|pipeline| pipeline.with_spool(spool))
                    .map_err(|error| error.to_string())
            },
            move |frame| vulkan_replay_backend_with_nvidia_beta(frame, settings, allow_nvidia_beta),
        );
        if result.is_err() {
            coordinator
                .replay_runtime()
                .fail(ReplayFailure::EncoderFailed);
        }
        result
    }

    /// Request a duration-selected save from the one daemon-owned Replay
    /// runtime. The call returns after dispatching asynchronous assembly.
    ///
    /// # Errors
    ///
    /// Returns an error when no verified recorder is currently buffering.
    pub fn save_replay(
        &self,
        duration: redunar_core::ReplayDuration,
    ) -> Result<(), ReplayRuntimeError> {
        self.game_session_coordinator().save_replay(duration)
    }

    /// Publish a presentation-only in-game confirmation for a completed save.
    /// Failure never changes the already committed clip.
    ///
    /// # Errors
    ///
    /// Returns an error if no game overlay session is active or its telemetry
    /// file cannot be updated.
    pub fn publish_replay_saved_notice(&self) -> Result<(), ReplayRuntimeError> {
        self.game_session_coordinator()
            .publish_replay_saved_notice()
    }

    /// Report which replay backend components have passed their current
    /// headless verification gates. This is read-only and never initializes a
    /// frame source, encoder, or replay store.
    #[must_use]
    pub fn replay_backend_readiness(&self) -> ReplayBackendReadiness {
        if self.replay_validation_candidate_available() {
            ReplayBackendReadiness::hardware_validation_candidate()
        } else {
            ReplayBackendReadiness::headless_foundation()
        }
    }

    /// Inspect local DRM render nodes as hardware-encoder candidates.
    /// This performs no encode and can never make Replay production-ready by
    /// itself; profile/codec support and DMA-BUF import still require a real
    /// generated-frame verification run.
    #[must_use]
    pub fn replay_hardware_encoder_probe(&self) -> HardwareEncoderProbe {
        HardwareEncoderProbe::local_with_nvidia_beta(self.beta_access_at_start)
    }

    /// Inspect connected DRM connector modes for the display-aware Replay
    /// selector. This is bounded metadata discovery only; it does not capture,
    /// modeset, authenticate, or claim that the encoder is production-ready.
    #[must_use]
    pub fn replay_display_capability(&self) -> ReplayDisplayCapability {
        match redunar_capture_kms::discover_outputs_at(Path::new("/")) {
            Ok(outputs) => {
                let mut compatible_120_modes = 0;
                let mut maximum_width: Option<u32> = None;
                let mut maximum_height: Option<u32> = None;
                for mode in outputs.iter().flat_map(|output| output.modes.iter()) {
                    maximum_width =
                        Some(maximum_width.map_or(mode.width, |value| value.max(mode.width)));
                    maximum_height =
                        Some(maximum_height.map_or(mode.height, |value| value.max(mode.height)));
                    if redunar_core::ReplayFrameRate::Fps120
                        .supports_dimensions(mode.width, mode.height)
                    {
                        compatible_120_modes += 1;
                    }
                }
                ReplayDisplayCapability {
                    connected_outputs: outputs.len(),
                    compatible_120_modes,
                    maximum_width,
                    maximum_height,
                    error: None,
                }
            }
            Err(error) => ReplayDisplayCapability {
                connected_outputs: 0,
                compatible_120_modes: 0,
                maximum_width: None,
                maximum_height: None,
                error: Some(error.to_string()),
            },
        }
    }

    /// Prepare the private clip store for backend integration tests. This does
    /// not start capture or encoding and does not change Replay capability.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when settings cannot be loaded or the
    /// dedicated replay directory cannot be initialized safely.
    pub fn prepare_replay_store(&self) -> Result<ReplayClipStore, ReplayStorageAccessError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?
            .global_profile
            .replay;
        ReplayClipStore::open(
            self.replay_save_directory()?,
            ReplayBudget::from_settings(settings),
        )
        .map_err(|error| ReplayStorageAccessError::store(&error))
    }

    /// Inspect the configured replay save path and quota without creating or
    /// modifying the private store.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when settings cannot be loaded or
    /// existing replay storage does not have Redunar's exact safe ownership
    /// state.
    pub fn replay_storage_status(&self) -> Result<ReplayStorageStatus, ReplayStorageAccessError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?
            .global_profile
            .replay;
        ReplayClipStore::inspect(
            self.replay_save_directory()?,
            ReplayBudget::from_settings(settings),
        )
        .map_err(|error| ReplayStorageAccessError::store(&error))
    }

    /// List saved Redunar replay clips newest first without initializing the
    /// store. An uninitialized store is represented by an empty inventory.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when settings or existing store
    /// ownership cannot be verified safely.
    pub fn replay_clip_inventory(&self) -> Result<Vec<ReplayClipEntry>, ReplayStorageAccessError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?
            .global_profile
            .replay;
        let budget = ReplayBudget::from_settings(settings);
        let directory = self.replay_save_directory()?;
        if std::fs::symlink_metadata(&directory)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(Vec::new());
        }
        // Validate the private store on every request, even a cache hit.
        let store = ReplayClipStore::open_existing(&directory, budget)
            .map_err(|error| ReplayStorageAccessError::inventory(&error))?;
        let stamp = replay_inventory_cache::InventoryStamp::read(&directory, &self.state_directory);
        if let Some(clips) = self
            .runtime
            .inventory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&stamp)
        {
            return Ok(clips);
        }
        let clips = self.attach_clip_games(
            store
                .inventory()
                .map_err(|error| ReplayStorageAccessError::inventory(&error))?,
        );
        if stamp == replay_inventory_cache::InventoryStamp::read(&directory, &self.state_directory)
        {
            self.runtime
                .inventory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .put(stamp, clips.clone());
        }
        Ok(clips)
    }

    /// Join each clip with the game that recorded it. The persisted ledger
    /// wins; clips saved before attribution existed (or after a ledger
    /// failure) fall back to the commit time encoded in the clip name
    /// matching exactly one recorded session window. Listing never fails
    /// because labeling evidence is missing or unreadable.
    fn attach_clip_games(&self, mut clips: Vec<ReplayClipEntry>) -> Vec<ReplayClipEntry> {
        let ledger = replay_clip_games::games(&self.state_directory);
        let sessions = session_history::load(&self.state_directory).unwrap_or_default();
        for clip in &mut clips {
            clip.game_name = replay_clip_games::resolve(&clip.file_name, &ledger, &sessions);
        }
        clips
    }

    /// Attribute one owned clip to a game name (for example, an export of a
    /// clip whose source was recorded by a game). Best-effort labeling;
    /// callers must not fail a feature because attribution could not persist.
    ///
    /// # Errors
    ///
    /// Returns an error when the clip name is not a Redunar-generated name or
    /// the private ledger cannot be rewritten.
    pub fn record_clip_game(
        &self,
        file_name: &str,
        game_name: &str,
    ) -> Result<(), ReplayStorageAccessError> {
        if !replay_store::is_owned_clip_name(std::ffi::OsStr::new(file_name)) {
            return Err(ReplayStorageAccessError::inventory(
                &"clip name is not a Redunar-generated name",
            ));
        }
        replay_clip_games::record(&self.state_directory, file_name, game_name)
            .map_err(|error| ReplayStorageAccessError::inventory(&error))
    }

    /// Delete exactly one clip identifier returned by
    /// [`Self::replay_clip_inventory`]. This never initializes a missing store
    /// and never accepts an arbitrary path.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when settings, store ownership, or
    /// the exact clip target cannot be verified safely.
    pub fn delete_replay_clip(
        &self,
        file_name: &str,
    ) -> Result<ReplayClipEntry, ReplayStorageAccessError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?
            .global_profile
            .replay;
        ReplayClipStore::open_existing(
            self.replay_save_directory()?,
            ReplayBudget::from_settings(settings),
        )
        .and_then(|store| store.delete_clip(file_name))
        .map_err(|error| ReplayStorageAccessError::delete(&error))
    }

    /// Archive one exact listed Replay clip below the configured store.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when settings, store ownership,
    /// or the exact clip target cannot be verified safely.
    pub fn archive_replay_clip(
        &self,
        file_name: &str,
    ) -> Result<PathBuf, ReplayStorageAccessError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?
            .global_profile
            .replay;
        ReplayClipStore::open_existing(
            self.replay_save_directory()?,
            ReplayBudget::from_settings(settings),
        )
        .and_then(|store| store.archive_clip(file_name))
        .map_err(|error| ReplayStorageAccessError::archive(&error))
    }

    /// Load daemon-owned Replay preferences. Missing state uses the Videos
    /// folder and the default planned save shortcut without writing anything.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] for malformed or unsafe local state.
    pub fn replay_preferences(&self) -> Result<ReplayPreferences, ReplayPreferencesError> {
        replay_preferences::load(&self.state_directory)
    }

    /// Whether the user has explicitly persisted Replay preferences. This is
    /// used only to avoid prompting for the default shortcut on first launch.
    #[must_use]
    pub fn replay_preferences_are_saved(&self) -> bool {
        replay_preferences::saved_preferences_exist(&self.state_directory)
    }

    /// Resolve the dedicated owned clip directory. Selection and resolution
    /// are read-only; only an explicit recorder/store operation creates it.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayStorageAccessError`] when preferences cannot be loaded.
    pub fn replay_save_directory(&self) -> Result<PathBuf, ReplayStorageAccessError> {
        let preferences = self
            .replay_preferences()
            .map_err(|error| ReplayStorageAccessError::preferences(&error))?;
        Ok(preferences
            .resolved_directory(self.replay_home_directory.as_deref(), &self.state_directory))
    }

    /// Select an optional parent for Redunar's fixed `Redunar Replays` child.
    /// This saves preference state only; it never creates, moves, or deletes
    /// clips or directories.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] for unsafe paths or persistence
    /// failures.
    pub fn set_replay_save_parent(
        &self,
        parent: Option<PathBuf>,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        replay_preferences::set_save_parent(&self.state_directory, parent)
    }

    /// Replace the bounded shortcut-to-duration bindings atomically. Redunar
    /// does not register global listeners while the production recorder and
    /// desktop shortcut integration are unavailable.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] for invalid, duplicate, empty, or
    /// excessive bindings and persistence failures.
    pub fn set_replay_save_shortcuts(
        &self,
        bindings: Vec<ReplayShortcutBinding>,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences = replay_preferences::set_save_shortcuts(&self.state_directory, bindings)?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    /// Replace the overlay toggle and duration-specific save shortcuts in one
    /// atomic preference update. No two actions may share a chord.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] when a shortcut is invalid,
    /// duplicated, unbounded, or cannot be persisted atomically.
    pub fn set_replay_hotkeys(
        &self,
        overlay_shortcut: String,
        bindings: Vec<ReplayShortcutBinding>,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences =
            replay_preferences::set_hotkeys(&self.state_directory, overlay_shortcut, bindings)?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    /// Update the manual Replay overlay shortcut and pointer-dismiss behavior
    /// while preserving every duration-specific direct-save binding.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] when the shortcut conflicts with a
    /// save binding or the preference cannot be persisted atomically.
    pub fn set_replay_overlay_behavior(
        &self,
        overlay_shortcut: String,
        close_on_outside_click: bool,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences = replay_preferences::set_overlay_behavior(
            &self.state_directory,
            overlay_shortcut,
            close_on_outside_click,
        )?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    /// Change only dismissal, preserving the current shortcut assignments.
    ///
    /// # Errors
    /// Returns an error if private preferences cannot be loaded or saved.
    pub fn set_replay_outside_click(
        &self,
        enabled: bool,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences = replay_preferences::set_outside_click(&self.state_directory, enabled)?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    /// Set the duration shown in the Replay save control when it opens.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] when the duration is unsupported or
    /// the private preferences cannot be persisted atomically.
    pub fn set_replay_initial_save_duration(
        &self,
        duration: redunar_core::ReplayDuration,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences =
            replay_preferences::set_initial_save_duration(&self.state_directory, duration)?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    /// Select the container used for future Replay saves and update the live
    /// recorder without restarting the active game session.
    ///
    /// # Errors
    ///
    /// Returns an error if the private Replay preferences cannot be loaded or
    /// replaced atomically.
    /// Set microphone capture mode for future Replay sessions.
    ///
    /// Microphone capture is Off by default; changing this setting never
    /// interrupts an active video or system-audio pipeline.
    pub fn set_replay_microphone(
        &self,
        microphone: MicrophoneMode,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences = replay_preferences::set_microphone(&self.state_directory, microphone)?;
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    ///
    /// # Errors
    ///
    /// Returns an error if preferences cannot be persisted or the active
    /// replay runtime cannot apply the selected format.
    pub fn set_replay_output_format(
        &self,
        output_format: ReplayOutputFormat,
    ) -> Result<ReplayPreferences, ReplayPreferencesError> {
        let preferences =
            replay_preferences::set_output_format(&self.state_directory, output_format)?;
        self.game_session_coordinator()
            .replay_runtime()
            .set_output_format(output_format);
        self.publish_replay_preferences(&preferences);
        Ok(preferences)
    }

    fn publish_replay_preferences(&self, preferences: &ReplayPreferences) {
        if let Some(coordinator) = self.runtime.game_session.get() {
            coordinator.update_replay_preferences(preferences.clone());
        }
    }

    /// Rolling history retained while Replay is enabled. Clip-length and
    /// shortcut changes select a suffix to save; they never discard older
    /// history that may still be requested during the same game session.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayPreferencesError`] when shortcut preferences are
    /// malformed and [`GameCatalogError`] when Replay settings are malformed.
    pub fn effective_replay_retention_duration(
        &self,
    ) -> Result<redunar_core::ReplayDuration, ReplayStorageAccessError> {
        self.load_game_catalog()
            .map_err(|error| ReplayStorageAccessError::catalog(&error))?;
        self.replay_preferences()
            .map_err(|error| ReplayStorageAccessError::preferences(&error))?;
        Ok(redunar_core::ReplayDuration::Seconds900)
    }

    /// Run the bounded generated-packet replay foundation self-test in one
    /// explicit diagnostic directory.
    ///
    /// This uses the persisted replay settings but does not capture frames,
    /// access a GPU, invoke an encoder, or change production readiness. The
    /// caller owns cleanup of the generated diagnostic clip.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayFoundationSelfTestError`] when settings cannot be loaded
    /// or any bounded packet-flow, mux, durable-store, or readback check fails.
    pub fn run_replay_foundation_self_test(
        &self,
        directory: impl Into<PathBuf>,
    ) -> Result<ReplayFoundationSelfTestReport, ReplayFoundationSelfTestError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayFoundationSelfTestError::settings(&error))?
            .global_profile
            .replay;
        replay_diagnostics::run_replay_foundation_self_test(directory, settings)
    }

    /// Start one daemon-owned local capture receiver. This prepares only a
    /// private socket and explicit-layer manifest; it does not launch a game.
    /// Direct launch plans also preload the optional adjacent OpenGL observer.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureSessionError`] if private runtime state, the layer
    /// library, socket binding, randomness, or worker creation fails.
    pub fn start_capture_session(
        &self,
        config: &CaptureSessionConfig,
    ) -> Result<CaptureSessionHandle, CaptureSessionError> {
        let statuses = self.module_statuses();
        let gates = capture_session::CaptureModuleGates {
            frame_metrics: statuses.frame_metrics.allows_runtime(),
            in_game_overlay: statuses.in_game_overlay.allows_runtime(),
            // Replay cannot request capture merely because the user wants it;
            // its complete backend capability must first become available.
            instant_replay: statuses.instant_replay.allows_runtime(),
        };
        if !gates.capture_requested() {
            return Err(CaptureSessionError::new(
                "frame metrics, in-game overlay, and instant replay are unavailable on this system",
            ));
        }
        CaptureSessionHandle::start(config, gates)
    }

    /// Return current runtime capability truth for the production features.
    #[must_use]
    pub fn module_statuses(&self) -> ModuleStatuses {
        module_state::permanent_core_statuses(&self.module_capabilities())
    }

    /// Return one effective app-wide module state.
    #[must_use]
    pub fn module_status(&self, module: AppModule) -> ModuleStatus {
        self.module_statuses().get(module).clone()
    }

    fn module_capabilities(&self) -> module_state::ModuleCapabilities {
        let capture = match Self::raw_capture_runtime_status() {
            CaptureRuntimeStatus::Available => module_state::ModuleCapability::Available,
            CaptureRuntimeStatus::Unavailable(reason) => {
                module_state::ModuleCapability::UnavailableOnSystem(reason)
            }
        };
        let replay = if self.replay_validation_candidate_available() {
            module_state::ModuleCapability::Available
        } else {
            module_state::ModuleCapability::PlannedUnavailable(
                "Instant Replay requires Vulkan game capture and an accessible hardware encoder. NVIDIA candidates require Beta access, one identifiable NVIDIA GPU, and a matching game device.".to_owned(),
            )
        };
        module_state::ModuleCapabilities {
            frame_metrics: capture.clone(),
            in_game_overlay: capture,
            instant_replay: replay,
        }
    }

    fn replay_validation_candidate_available(&self) -> bool {
        self.allow_validation_candidate
            && cfg!(target_arch = "x86_64")
            && matches!(
                Self::raw_capture_runtime_status(),
                CaptureRuntimeStatus::Available
            )
            && HardwareEncoderProbe::local_with_nvidia_beta(self.beta_access_at_start)
                .validation_allowed()
    }

    /// Load the bounded local, launcher-agnostic game catalog.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when stored state is unreadable or malformed.
    pub fn load_game_catalog(&self) -> Result<redunar_core::GameCatalog, GameCatalogError> {
        game_catalog::load(&self.state_directory)
    }

    /// Resolve the current global defaults and one game's explicit overrides.
    /// This is read-only and does not start capture or overlay work.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when stored state is invalid or the game
    /// no longer exists.
    pub fn effective_game_profile(
        &self,
        id: redunar_core::GameId,
    ) -> Result<redunar_core::EffectiveGameProfile, GameCatalogError> {
        let catalog = self.load_game_catalog()?;
        let game = catalog
            .games
            .iter()
            .find(|game| game.id == id)
            .ok_or_else(|| GameCatalogError::new("the selected game no longer exists"))?;
        Ok(game.profile.resolve(catalog.global_profile))
    }

    /// Resolve a detected process against local game identity and return an
    /// effective profile only for a unique, high-confidence match.
    ///
    /// Suggested filename matches and ambiguous identities intentionally carry
    /// no effective profile, so detection alone cannot activate a session feature.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when stored state is invalid.
    pub fn resolve_game_process(
        &self,
        process: &redunar_core::GameProcess,
    ) -> Result<ResolvedCatalogGame, GameCatalogError> {
        let mut resolved = game_catalog::resolve_process(&self.state_directory, process)?;
        if let redunar_core::GameResolution::Automatic(id) = resolved.resolution {
            resolved.effective_profile = Some(self.effective_game_profile(id)?);
        }
        Ok(resolved)
    }

    /// Add one direct executable to the local game catalog.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] for invalid, duplicate, or unsavable records.
    pub fn add_game(
        &self,
        request: AddGameRequest,
    ) -> Result<redunar_core::GameRecord, GameCatalogError> {
        game_catalog::add(&self.state_directory, request)
    }

    /// Atomically import a bounded set of direct executable records.
    ///
    /// Every entry is validated and canonicalized before the catalog is
    /// changed. A duplicate or invalid entry rejects the whole batch.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] for an empty or oversized batch, invalid
    /// launch data, duplicates, catalog capacity, or persistence failure.
    pub fn import_games(
        &self,
        requests: Vec<AddGameRequest>,
    ) -> Result<Vec<redunar_core::GameRecord>, GameCatalogError> {
        game_catalog::import(&self.state_directory, requests)
    }

    /// Discover local Steam installs and direct native XDG game entries.
    /// Discovery is bounded and read-only; it does not add, launch, or tune a
    /// game. Shared desktop launchers are left to their dedicated adapters so
    /// they cannot become an ambiguous process identity.
    ///
    /// # Errors
    ///
    /// Returns [`GameDiscoveryError`] when the local home or an existing Steam
    /// library cannot be inspected safely.
    pub fn discover_installed_games(&self) -> Result<Vec<DiscoveredGame>, GameDiscoveryError> {
        let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            GameDiscoveryError::new("HOME is unavailable for local game discovery")
        })?;
        let mut discovered = SteamGameDiscovery::new(home.clone()).discover()?;
        discovered.extend(DesktopGameDiscovery::from_environment(&home).discover()?);
        Ok(discovered)
    }

    /// Atomically add candidates which have a verified shell-free launch.
    /// Shared launchers are not used as process identity; their explicit Steam
    /// app identifiers remain the canonical matching evidence.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] if any candidate is not launchable, has
    /// invalid identity, duplicates a saved game, or cannot be persisted.
    pub fn import_discovered_games(
        &self,
        candidates: Vec<DiscoveredGame>,
    ) -> Result<Vec<redunar_core::GameRecord>, GameCatalogError> {
        let requests = candidates
            .into_iter()
            .map(|candidate| {
                let launch = candidate.launch.ok_or_else(|| {
                    GameCatalogError::new(
                        "a discovered game has no verified shell-free launch on this system",
                    )
                })?;
                Ok(AddGameRequest::external_launcher(
                    candidate.display_name,
                    launch,
                    candidate.match_rules,
                ))
            })
            .collect::<Result<Vec<_>, GameCatalogError>>()?;
        game_catalog::import(&self.state_directory, requests)
    }

    /// Atomically refresh saved launch commands found again by local
    /// discovery. Only a unique strong identity such as a Steam app ID may
    /// update a record; profiles and display names are preserved.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when a discovered launch is invalid or
    /// the updated catalog cannot be persisted safely.
    pub fn refresh_discovered_game_launches(
        &self,
        candidates: &[DiscoveredGame],
    ) -> Result<Vec<redunar_core::GameRecord>, GameCatalogError> {
        let requests = candidates
            .iter()
            .filter_map(|candidate| {
                candidate.launch.clone().map(|launch| {
                    AddGameRequest::external_launcher(
                        candidate.display_name.clone(),
                        launch,
                        candidate.match_rules.clone(),
                    )
                })
            })
            .collect();
        game_catalog::refresh_discovered_launches(&self.state_directory, requests)
    }

    /// Build an import candidate from an already conservatively detected game
    /// process only when its exact executable is a unique host executable.
    ///
    /// # Errors
    ///
    /// Returns [`GameDiscoveryError`] for stale, non-executable, shared
    /// launcher, Wine, or Proton process identities.
    pub fn candidate_from_game_process(
        &self,
        process: &redunar_core::GameProcess,
    ) -> Result<DiscoveredGame, GameDiscoveryError> {
        redunar_platform::candidate_from_process(process)
    }

    /// Replace one game's shell-free launch configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when the game or launch data is invalid.
    pub fn update_game_launch(
        &self,
        id: redunar_core::GameId,
        launch: redunar_core::GameLaunchConfig,
    ) -> Result<redunar_core::GameRecord, GameCatalogError> {
        game_catalog::update_launch(&self.state_directory, id, launch)
    }

    /// Persist explicit per-game inheritance or overrides. This records user
    /// intent only and does not change an active capture session by itself.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when the game is missing or state cannot be saved.
    pub fn update_game_profile(
        &self,
        id: redunar_core::GameId,
        profile: redunar_core::PerGameProfile,
    ) -> Result<redunar_core::GameRecord, GameCatalogError> {
        game_catalog::update_profile(&self.state_directory, id, profile)
    }

    /// Remove one user-owned local game record.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when the game is missing or state cannot be saved.
    pub fn remove_game(
        &self,
        id: redunar_core::GameId,
    ) -> Result<redunar_core::GameRecord, GameCatalogError> {
        game_catalog::remove(&self.state_directory, id)
    }

    /// Update the global game-profile defaults. No setting is applied here.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when state cannot be saved.
    /// Read completed local game sessions newest last.
    pub fn session_history(&self) -> Result<Vec<SessionRecord>, String> {
        session_history::load(&self.state_directory).map_err(|error| error.to_string())
    }

    /// Append one completed capture session after daemon cleanup has finished.
    ///
    /// # Errors
    ///
    /// Returns an error when the bounded local journal cannot be updated.
    pub fn record_session(&self, record: SessionRecord) -> Result<(), String> {
        session_history::append(&self.state_directory, record).map_err(|error| error.to_string())
    }

    /// Persist the Global workspace draft after comparing it with the values the UI loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if another writer changed the draft, recording settings are locked, or persistence fails.
    /// Frame rate, quality, and storage remain subject to the active-session
    /// lock. Output format may change for future saves during that session.
    pub fn save_global_workspace(
        &self,
        expected: redunar_core::GlobalGameProfile,
        requested: redunar_core::GlobalGameProfile,
        expected_format: ReplayOutputFormat,
        requested_format: ReplayOutputFormat,
    ) -> Result<(), String> {
        let current = self.load_game_catalog().map_err(|e| e.to_string())?;
        if current.global_profile != expected {
            return Err(
                "Global settings changed elsewhere. Reload before saving this draft.".to_owned(),
            );
        }
        let current_format = self
            .replay_preferences()
            .map_err(|e| e.to_string())?
            .output_format;
        if current_format != expected_format {
            return Err(
                "Replay format changed elsewhere. Reload before saving this draft.".to_owned(),
            );
        }
        let locked = self.replay_recording_settings_locked();
        if locked
            && (requested.replay.frame_rate != expected.replay.frame_rate
                || requested.replay.quality != expected.replay.quality
                || requested.replay.storage_limit != expected.replay.storage_limit)
        {
            return Err(
                "Replay recording settings are locked until the current game closes.".to_owned(),
            );
        }
        self.update_global_game_profile(requested)
            .map_err(|e| e.to_string())?;
        if requested_format != expected_format
            && let Err(error) = self.set_replay_output_format(requested_format)
        {
            let _ = self.update_global_game_profile(expected);
            return Err(error.to_string());
        }
        Ok(())
    }

    /// Replace the persisted global game profile after validating its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when the catalog cannot be read, validated,
    /// or atomically updated.
    pub fn update_global_game_profile(
        &self,
        profile: redunar_core::GlobalGameProfile,
    ) -> Result<redunar_core::GameCatalog, GameCatalogError> {
        game_catalog::update_global_profile(&self.state_directory, profile)
    }

    /// Update only the global Replay defaults, preserving unrelated profile
    /// fields that may have changed in another settings surface.
    ///
    /// # Errors
    ///
    /// Returns [`GameCatalogError`] when local profile state cannot be loaded
    /// or saved safely.
    pub fn update_global_replay_settings(
        &self,
        settings: redunar_core::ReplaySettings,
    ) -> Result<redunar_core::GameCatalog, GameCatalogError> {
        if self.replay_recording_settings_locked() {
            let current = self.load_game_catalog()?.global_profile.replay;
            if settings.frame_rate != current.frame_rate
                || settings.quality != current.quality
                || settings.storage_limit != current.storage_limit
            {
                return Err(GameCatalogError::new(
                    "Replay recording settings are locked until the current game closes",
                ));
            }
        }
        game_catalog::update_global_replay_settings(&self.state_directory, settings)
    }

    /// Recording-profile changes cannot reconfigure an encoder already owned
    /// by a live game session. Clip duration remains a save-time choice.
    #[must_use]
    pub fn replay_recording_settings_locked(&self) -> bool {
        replay_settings_locked_for_phase(self.game_session_coordinator().status().phase)
    }

    /// Inspect the clean-room KMS Replay source without opening a DRM device,
    /// requesting authorization, starting an encoder, or changing production
    /// Replay availability.
    #[must_use]
    pub fn kms_replay_diagnostic_probe(&self) -> KmsProbe {
        KmsProbe::local()
    }

    /// Run a short, hardware-only KMS-to-Vulkan-Video verification without
    /// creating a Replay spool or saving captured desktop pixels.
    ///
    /// This requests the same Polkit authorization and uses the same helper,
    /// DMA-BUF import, conversion shader, and encoder as a real game session.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayRuntimeError`] when settings cannot be loaded or any
    /// live capture/encode stage fails closed.
    pub fn run_kms_replay_live_diagnostic(
        &self,
        requested_frames: u16,
    ) -> Result<KmsReplayLiveDiagnostic, ReplayRuntimeError> {
        let settings = self
            .load_game_catalog()
            .map_err(|error| ReplayRuntimeError::new(error.to_string()))?
            .global_profile
            .replay;
        kms_replay_source::run_live_diagnostic(settings, requested_frames)
    }
}

const fn replay_settings_locked_for_phase(phase: GameSessionPhase) -> bool {
    !matches!(phase, GameSessionPhase::Idle | GameSessionPhase::Ended)
}

fn vulkan_replay_backend(
    frame: &DmaBufReplayFrame,
    settings: redunar_core::ReplaySettings,
) -> Result<Box<dyn HardwareEncoderBackend>, String> {
    // Retained KMS engineering diagnostics have no game producer identity.
    // This separate entry point never authorizes the production game path.
    vulkan_replay_backend_inner(frame, settings, false, false)
}

fn vulkan_replay_backend_with_nvidia_beta(
    frame: &DmaBufReplayFrame,
    settings: redunar_core::ReplaySettings,
    allow_nvidia_beta: bool,
) -> Result<Box<dyn HardwareEncoderBackend>, String> {
    vulkan_replay_backend_inner(frame, settings, allow_nvidia_beta, true)
}

fn validate_game_gpu_identity(
    identity: Option<redunar_capture::CaptureGpuIdentity>,
    allow_nvidia_beta: bool,
    require_game_identity: bool,
) -> Result<bool, String> {
    if require_game_identity && identity.is_none() {
        replay_nvidia_diagnostics::source_device(
            allow_nvidia_beta,
            "unavailable",
            "source_identity_missing",
        );
        return Err("Replay source GPU identity is unavailable".to_owned());
    }
    let selected_nvidia = identity.is_some_and(|identity| identity.vendor_id() == 0x10de);
    if identity.is_some_and(|identity| !matches!(identity.vendor_id(), 0x1002 | 0x10de))
        || (selected_nvidia && !allow_nvidia_beta)
    {
        replay_nvidia_diagnostics::source_device(
            allow_nvidia_beta,
            "unavailable",
            "source_vendor_not_admitted",
        );
        return Err("Replay source GPU is not an admitted hardware encoder candidate".to_owned());
    }
    Ok(selected_nvidia)
}

fn vulkan_replay_backend_inner(
    frame: &DmaBufReplayFrame,
    settings: redunar_core::ReplaySettings,
    allow_nvidia_beta: bool,
    require_game_identity: bool,
) -> Result<Box<dyn HardwareEncoderBackend>, String> {
    use replay_nvidia_diagnostics::{Stage, startup_result};
    const DRM_FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
    const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
    const DRM_FORMAT_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
    const DRM_FORMAT_ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
    let variable_rate = settings.frame_rate == redunar_core::ReplayFrameRate::Variable;
    let identity = frame.gpu_identity();
    let selected_nvidia =
        validate_game_gpu_identity(identity, allow_nvidia_beta, require_game_identity)?;
    replay_nvidia_diagnostics::source_device(
        selected_nvidia,
        "identified",
        "awaiting_encoder_match",
    );
    let request = redunar_capture_vulkan::replay_video::VulkanVideoH264Request {
        width: frame.width,
        height: frame.height,
        frames_per_second: if variable_rate {
            redunar_capture_vulkan::replay_video::maximum_variable_frame_rate(
                frame.width,
                frame.height,
            )
        } else {
            u8::try_from(settings.frame_rate.frames_per_second())
                .map_err(|_| "Replay frame rate exceeds the Vulkan backend bound".to_owned())?
        },
        variable_rate,
        target_megabits_per_second: settings.quality.target_megabits_per_second(),
    };
    let device = startup_result(
        selected_nvidia,
        Stage::Device,
        match identity {
            Some(identity) => {
                redunar_capture_vulkan::replay_video::VulkanVideoH264Device::open_for_capture_gpu(
                    request, identity,
                )
            }
            None => redunar_capture_vulkan::replay_video::VulkanVideoH264Device::open(request),
        },
    )?;
    replay_nvidia_diagnostics::startup_ready(selected_nvidia, Stage::Device);
    if diagnostic_log::enabled() {
        let candidate = device.candidate();
        diagnostic_log::log(&format!(
            "Replay event=device_selected vendor_id={:04x} vulkan_api_raw={} driver_version_raw={} identity=matched",
            candidate.vendor_id, candidate.api_version, candidate.driver_version,
        ));
    }
    diagnostic_log::log(&format!(
        "Replay event=encoder_request width={} height={} fps_limit={} variable={} bitrate_mbps={} source_fourcc={}",
        request.width,
        request.height,
        request.frames_per_second,
        request.variable_rate,
        request.target_megabits_per_second,
        frame.drm_fourcc,
    ));
    let session = startup_result(selected_nvidia, Stage::Session, device.create_session())?;
    replay_nvidia_diagnostics::startup_ready(selected_nvidia, Stage::Session);
    let parameters = startup_result(
        selected_nvidia,
        Stage::Parameters,
        session.create_parameters(),
    )?;
    replay_nvidia_diagnostics::startup_ready(selected_nvidia, Stage::Parameters);
    let encoder = startup_result(
        selected_nvidia,
        Stage::Encoder,
        if matches!(
            frame.drm_fourcc,
            DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 | DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888
        ) {
            parameters.create_production_kms_encoder()
        } else {
            parameters.create_production_encoder()
        },
    )?;
    replay_nvidia_diagnostics::startup_ready(selected_nvidia, Stage::Encoder);
    VulkanVideoH264Backend::new(encoder)
        .map(|backend| match identity {
            Some(identity) => backend.with_capture_gpu(identity),
            None => backend,
        })
        .map(|backend| Box::new(backend) as Box<dyn HardwareEncoderBackend>)
        .map_err(|error| {
            if !selected_nvidia {
                return error.to_string();
            }
            replay_nvidia_diagnostics::backend_failed(selected_nvidia);
            "Replay hardware encoder startup failed".to_owned()
        })
}

fn default_state_directory() -> PathBuf {
    if let Some(state_home) = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return state_home.join("redunar");
    }
    if let Some(home) = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return home.join(".local/state/redunar");
    }

    // A relative environment-controlled path would make daemon state depend
    // on its launch directory. `current_dir` is normally absolute and keeps
    // the last-resort behavior deterministic without writing system-wide.
    env::current_dir()
        .unwrap_or_else(|_| env::temp_dir())
        .join(".redunar-state")
}

fn absolute_home_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn beta_preference_never_overrides_the_game_vendor() {
        let identity = |vendor| {
            Some(redunar_capture::CaptureGpuIdentity::new(vendor, [1; 16], [2; 16]).unwrap())
        };
        for beta in [false, true] {
            assert_eq!(
                validate_game_gpu_identity(identity(0x1002), beta, true),
                Ok(false)
            );
            assert!(validate_game_gpu_identity(identity(0x8086), beta, true).is_err());
        }
        assert!(validate_game_gpu_identity(identity(0x10de), false, true).is_err());
        assert_eq!(
            validate_game_gpu_identity(identity(0x10de), true, true),
            Ok(true)
        );
    }

    #[test]
    fn production_replay_without_game_gpu_identity_never_probes_hardware() {
        let file = fs::File::open("/dev/null").unwrap();
        let frame = DmaBufReplayFrame::new_multi_object(
            vec![file.into()],
            320,
            240,
            u32::from_le_bytes(*b"XR24"),
            0,
            1,
            16_666_667,
            vec![DmaBufImagePlane {
                object_index: 0,
                offset: 0,
                stride: 1_280,
            }],
        )
        .unwrap();
        for beta in [false, true] {
            let error = vulkan_replay_backend_with_nvidia_beta(
                &frame,
                redunar_core::ReplaySettings::default(),
                beta,
            )
            .err()
            .unwrap();
            assert_eq!(error, "Replay source GPU identity is unavailable");
        }
    }

    #[test]
    fn read_only_tauri_service_cannot_claim_the_primary_replay_socket() {
        let secondary = RedunarService::for_tauri_read_only();
        assert!(secondary.replay_control_path.is_none());
        assert!(secondary.runtime.replay_control.get().is_none());
    }

    #[test]
    fn replay_recording_settings_lock_tracks_the_complete_game_lifecycle() {
        assert!(!replay_settings_locked_for_phase(GameSessionPhase::Idle));
        assert!(replay_settings_locked_for_phase(
            GameSessionPhase::Launching
        ));
        assert!(replay_settings_locked_for_phase(GameSessionPhase::Running));
        assert!(replay_settings_locked_for_phase(GameSessionPhase::Ending));
        assert!(!replay_settings_locked_for_phase(GameSessionPhase::Ended));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = env::temp_dir().join(format!("redunar-daemon-{}-{id}", std::process::id()));
            fs::create_dir_all(&root).expect("create fixture");
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("remove fixture");
        }
    }

    #[test]
    fn service_resolves_global_and_per_game_overlay_visibility() {
        let fixture = Fixture::new();
        let executable = fixture.root.join("game-bin");
        fs::write(&executable, b"fixture").expect("write executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("make executable");
        let service = RedunarService::with_state_directory(fixture.root.join("state"));
        let game = service
            .add_game(AddGameRequest::new("Overlay Game", executable))
            .expect("add game");
        service
            .update_global_game_profile(redunar_core::GlobalGameProfile {
                capture_metrics: false,
                overlay_visible: true,
                instant_replay: false,
                ..redunar_core::GlobalGameProfile::default()
            })
            .expect("update global profile");

        let inherited = service
            .effective_game_profile(game.id)
            .expect("resolve inherited profile");
        assert!(!inherited.capture_metrics);
        assert!(inherited.overlay_visible);

        service
            .update_game_profile(
                game.id,
                redunar_core::PerGameProfile {
                    capture_metrics: redunar_core::Inheritable::InheritGlobal,
                    overlay_visible: redunar_core::Inheritable::Custom(false),
                    instant_replay: redunar_core::Inheritable::InheritGlobal,
                },
            )
            .expect("update per-game profile");
        assert!(
            !service
                .effective_game_profile(game.id)
                .expect("resolve custom profile")
                .overlay_visible
        );
    }

    #[test]
    fn replay_status_uses_persisted_settings_but_refuses_activation() {
        let fixture = Fixture::new();
        let service = RedunarService::with_state_directory(fixture.root.join("state"));
        service
            .update_global_game_profile(redunar_core::GlobalGameProfile {
                overlay_visible: true,
                replay: redunar_core::ReplaySettings {
                    duration: redunar_core::ReplayDuration::Seconds60,
                    frame_rate: redunar_core::ReplayFrameRate::Fps30,
                    quality: redunar_core::ReplayQuality::Efficient,
                    storage_limit: redunar_core::ReplayStorageLimit::GiB5,
                },
                ..redunar_core::GlobalGameProfile::default()
            })
            .expect("persist replay settings");

        let replay_settings = redunar_core::ReplaySettings {
            duration: redunar_core::ReplayDuration::Seconds120,
            frame_rate: redunar_core::ReplayFrameRate::Fps30,
            quality: redunar_core::ReplayQuality::High,
            storage_limit: redunar_core::ReplayStorageLimit::GiB25,
        };
        let catalog = service
            .update_global_replay_settings(replay_settings)
            .expect("update only replay settings");
        assert!(catalog.global_profile.overlay_visible);
        assert_eq!(catalog.global_profile.replay, replay_settings);

        let status = service.replay_runtime_status().expect("replay status");
        assert_eq!(status.phase, ReplayPhase::Unavailable);
        assert_eq!(status.settings.duration.seconds(), 120);
        assert_eq!(status.budget.frame_capacity, 3_600);
        assert_eq!(request_replay_start(status), Err(ReplayStartError));
    }

    #[test]
    fn replay_store_is_initialized_only_by_the_explicit_preparation_api() {
        let fixture = Fixture::new();
        let state = fixture.root.join("state");
        let service = RedunarService::with_state_directory(&state);
        let custom_parent = fixture.root.join("captures");
        service
            .set_replay_save_parent(Some(custom_parent.clone()))
            .expect("select custom Replay parent");
        service
            .set_replay_save_shortcuts(vec![ReplayShortcutBinding {
                shortcut: "Super+F10".to_owned(),
                duration: redunar_core::ReplayDuration::Seconds300,
            }])
            .expect("save Replay shortcuts");
        assert_eq!(
            service
                .replay_preferences()
                .expect("load Replay preferences")
                .save_shortcuts,
            vec![ReplayShortcutBinding {
                shortcut: "Super+F10".to_owned(),
                duration: redunar_core::ReplayDuration::Seconds300,
            }]
        );
        assert_eq!(
            service
                .effective_replay_retention_duration()
                .expect("resolve Replay retention"),
            redunar_core::ReplayDuration::Seconds900
        );
        let replay_directory = custom_parent.join("Redunar Replays");
        assert_eq!(
            service
                .replay_save_directory()
                .expect("resolve custom Replay directory"),
            replay_directory
        );
        assert!(!custom_parent.exists());
        service.replay_runtime_status().expect("read replay status");
        let storage = service
            .replay_storage_status()
            .expect("inspect replay storage");
        assert!(!storage.initialized);
        assert_eq!(storage.directory, replay_directory);
        assert!(!replay_directory.exists());
        assert!(
            service
                .replay_clip_inventory()
                .expect("inspect missing clip inventory")
                .is_empty()
        );
        assert!(service.delete_replay_clip("../../not-a-clip").is_err());
        assert!(!replay_directory.exists());

        let store = service
            .prepare_replay_store()
            .expect("prepare replay store");
        assert_eq!(store.directory(), replay_directory);
        assert!(store.directory().is_dir());
        assert!(
            service
                .replay_clip_inventory()
                .expect("inspect initialized clip inventory")
                .is_empty()
        );
        assert!(!service.replay_backend_readiness().activation_allowed());
        assert_eq!(
            service.replay_runtime_status().expect("status").phase,
            ReplayPhase::Unavailable
        );

        service
            .set_replay_save_parent(None)
            .expect("reset Replay parent");
        assert_eq!(
            service
                .replay_save_directory()
                .expect("resolve fallback Replay directory"),
            state.join("replays-v1")
        );
        assert!(!state.join("replays-v1").exists());
        assert!(replay_directory.exists());
    }

    #[test]
    fn replay_foundation_self_test_uses_persisted_settings_without_opening_production_gate() {
        let fixture = Fixture::new();
        let state = fixture.root.join("state");
        let diagnostic = fixture.root.join("diagnostic-replay");
        let service = RedunarService::with_state_directory(&state);
        service
            .update_global_game_profile(redunar_core::GlobalGameProfile {
                replay: redunar_core::ReplaySettings {
                    frame_rate: redunar_core::ReplayFrameRate::Fps30,
                    ..redunar_core::ReplaySettings::default()
                },
                ..redunar_core::GlobalGameProfile::default()
            })
            .expect("persist replay settings");

        let report = service
            .run_replay_foundation_self_test(&diagnostic)
            .expect("run replay foundation self-test");
        assert_eq!(
            report.settings.frame_rate,
            redunar_core::ReplayFrameRate::Fps30
        );
        assert_eq!(report.packet_count, 60);
        assert!(report.stored_clip.path.starts_with(&diagnostic));
        assert!(!report.backend_readiness.activation_allowed());
        assert_eq!(
            service.replay_runtime_status().expect("status").phase,
            ReplayPhase::Unavailable
        );
        assert!(!state.join("replays-v1").exists());
    }
}
