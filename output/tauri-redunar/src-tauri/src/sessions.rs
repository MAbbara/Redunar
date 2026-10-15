//! Tauri's native session owner. The webview only requests actions and reads
//! snapshots; a bounded worker keeps supervising the game when it is hidden.
use crate::launch_plan::{self, PreparedLaunch};
use redunar_core::GameId;
use redunar_daemon::{
    CaptureLaunchDisposition, CaptureLaunchProcessState, CaptureSnapshot,
    GameLaunchProcessOwnership, GameSessionPhase, MonitorReader, MonitorSnapshot, RedunarService,
    SessionRecord, SessionTelemetrySample,
};
use std::{
    path::Path,
    process::Child,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_SESSION_TIMELINE_SAMPLES: usize = 7_200;
// Steam can spend several minutes preparing shaders before it exposes the
// game process. Keep the forwarding supervisor alive for that startup window.
const STEAM_START_TIMEOUT: Duration = Duration::from_mins(5);

pub struct Sessions {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
    steam: Mutex<Option<redunar_platform::SteamSessionBroker>>,
}
type LifecycleHandler = Arc<dyn Fn(crate::session_lifecycle::Event) + Send + Sync>;

struct Shared {
    engine: Mutex<Engine>,
    wake: Condvar,
    lifecycle_handler: Mutex<Option<LifecycleHandler>>,
}

struct ActiveLaunch {
    child: LaunchProcess,
    ownership: GameLaunchProcessOwnership,
    capture: bool,
    profile: redunar_core::EffectiveGameProfile,
    game_name: String,
    game_id: Option<GameId>,
    started: Instant,
    started_unix: u64,
    initial_save_revision: u64,
    steam_seen: bool,
    confirmed: bool,
    stopped: bool,
    completed: bool,
    record: Option<SessionRecord>,
    history_written: bool,
    cleanup_done: bool,
    monitor_revision: u64,
    exit_failure: Option<String>,
    timeline: SessionTimeline,
}

// External Steam launches exec in the authenticated wrapper PID. Track its
// kernel start time; an exec preserves identity while PID reuse does not.
enum LaunchProcess {
    Child(Child),
    Steam(redunar_daemon::ProcessIdentity),
}

impl LaunchProcess {
    #[cfg(test)]
    fn child_mut(&mut self) -> &mut Child {
        match self {
            Self::Child(child) => child,
            Self::Steam(_) => panic!("fixture expected owned child"),
        }
    }

    fn observe(&mut self) -> (CaptureLaunchProcessState, Option<String>) {
        let mut failure = None;
        let state = match self {
            Self::Child(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        failure = Some(format!(
                            "Game launch process exited unsuccessfully: {status}"
                        ));
                    }
                    CaptureLaunchProcessState::Exited(status.to_string())
                }
                Ok(None) => CaptureLaunchProcessState::Running,
                Err(error) => CaptureLaunchProcessState::MonitorFailed(error.to_string()),
            },
            Self::Steam(identity) => match identity.is_alive() {
                Ok(true) => CaptureLaunchProcessState::Running,
                Ok(false) => CaptureLaunchProcessState::Exited("Steam launch process ended".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    CaptureLaunchProcessState::Exited("Steam launch process ended".into())
                }
                Err(error) => CaptureLaunchProcessState::MonitorFailed(error.to_string()),
            },
        };
        (state, failure)
    }
}

#[derive(Debug)]
struct SessionTimeline {
    samples: Vec<SessionTelemetrySample>,
    cadence_seconds: u32,
    next_sample_seconds: u32,
}

impl Default for SessionTimeline {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            cadence_seconds: 1,
            next_sample_seconds: 0,
        }
    }
}

impl SessionTimeline {
    fn record(
        &mut self,
        elapsed: Duration,
        capture: Option<&CaptureSnapshot>,
        monitor: &MonitorSnapshot,
    ) {
        let elapsed_seconds = elapsed.as_secs().min(u64::from(u32::MAX)) as u32;
        if elapsed_seconds < self.next_sample_seconds {
            return;
        }
        let metrics = capture.and_then(|snapshot| snapshot.metrics.as_ref());
        let hardware = monitor.hardware.as_deref();
        let gpu = redunar_daemon::capture_gpu(capture, hardware);
        self.push(SessionTelemetrySample {
            elapsed_seconds,
            fps: metrics.map(|value| value.average_fps),
            frame_time_ms: metrics.map(|value| value.newest_frame_time_ms),
            cpu_temperature_celsius: hardware.and_then(|value| value.cpu.temperature_celsius),
            gpu_temperature_celsius: gpu.and_then(|value| value.temperature_celsius),
            cpu_utilization_percent: hardware.and_then(|value| value.cpu.utilization_percent),
            gpu_utilization_percent: gpu.and_then(|value| value.utilization_percent),
        });
        self.next_sample_seconds = elapsed_seconds.saturating_add(self.cadence_seconds);
    }

    fn push(&mut self, sample: SessionTelemetrySample) {
        self.samples.push(sample);
        if self.samples.len() <= MAX_SESSION_TIMELINE_SAMPLES {
            return;
        }
        // Retain the complete time span while bounding memory and journal
        // growth. Very long sessions gradually trade one-second precision for
        // two, four, or eight-second observations instead of losing the start.
        self.samples = self.samples.iter().step_by(2).cloned().collect();
        self.cadence_seconds = self.cadence_seconds.saturating_mul(2).max(1);
    }
}

pub(crate) struct Engine {
    service: RedunarService,
    active: Option<ActiveLaunch>,
    pub capture: Option<Arc<CaptureSnapshot>>,
    pub message: Option<String>,
    pub history_revision: u64,
    shutdown: bool,
    pub(crate) lifecycle: crate::session_lifecycle::Lifecycle,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn steam_wrapper_matches(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let expected = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("redunar-steam-launch"))
    });
    expected
        .and_then(|path| std::fs::metadata(path).ok())
        .zip(std::fs::metadata(format!("/proc/{pid}/exe")).ok())
        .is_some_and(|(expected, actual)| {
            expected.dev() == actual.dev() && expected.ino() == actual.ino()
        })
}

impl Sessions {
    pub fn new(service: RedunarService, monitor: MonitorReader) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            engine: Mutex::new(Engine::new(service)),
            wake: Condvar::new(),
            lifecycle_handler: Mutex::new(None),
        });
        let worker = start_worker(shared.clone(), move || monitor.snapshot())?;
        let steam = if !lock(&shared.engine).service.is_read_only() {
            let weak = Arc::downgrade(&shared);
            let bind = (|| {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(std::path::PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .ok_or_else(|| std::io::Error::other("runtime unavailable"))?;
                let directory = runtime.join("redunar");
                match std::fs::create_dir(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                redunar_platform::SteamSessionBroker::bind(
                    redunar_platform::steam_session_socket_path(&runtime),
                    move |app, pid| {
                        let Some(shared) = weak.upgrade() else {
                            return false;
                        };
                        let mut engine = lock(&shared.engine);
                        let result = if steam_wrapper_matches(pid) {
                            engine.attach_steam(app, pid)
                        } else {
                            Err("Steam launch wrapper does not match this Redunar build.".into())
                        };
                        if let Err(error) = &result {
                            engine.message = Some(error.clone());
                        }
                        redunar_daemon::diagnostic_log::log(if result.is_ok() {
                            "Steam launch event=session_prepared"
                        } else {
                            "Steam launch event=session_rejected"
                        });
                        shared.wake.notify_one();
                        result.is_ok()
                    },
                )
            })();
            match bind {
                Ok(broker) => Some(broker),
                Err(_) => {
                    redunar_daemon::diagnostic_log::log("Steam launch event=listener_unavailable");
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
            steam: Mutex::new(steam),
        })
    }

    pub fn read<T>(&self, read: impl FnOnce(&Engine) -> T) -> T {
        read(&lock(&self.shared.engine))
    }

    pub(crate) fn set_lifecycle_handler(
        &self,
        handler: impl Fn(crate::session_lifecycle::Event) + Send + Sync + 'static,
    ) {
        *lock(&self.shared.lifecycle_handler) = Some(Arc::new(handler));
        // Hold the engine lock so a completion during setup cannot lose this wake.
        let _engine = lock(&self.shared.engine);
        self.shared.wake.notify_one();
    }

    pub(crate) fn prepare_background_exit(&self, generation: u64) -> Result<bool, String> {
        lock(&self.shared.engine).prepare_background_exit(generation)
    }

    pub fn launch(&self, game_id: &str) -> Result<(), String> {
        let mut engine = lock(&self.shared.engine);
        engine.ensure_idle()?;
        let id = GameId::new(game_id.parse().map_err(|_| "Invalid game identifier")?)
            .map_err(|e| e.to_string())?;
        let plan = launch_plan::prepare(&engine.service, id)?;
        engine.start(plan)?;
        self.shared.wake.notify_one();
        Ok(())
    }

    pub fn end(&self) -> Result<(), String> {
        let result = lock(&self.shared.engine).finish();
        self.shared.wake.notify_one();
        result
    }

    /// Apply Global overlay appearance changes to the running capture without
    /// restarting the game. Visibility follows the saved global/per-game profile;
    /// other feature switches retain their launch values.
    pub fn update_active_overlay_config(
        &self,
        profile: &redunar_core::GlobalGameProfile,
    ) -> Result<Option<bool>, String> {
        lock(&self.shared.engine).update_active_overlay_config(profile)
    }

    pub fn update_game_overlay_config(&self, game_id: GameId) -> Result<Option<bool>, String> {
        let mut engine = lock(&self.shared.engine);
        engine.update_game_overlay_config(game_id)
    }

    pub(crate) fn claim_update_restart(&self) -> Result<(), String> {
        lock(&self.shared.engine).claim_update_restart()
    }

    pub fn end_for_exit(&self) -> Result<(), String> {
        let mut engine = lock(&self.shared.engine);
        engine.shutdown = true;
        let result = engine.finish();
        if result.is_err() {
            engine.shutdown = false;
        }
        result
    }

    pub fn shutdown(&self) {
        // The desktop callback captures its AppHandle. Clear it explicitly so
        // managed session state cannot keep the exited app alive through a cycle.
        lock(&self.shared.lifecycle_handler).take();
        // Stop accepting new launches before ending the worker and backend.
        if let Some(mut steam) = lock(&self.steam).take() {
            steam.shutdown();
        }
        lock(&self.shared.engine).shutdown = true;
        self.shared.wake.notify_one();
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
    }
}

fn start_worker(
    work: Arc<Shared>,
    monitor: impl Fn() -> Arc<MonitorSnapshot> + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("redunar-sessions".into())
        .spawn(move || {
            let mut engine = lock(&work.engine);
            loop {
                if engine.shutdown {
                    break;
                }
                if engine.active.is_some() {
                    engine.tick(&monitor());
                }
                let handler = lock(&work.lifecycle_handler).clone();
                if let Some(handler) = handler {
                    if let Some(event) = engine.lifecycle.pending.take() {
                        // Handlers can read session state or request shutdown;
                        // neither callback nor GTK scheduling owns these locks.
                        drop(engine);
                        handler(event);
                        engine = lock(&work.engine);
                        continue;
                    }
                }
                engine = if engine.active.is_some() {
                    work.wake
                        .wait_timeout(engine, POLL_INTERVAL)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0
                } else {
                    work.wake
                        .wait(engine)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                };
            }
        })
}

impl Engine {
    fn new(service: RedunarService) -> Self {
        Self {
            service,
            active: None,
            capture: None,
            message: None,
            history_revision: 0,
            shutdown: false,
            lifecycle: crate::session_lifecycle::Lifecycle::default(),
        }
    }

    fn claim_update_restart(&mut self) -> Result<(), String> {
        self.ensure_idle()
            .map_err(|_| "End the game session before restarting Redunar".to_owned())?;
        // Same lock as manual launch and authenticated Steam attachment.
        self.shutdown = true;
        Ok(())
    }

    fn prepare_background_exit(&mut self, generation: u64) -> Result<bool, String> {
        if self.launch_locked() || !self.lifecycle.can_exit(generation) {
            return Ok(false);
        }
        if self
            .service
            .app_preferences()
            .map_err(|error| format!("Could not check Close to tray: {error}"))?
            .close_to_tray
        {
            return Ok(false);
        }
        // Claim shutdown while holding the same lock as Steam attachment. A
        // newer launch either wins this lock or is rejected before capture starts.
        self.shutdown = true;
        Ok(true)
    }
    pub fn launch_locked(&self) -> bool {
        self.active.is_some() || self.shutdown
    }
    pub(crate) fn has_live_game(&self) -> bool {
        self.active.as_ref().is_some_and(|active| !active.completed)
    }
    pub fn can_end(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| !a.stopped || !a.cleanup_done || !a.history_written)
    }
    pub fn elapsed_seconds(&self) -> Option<u64> {
        self.active.as_ref().map(|active| {
            active.record.as_ref().map_or_else(
                || active.started.elapsed().as_secs(),
                |record| u64::from(record.duration_seconds),
            )
        })
    }
    pub fn captures_saved(&self) -> Option<u64> {
        self.active.as_ref().map(|active| {
            self.service
                .game_session_coordinator()
                .replay_runtime()
                .status()
                .completed_save_revision
                .saturating_sub(active.initial_save_revision)
        })
    }
    pub fn feature_summary(&self) -> Option<String> {
        self.active.as_ref().map(|active| {
            let metrics = if active.profile.capture_metrics {
                "Metrics enabled"
            } else {
                "Metrics disabled"
            };
            let replay = if active.profile.instant_replay {
                "replay enabled"
            } else {
                "replay disabled"
            };
            format!("{metrics} · {replay}")
        })
    }
    fn ensure_idle(&self) -> Result<(), String> {
        let phase = self.service.game_session_coordinator().status().phase;
        if self.launch_locked()
            || !matches!(phase, GameSessionPhase::Idle | GameSessionPhase::Ended)
        {
            return Err("A game is already being supervised or its session still needs cleanup. End it before launching another game.".into());
        }
        Ok(())
    }
    fn attach_steam(&mut self, app: redunar_daemon::SteamAppId, pid: u32) -> Result<(), String> {
        self.ensure_idle()?;
        let identity = redunar_daemon::ProcessIdentity::capture(Path::new("/proc"), pid)
            .map_err(|_| "Steam launch process is no longer available")?;
        let plan = launch_plan::prepare_steam(&self.service, app)?;
        self.start_with_process(plan, Some(LaunchProcess::Steam(identity)))
    }

    fn start(&mut self, plan: PreparedLaunch) -> Result<(), String> {
        self.start_with_process(plan, None)
    }

    fn start_with_process(
        &mut self,
        mut plan: PreparedLaunch,
        process: Option<LaunchProcess>,
    ) -> Result<(), String> {
        self.ensure_idle()?;
        let session = self.service.game_session_coordinator();
        let capture = plan.capture.is_some();
        let name = plan.request.game_name.clone();
        let game_id = plan.request.game_id;
        // Accept ownership before spawning. A failed begin must never leave a
        // running child that neither shell nor coordinator supervises.
        session
            .begin(plan.request, plan.capture, plan.replay)
            .map_err(|e| e.to_string())?;
        self.capture = None;
        self.message = None;
        let child = match process.map_or_else(|| plan.command.spawn().map(LaunchProcess::Child), Ok)
        {
            Ok(child) => child,
            Err(error) => {
                let cleanup = session.end();
                let message = match cleanup {
                    Ok(()) => format!("The game could not start: {error}"),
                    Err(cleanup) => format!(
                        "The game could not start: {error}. Session cleanup requires attention: {cleanup}"
                    ),
                };
                self.message = Some(message.clone());
                return Err(message);
            }
        };
        self.message = Some(
            if capture {
                "Waiting for the game renderer. Steam shader preparation can take a few minutes."
            } else {
                "Game launched without frame capture."
            }
            .into(),
        );
        if !capture && matches!(plan.ownership, GameLaunchProcessOwnership::DirectChild) {
            session.mark_running();
        }
        if capture {
            let _ = session.update_overlay_config(&plan.profile);
        }
        self.lifecycle.started();
        self.active = Some(ActiveLaunch {
            child,
            ownership: plan.ownership,
            capture,
            profile: plan.profile,
            game_name: name,
            game_id,
            started: Instant::now(),
            started_unix: redunar_daemon::session_started_unix(),
            initial_save_revision: session.replay_runtime().status().completed_save_revision,
            steam_seen: false,
            confirmed: matches!(plan.ownership, GameLaunchProcessOwnership::DirectChild),
            stopped: false,
            completed: false,
            record: None,
            history_written: false,
            cleanup_done: false,
            monitor_revision: 0,
            exit_failure: None,
            timeline: SessionTimeline::default(),
        });
        if plan.profile.instant_replay {
            if let Err(error) = self.service.start_replay_validation_candidate() {
                self.message = Some(format!(
                    "Game launched, but recording could not start: {error}"
                ));
            }
        }
        Ok(())
    }

    fn update_game_overlay_config(&mut self, game_id: GameId) -> Result<Option<bool>, String> {
        if self.active.as_ref().and_then(|active| active.game_id) != Some(game_id) {
            return Ok(None);
        }
        let global = self
            .service
            .load_game_catalog()
            .map_err(|error| error.to_string())?
            .global_profile;
        self.update_active_overlay_config(&global)
    }

    fn update_active_overlay_config(
        &mut self,
        global: &redunar_core::GlobalGameProfile,
    ) -> Result<Option<bool>, String> {
        let Some(active) = self.active.as_mut() else {
            return Ok(None);
        };
        if !active.capture || active.stopped {
            return Ok(None);
        }
        // Resolve visibility from the saved catalog so Global changes respect
        // this game's override. Other running feature switches are unchanged.
        let mut profile = active.profile;
        if let Some(id) = active.game_id {
            let catalog = self
                .service
                .load_game_catalog()
                .map_err(|error| error.to_string())?;
            let game = catalog
                .games
                .iter()
                .find(|game| game.id == id)
                .ok_or("The running game is no longer in the library.")?;
            profile.overlay_visible = game.profile.resolve(*global).overlay_visible;
        }
        profile.overlay_preset = global.overlay_preset;
        profile.overlay_layout = global.overlay_layout;
        profile.overlay_palette = global.overlay_palette;
        profile.overlay_branding = global.overlay_branding;
        profile.overlay_metrics = global.overlay_metrics;
        profile.overlay_corner = global.overlay_corner;
        profile.overlay_opacity = global.overlay_opacity;
        profile.overlay_scale = global.overlay_scale;
        self.service
            .game_session_coordinator()
            .update_overlay_config(&profile)
            .map_err(|error| error.to_string())?;
        active.profile = profile;
        Ok(Some(active.profile.overlay_visible))
    }

    fn tick(&mut self, monitor: &MonitorSnapshot) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        // A failed history write/cleanup waits for explicit End/Quit retry;
        // repeated ticks must not repeatedly write or overwrite the error.
        if active.stopped && (!active.cleanup_done || !active.history_written) {
            return;
        }
        let (process, failure) = active.child.observe();
        if failure.is_some() {
            active.exit_failure = failure;
        }
        let session = self.service.game_session_coordinator();
        let release = if active.capture && !active.stopped {
            if monitor.revision != active.monitor_revision {
                active.monitor_revision = monitor.revision;
                let _ = session.update_overlay_hardware(monitor.hardware.as_deref());
            }
            let release =
                session.observe_launch_process(&process) == Some(CaptureLaunchDisposition::Release);
            self.capture = session.capture_snapshot();
            if let Some(snapshot) = &self.capture {
                // Steam can expose its helper before the final game process
                // is visible, especially while shaders compile. Accepted
                // capture evidence proves that the launched game connected.
                active.confirmed |= capture_evidence_confirms(
                    snapshot.producer_process_id,
                    snapshot.received_frame_count,
                    snapshot.metrics.is_some(),
                );
                self.message = snapshot.failure.clone().or_else(|| {
                    Some(format!(
                        "{:?} · {} frames received · {} transport drops",
                        snapshot.phase, snapshot.received_frame_count, snapshot.dropped_frame_count
                    ))
                });
            }
            match active.ownership {
                GameLaunchProcessOwnership::DirectChild => release,
                GameLaunchProcessOwnership::ForwardedSteam { app_id } => {
                    // The Steam helper and even its capture producer can exit
                    // before the game. Keep separate cached game observations.
                    if release {
                        session.stop_capture();
                        active.capture = false;
                    }
                    forwarded_finished(active, monitor, app_id, &process)
                }
            }
        } else {
            match active.ownership {
                GameLaunchProcessOwnership::DirectChild => {
                    matches!(process, CaptureLaunchProcessState::Exited(_))
                }
                GameLaunchProcessOwnership::ForwardedSteam { app_id } => {
                    forwarded_finished(active, monitor, app_id, &process)
                }
            }
        };
        active
            .timeline
            .record(active.started.elapsed(), self.capture.as_deref(), monitor);
        if let CaptureLaunchProcessState::MonitorFailed(error) = process {
            self.message = Some(format!("Process exit could not be confirmed: {error}"));
            return;
        }
        if active.steam_seen && !active.stopped {
            session.mark_running();
        }
        if release {
            active.completed = true;
            if !active.stopped {
                let _ = self.finish();
            }
            if self
                .active
                .as_ref()
                .is_some_and(|a| a.cleanup_done && a.history_written)
            {
                self.active = None;
                self.lifecycle.finished();
            }
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        let session = self.service.game_session_coordinator();
        let Some(active) = self.active.as_mut() else {
            return session.end().map_err(|e| e.to_string());
        };
        if !active.stopped {
            if active.capture {
                self.capture = session.capture_snapshot().or(self.capture.take());
            }
            active.stopped = true;
            let metrics = self
                .capture
                .as_ref()
                .and_then(|snapshot| snapshot.metrics.as_ref());
            active.record = Some(SessionRecord {
                id: 0,
                game: active.game_name.clone(),
                started_unix: active.started_unix,
                duration_seconds: active.started.elapsed().as_secs().min(u64::from(u32::MAX))
                    as u32,
                average_fps: metrics.map(|m| m.average_fps),
                one_percent_low_fps: metrics.map(|m| m.one_percent_low_fps),
                point_one_percent_low_fps: metrics.map(|m| m.point_one_percent_low_fps),
                frame_intervals_ns: self.capture.as_ref().map_or_else(Vec::new, |s| {
                    s.recent_frame_intervals_ns
                        .iter()
                        .copied()
                        .take(240)
                        .collect()
                }),
                timeline: active.timeline.samples.clone(),
            });
        }
        // Cleanup always runs, even when saving the history record fails.
        let cleanup = session.end().map_err(|e| e.to_string());
        active.cleanup_done = cleanup.is_ok();
        let history = if !active.confirmed || active.history_written {
            active.history_written = true;
            Ok(())
        } else {
            self.service
                .record_session(active.record.as_ref().expect("finished record").clone())
                .map(|()| {
                    active.history_written = true;
                    self.history_revision = self.history_revision.saturating_add(1);
                })
        };
        let result = cleanup.and(history);
        if let Err(error) = &result {
            self.message = Some(format!(
                "Session ended with pending work: {error}. Use End session to retry."
            ));
            self.lifecycle
                .needs_attention(self.message.clone().expect("cleanup failure message"));
        } else {
            self.message = active.exit_failure.clone().or_else(|| Some(if !active.confirmed { "Steam launch was not confirmed. Redunar released its session resources; the game may still start." } else if active.completed { "Game session ended." } else { "Redunar's session ended. The game can keep running." }.into()));
        }
        if result.is_ok() && active.completed {
            self.active = None;
            self.lifecycle.finished();
        }
        result
    }
}

fn capture_evidence_confirms(
    producer_process_id: Option<u32>,
    received_frame_count: u64,
    metrics_available: bool,
) -> bool {
    producer_process_id.is_some() || received_frame_count > 0 || metrics_available
}

fn forwarded_finished(
    active: &mut ActiveLaunch,
    monitor: &MonitorSnapshot,
    app_id: Option<u32>,
    process: &CaptureLaunchProcessState,
) -> bool {
    // Failed/stale process scans are not evidence that a game exited.
    if monitor.diagnostics.game_detection_error.is_some() || monitor.diagnostics.game_scans == 0 {
        return false;
    }
    let running = app_id.is_some_and(|id| {
        monitor
            .games
            .iter()
            .any(|game| game.steam_app_id == Some(id))
    });
    if running {
        active.steam_seen = true;
        active.confirmed = true;
        return false;
    }
    // The directly authenticated Steam wrapper may itself be the game after
    // exec. Cached discovery alone must not release a still-live owner.
    if matches!(active.child, LaunchProcess::Steam(_))
        && matches!(process, CaptureLaunchProcessState::Running)
    {
        return false;
    }
    active.steam_seen || active.started.elapsed() >= STEAM_START_TIMEOUT
}

#[tauri::command]
pub fn launch_game(game_id: String, sessions: tauri::State<'_, Sessions>) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    sessions.launch(&game_id)
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
