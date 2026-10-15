use super::*;
#[path = "sessions_background_tests.rs"]
mod background;
use redunar_core::{
    CpuSnapshot, GameProcess, GlobalGameProfile, GpuSnapshot, PerGameProfile, SystemSnapshot,
};
use redunar_daemon::{
    AddGameRequest, CaptureSessionConfig, GameSessionRequest, ReplayRuntimeStatus,
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    engine: Engine,
}
struct TempRoot(PathBuf);
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rd-tauri-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = Engine::new(RedunarService::with_state_directory(root.join("state")));
        Self { root, engine }
    }
    fn plan(&self, program: &str, args: &[&str]) -> PreparedLaunch {
        let profile = PerGameProfile::default().resolve(GlobalGameProfile {
            capture_metrics: false,
            overlay_visible: false,
            instant_replay: false,
            ..Default::default()
        });
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        PreparedLaunch {
            command,
            capture: None,
            profile,
            ownership: GameLaunchProcessOwnership::DirectChild,
            request: GameSessionRequest {
                game_id: None,
                game_name: "Fixture game".into(),
                metrics: false.into(),
                overlay: false.into(),
                replay: false.into(),
            },
            replay: ReplayRuntimeStatus::unavailable(profile.replay),
        }
    }
    fn reap(&mut self) {
        if let LaunchProcess::Child(child) = &mut self.engine.active.as_mut().unwrap().child {
            child.wait().unwrap();
        }
        self.engine.tick(&MonitorSnapshot::default());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(active) = self.engine.active.as_mut() {
            // Only the harmless child created by this test is terminated.
            if let LaunchProcess::Child(child) = &mut active.child {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        let _ = self.engine.service.game_session_coordinator().end();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn duplicate_launch_is_rejected_before_any_second_child_starts() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/sleep", &["30"])).unwrap();
    let marker = f.root.join("unexpected-child");
    assert!(f
        .engine
        .start(f.plan("/usr/bin/touch", &[marker.to_str().unwrap()]))
        .is_err());
    assert!(!marker.exists());
}

#[test]
fn active_capture_accepts_global_overlay_changes_without_a_restart() {
    let mut f = Fixture::new();
    let library = f.root.join("fixture-layer.so");
    std::fs::write(&library, b"fixture only; never loaded").unwrap();
    let capture = f
        .engine
        .service
        .start_capture_session(&CaptureSessionConfig::new(f.root.join("runtime"), library))
        .unwrap();
    let mut plan = f.plan("/bin/sleep", &["30"]);
    plan.capture = Some(capture);
    plan.profile.overlay_visible = true;
    plan.request.overlay = true.into();
    f.engine.start(plan).unwrap();

    let requested = GlobalGameProfile {
        overlay_preset: redunar_core::OverlayPreset::Custom,
        overlay_layout: redunar_core::OverlayLayout::Telemetry,
        overlay_palette: redunar_core::OverlayPalette::Amethyst,
        overlay_metrics: redunar_core::OverlayMetricSet::from_bits(
            redunar_core::OverlayMetricSet::FPS | redunar_core::OverlayMetricSet::GPU_LOAD,
        )
        .unwrap(),
        overlay_corner: redunar_core::OverlayCorner::BottomRight,
        overlay_opacity: redunar_core::OverlayOpacity::new(72).unwrap(),
        overlay_scale: redunar_core::OverlayScale::new(135).unwrap(),
        ..GlobalGameProfile::default()
    };
    assert_eq!(
        f.engine.update_active_overlay_config(&requested).unwrap(),
        Some(true)
    );
    let active = f.engine.active.as_ref().unwrap();
    assert_eq!(active.profile.overlay_preset, requested.overlay_preset);
    assert_eq!(active.profile.overlay_layout, requested.overlay_layout);
    assert_eq!(active.profile.overlay_palette, requested.overlay_palette);
    assert_eq!(active.profile.overlay_metrics, requested.overlay_metrics);
    assert_eq!(active.profile.overlay_corner, requested.overlay_corner);
    assert_eq!(active.profile.overlay_opacity, requested.overlay_opacity);
    assert_eq!(active.profile.overlay_scale, requested.overlay_scale);

    f.engine.finish().unwrap();
}

#[test]
fn failed_spawn_ends_the_reservation_and_creates_no_history() {
    let mut f = Fixture::new();
    assert!(f
        .engine
        .start(f.plan("/no-such-redunar-game", &[]))
        .is_err());
    assert_eq!(
        f.engine.service.game_session_coordinator().status().phase,
        GameSessionPhase::Ended
    );
    assert!(f.engine.service.session_history().unwrap().is_empty());
    f.engine.start(f.plan("/bin/true", &[])).unwrap();
    f.reap();
}

#[test]
fn natural_exit_records_once_and_unknown_metrics_stay_unknown() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/true", &[])).unwrap();
    f.reap();
    f.engine.tick(&MonitorSnapshot::default());
    f.engine.finish().unwrap();
    let history = f.engine.service.session_history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].average_fps, None);
    assert!(history[0].frame_intervals_ns.is_empty());
    assert_eq!(history[0].timeline.len(), 1);
    assert_eq!(history[0].timeline[0].fps, None);
    assert!(!f.engine.launch_locked());
    assert_eq!(f.engine.history_revision, 1);
}

#[test]
fn long_session_timeline_keeps_both_ends_with_bounded_storage() {
    let mut timeline = SessionTimeline::default();
    for elapsed_seconds in 0..=MAX_SESSION_TIMELINE_SAMPLES as u32 {
        timeline.push(SessionTelemetrySample {
            elapsed_seconds,
            fps: Some(60.0),
            frame_time_ms: Some(16.7),
            cpu_temperature_celsius: Some(60.0),
            gpu_temperature_celsius: Some(70.0),
            cpu_utilization_percent: Some(30.0),
            gpu_utilization_percent: Some(90.0),
        });
    }
    assert!(timeline.samples.len() <= MAX_SESSION_TIMELINE_SAMPLES);
    assert_eq!(timeline.samples.first().unwrap().elapsed_seconds, 0);
    assert_eq!(
        timeline.samples.last().unwrap().elapsed_seconds,
        MAX_SESSION_TIMELINE_SAMPLES as u32
    );
    assert_eq!(timeline.cadence_seconds, 2);
}

#[test]
fn session_timeline_without_game_identity_records_cpu_and_omits_gpu() {
    let mut timeline = SessionTimeline::default();
    let monitor = MonitorSnapshot {
        hardware: Some(Arc::new(SystemSnapshot {
            memory: None,
            cpu: CpuSnapshot {
                vendor: "AuthenticAMD".into(),
                model: "Fixture CPU".into(),
                logical_cpus: 8,
                temperature_celsius: Some(61.5),
                utilization_percent: Some(37.0),
                scaling_driver: None,
                governor: None,
                energy_performance_preference: None,
            },
            gpus: vec![GpuSnapshot {
                card: "card0".into(),
                vendor_id: "0x1002".into(),
                device_id: Some("0x73bf".into()),
                model: "Fixture GPU".into(),
                driver: Some("amdgpu".into()),
                temperature_celsius: Some(69.0),
                utilization_percent: Some(93.0),
                clock_mhz: None,
                vram_used_bytes: None,
                vram_total_bytes: None,
                power_watts: None,
                performance_level: None,
            }],
        })),
        ..MonitorSnapshot::default()
    };
    timeline.record(Duration::from_secs(12), None, &monitor);
    let sample = timeline.samples.first().unwrap();
    assert_eq!(sample.elapsed_seconds, 12);
    assert_eq!(sample.cpu_temperature_celsius, Some(61.5));
    assert_eq!(sample.gpu_temperature_celsius, None);
    assert_eq!(sample.cpu_utilization_percent, Some(37.0));
    assert_eq!(sample.gpu_utilization_percent, None);
}

#[test]
fn end_session_does_not_kill_the_game_and_does_not_duplicate_history() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/sleep", &["30"])).unwrap();
    f.engine.finish().unwrap();
    f.engine.finish().unwrap();
    assert!(f
        .engine
        .active
        .as_mut()
        .unwrap()
        .child
        .child_mut()
        .try_wait()
        .unwrap()
        .is_none());
    assert!(f.engine.launch_locked());
    assert!(!f.engine.can_end());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
    f.engine
        .active
        .as_mut()
        .unwrap()
        .child
        .child_mut()
        .kill()
        .unwrap();
    f.reap();
    assert!(!f.engine.launch_locked());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
}

#[test]
fn history_failure_still_cleans_up_and_explicit_retry_writes_once() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/true", &[])).unwrap();
    std::fs::create_dir_all(f.root.join("state")).unwrap();
    let history_path = f.root.join("state/session-history-v1.tsv");
    std::fs::write(&history_path, "invalid header").unwrap();
    f.reap();
    assert_eq!(
        f.engine.service.game_session_coordinator().status().phase,
        GameSessionPhase::Ended
    );
    assert!(f.engine.can_end());
    assert!(f.engine.ensure_idle().is_err());
    f.engine.tick(&MonitorSnapshot::default());
    assert_eq!(f.engine.history_revision, 0);
    std::fs::remove_file(history_path).unwrap();
    f.engine.finish().unwrap();
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
    assert!(!f.engine.launch_locked());
}

#[test]
fn steam_helper_exit_is_not_game_exit_and_failed_scans_keep_ownership() {
    let mut f = Fixture::new();
    let mut plan = f.plan("/bin/true", &[]);
    plan.ownership = GameLaunchProcessOwnership::ForwardedSteam { app_id: Some(42) };
    f.engine.start(plan).unwrap();
    f.engine
        .active
        .as_mut()
        .unwrap()
        .child
        .child_mut()
        .wait()
        .unwrap();
    let mut monitor = MonitorSnapshot::default();
    monitor.diagnostics.game_scans = 1;
    f.engine.tick(&monitor);
    assert!(f.engine.launch_locked());
    monitor.games = vec![GameProcess {
        pid: 10,
        comm: "Fixture".into(),
        executable: "/fixture/game".into(),
        steam_app_id: Some(42),
        game_mode_active: false,
    }]
    .into();
    f.engine.tick(&monitor);
    monitor.games = vec![].into();
    monitor.diagnostics.game_detection_error = Some("fixture scan failure".into());
    f.engine.tick(&monitor);
    assert!(f.engine.launch_locked());
    monitor.diagnostics.game_detection_error = None;
    f.engine.tick(&monitor);
    assert!(!f.engine.launch_locked());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
}

#[test]
fn an_unconfirmed_steam_timeout_does_not_invent_a_completed_game() {
    let mut f = Fixture::new();
    let mut plan = f.plan("/bin/true", &[]);
    plan.ownership = GameLaunchProcessOwnership::ForwardedSteam { app_id: Some(42) };
    f.engine.start(plan).unwrap();
    let active = f.engine.active.as_mut().unwrap();
    active.child.child_mut().wait().unwrap();
    active.started = Instant::now() - (super::STEAM_START_TIMEOUT + Duration::from_secs(1));
    let mut monitor = MonitorSnapshot::default();
    monitor.diagnostics.game_scans = 1;
    f.engine.tick(&monitor);
    assert!(!f.engine.launch_locked());
    assert!(f.engine.service.session_history().unwrap().is_empty());
    assert!(f.engine.message.as_ref().unwrap().contains("not confirmed"));
}

#[test]
fn accepted_capture_evidence_confirms_a_forwarded_session_without_a_steam_app_id() {
    assert!(super::capture_evidence_confirms(None, 4, true));
    assert!(!super::capture_evidence_confirms(None, 0, false));
}

#[test]
fn a_capture_failure_does_not_release_a_still_running_direct_child() {
    let mut f = Fixture::new();
    let library = f.root.join("fixture-layer.so");
    std::fs::write(&library, b"fixture only; never loaded").unwrap();
    let config = CaptureSessionConfig::new(f.root.join("runtime"), library);
    let capture = f.engine.service.start_capture_session(&config).unwrap();
    let mut plan = f.plan("/bin/sleep", &["30"]);
    plan.capture = Some(capture);
    plan.request.metrics = true.into();
    f.engine.start(plan).unwrap();
    // No Vulkan or encoder is opened: fail the transport with a synthetic
    // observation and ensure the native process owner still retains the child.
    let _ = f
        .engine
        .service
        .game_session_coordinator()
        .observe_launch_process(&CaptureLaunchProcessState::MonitorFailed(
            "fixture failure".into(),
        ));
    f.engine.tick(&MonitorSnapshot::default());
    assert!(f.engine.launch_locked());
    f.engine.finish().unwrap();
    assert!(f
        .engine
        .active
        .as_mut()
        .unwrap()
        .child
        .child_mut()
        .try_wait()
        .unwrap()
        .is_none());
}

#[test]
fn failed_new_launch_never_displays_the_previous_games_capture() {
    let mut f = Fixture::new();
    let library = f.root.join("fixture-layer.so");
    std::fs::write(&library, b"fixture only; never loaded").unwrap();
    let capture = f
        .engine
        .service
        .start_capture_session(&CaptureSessionConfig::new(f.root.join("runtime"), library))
        .unwrap();
    f.engine.capture = Some(capture.snapshot());
    f.engine.message = Some("Old session".into());
    assert!(f
        .engine
        .start(f.plan("/no-such-redunar-game", &[]))
        .is_err());
    assert!(f.engine.capture.is_none());
    assert!(f
        .engine
        .message
        .as_ref()
        .unwrap()
        .contains("could not start"));
}

#[test]
fn unsuccessful_child_exit_is_reported_as_unsuccessful() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/false", &[])).unwrap();
    f.reap();
    assert!(f
        .engine
        .message
        .as_ref()
        .unwrap()
        .contains("unsuccessfully"));
    assert!(!f.engine.launch_locked());
}

#[test]
fn public_sessions_launch_path_runs_and_records_an_isolated_direct_game() {
    let root = std::env::temp_dir().join(format!(
        "redunar-tauri-public-launch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let service = RedunarService::with_state_directory(root.join("state"));
    // This acceptance test exercises launch/supervision/history in isolation;
    // capture is covered by the hardware probe and must not be inferred from
    // a child process that cannot expose a Vulkan producer.
    let global = redunar_core::GlobalGameProfile {
        capture_metrics: false,
        ..Default::default()
    };
    service.update_global_game_profile(global).unwrap();
    let game = service
        .add_game(redunar_daemon::AddGameRequest::new(
            "Isolated direct game",
            "/bin/true",
        ))
        .unwrap();
    let mut monitor = service.start_monitor();
    let sessions = Sessions::new(service.clone(), monitor.reader()).unwrap();
    sessions.launch(&game.id.get().to_string()).unwrap();

    // A directly launched wrapper may exit before its Vulkan producer appears.
    // Wait through the capture lifecycle's owned startup grace, plus enough
    // time for the one-second session worker cadence to observe and clean up.
    let deadline =
        std::time::Instant::now() + redunar_daemon::ARMED_STARTUP_GRACE + Duration::from_secs(2);
    while std::time::Instant::now() < deadline && sessions.read(|engine| engine.launch_locked()) {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!sessions.read(|engine| engine.launch_locked()));
    assert_eq!(service.session_history().unwrap().len(), 1);
    assert_eq!(
        service.session_history().unwrap()[0].game,
        "Isolated direct game"
    );
    sessions.shutdown();
    monitor.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires a local Vulkan display, vkcubepp, and the built capture layer"]
fn capture_enabled_vulkan_fixture_runs_through_tauri_session_supervisor() {
    let layer = std::env::var_os("REDUNAR_TAURI_CAPTURE_LAYER")
        .map(PathBuf::from)
        .expect("set REDUNAR_TAURI_CAPTURE_LAYER to the built capture library");
    assert!(
        layer.is_file(),
        "capture layer is missing: {}",
        layer.display()
    );

    let mut fixture = Fixture::new();
    fixture
        .engine
        .service
        .update_global_game_profile(GlobalGameProfile {
            capture_metrics: true,
            overlay_visible: true,
            ..GlobalGameProfile::default()
        })
        .unwrap();
    let mut request = AddGameRequest::new("Vulkan fixture", "/usr/bin/vkcubepp");
    request.launch.arguments = [
        "--c".into(),
        "120".into(),
        "--wsi".into(),
        "xcb".into(),
        "--width".into(),
        "640".into(),
        "--height".into(),
        "240".into(),
    ]
    .into_iter()
    .collect();
    let game = fixture.engine.service.add_game(request).unwrap();
    let catalog = fixture.engine.service.load_game_catalog().unwrap();
    let config = CaptureSessionConfig::new(fixture.root.join("capture-runtime"), layer);
    let capture = fixture
        .engine
        .service
        .start_capture_session(&config)
        .unwrap();
    let environment = std::env::vars_os().collect();
    let command = capture
        .game_launch_plan_for_profile(
            catalog
                .games
                .iter()
                .find(|item| item.id == game.id)
                .unwrap(),
            catalog.global_profile,
            &environment,
        )
        .unwrap()
        .command();
    let profile = game.profile.resolve(catalog.global_profile);
    fixture
        .engine
        .start(PreparedLaunch {
            command,
            capture: Some(capture),
            request: GameSessionRequest {
                game_id: Some(game.id),
                game_name: game.display_name,
                metrics: true.into(),
                overlay: true.into(),
                replay: false.into(),
            },
            profile,
            ownership: GameLaunchProcessOwnership::DirectChild,
            replay: ReplayRuntimeStatus::unavailable(profile.replay),
        })
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while fixture.engine.launch_locked() && std::time::Instant::now() < deadline {
        fixture.engine.tick(&MonitorSnapshot::default());
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !fixture.engine.launch_locked(),
        "fixture session did not finish"
    );
    let capture = fixture.engine.capture.as_ref().expect("capture snapshot");
    assert_eq!(capture.phase, redunar_daemon::CapturePhase::Completed);
    assert!(capture.received_frame_count > 0);
    assert_eq!(fixture.engine.service.session_history().unwrap().len(), 1);
}

#[test]
#[ignore = "requires a local Vulkan display, the generated scene, built capture layer, and a validated hardware encoder"]
fn replay_enabled_vulkan_fixture_saves_clip_through_tauri_session_supervisor() {
    let layer = std::env::var_os("REDUNAR_TAURI_CAPTURE_LAYER")
        .map(PathBuf::from)
        .expect("set REDUNAR_TAURI_CAPTURE_LAYER to the built capture library");
    assert!(
        layer.is_file(),
        "capture layer is missing: {}",
        layer.display()
    );

    // The runner isolates HOME/XDG_STATE_HOME so this test exercises the same
    // Tauri service constructor without touching the user's production state.
    let fixture_state = std::env::var_os("REDUNAR_TAURI_TEST_STATE")
        .map(PathBuf::from)
        .expect("use the isolated Replay acceptance runner");
    let xdg_state = PathBuf::from(std::env::var_os("XDG_STATE_HOME").expect("isolated state"));
    assert_eq!(fixture_state, xdg_state.join("redunar"));
    let beta = match std::env::var("REDUNAR_TAURI_BETA_ACCESS").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => panic!("runner must explicitly select Beta access"),
    };
    // Mirror enabling the saved switch and restarting: the production service
    // must latch the value during construction, never via a bypass flag.
    let preferences = RedunarService::with_state_directory(&fixture_state);
    preferences.set_beta_access_enabled(beta).unwrap();
    preferences.set_diagnostic_log_enabled(true).unwrap();
    drop(preferences);
    let service = RedunarService::for_tauri().expect("backend ownership");
    service.start_diagnostic_log_if_enabled();
    let output_format = match std::env::var("REDUNAR_TAURI_REPLAY_FORMAT").as_deref() {
        Ok("mp4") => redunar_daemon::ReplayOutputFormat::Mp4,
        Ok("mkv") | Err(_) => redunar_daemon::ReplayOutputFormat::Matroska,
        _ => panic!("REDUNAR_TAURI_REPLAY_FORMAT must be mkv or mp4"),
    };
    service.set_replay_output_format(output_format).unwrap();
    assert!(
        service.replay_backend_readiness().validation_allowed(),
        "the local host must pass the Replay validation gate"
    );
    service
        .update_global_game_profile(GlobalGameProfile {
            capture_metrics: true,
            overlay_visible: true,
            instant_replay: true,
            ..GlobalGameProfile::default()
        })
        .unwrap();

    // AF_UNIX capture sockets have a small platform-defined path limit. Keep
    // this isolated fixture root short enough that the runtime socket fits.
    let root = TempRoot(PathBuf::from(format!(
        "/tmp/rdr-rt-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&root.0).unwrap();
    // Reuse the failure guard: a panic must stop/join the Replay pump before
    // dropping its coordinator and remove only this fixture's runtime.
    let mut fixture = Fixture {
        root: root.0.clone(),
        engine: Engine::new(service.clone()),
    };
    let engine = &mut fixture.engine;
    let scene = PathBuf::from(
        std::env::var_os("REDUNAR_TAURI_VULKAN_SCENE").expect("runner-built Vulkan 1.1 scene"),
    );
    assert!(scene.is_absolute() && scene.is_file());
    let mut request = AddGameRequest::new("Vulkan replay fixture", scene);
    request.launch.arguments = [
        if beta { "1920" } else { "640" }.into(),
        if beta { "1080" } else { "240" }.into(),
        "1200".into(),
    ]
    .into_iter()
    .collect();
    let game = service.add_game(request).unwrap();
    let catalog = service.load_game_catalog().unwrap();
    let config = CaptureSessionConfig::new(root.0.join("capture-runtime"), layer);
    let capture = service.start_capture_session(&config).unwrap();
    let environment = std::env::vars_os().collect();
    let command = capture
        .game_launch_plan_for_profile(
            catalog
                .games
                .iter()
                .find(|item| item.id == game.id)
                .unwrap(),
            catalog.global_profile,
            &environment,
        )
        .unwrap()
        .command();
    let profile = game.profile.resolve(catalog.global_profile);
    let replay = service.replay_runtime_status().unwrap();
    assert!(replay.backend_readiness.validation_allowed());
    engine
        .start(PreparedLaunch {
            command,
            capture: Some(capture),
            request: GameSessionRequest {
                game_id: Some(game.id),
                game_name: game.display_name,
                metrics: true.into(),
                overlay: true.into(),
                replay: true.into(),
            },
            profile,
            ownership: GameLaunchProcessOwnership::DirectChild,
            replay,
        })
        .unwrap();

    let mut save_requested = false;
    let mut spool_busy_retries = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    while engine.launch_locked() && std::time::Instant::now() < deadline {
        engine.tick(&MonitorSnapshot::default());
        let status = service.game_session_coordinator().replay_runtime().status();
        if !save_requested
            && status.phase == redunar_daemon::ReplayPhase::Buffering
            && status.buffered_duration_ns >= 15_000_000_000
        {
            match service.save_replay(redunar_core::ReplayDuration::Seconds15) {
                Ok(()) => save_requested = true,
                // The live spool deliberately uses try_lock to keep a save
                // snapshot bounded. Retry only its explicit contention result;
                // never turn a storage/encoder failure into an acceptance pass.
                Err(error)
                    if error.to_string()
                        == "replay disk history failed: Replay spool is busy; retry saving"
                        && spool_busy_retries < 10 =>
                {
                    spool_busy_retries += 1;
                }
                Err(error) => panic!("save the validated rolling Replay buffer: {error}"),
            }
        }
        // The bounded scene closes its Vulkan resources normally. Killing it
        // immediately after a save skips Goodbye and the producer-exit grace.
        std::thread::sleep(Duration::from_millis(20));
    }
    if engine.launch_locked() {
        if let Some(active) = engine.active.as_mut() {
            if let LaunchProcess::Child(child) = &mut active.child {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        engine.tick(&MonitorSnapshot::default());
        let _ = engine.finish();
    }
    assert!(
        !engine.launch_locked(),
        "Replay fixture session did not finish"
    );
    assert!(
        save_requested,
        "Replay buffer never reached fifteen seconds"
    );
    eprintln!("Replay fixture accepted save after {spool_busy_retries} spool contention retries");
    let capture = engine.capture.as_ref().expect("capture snapshot");
    assert_eq!(capture.phase, redunar_daemon::CapturePhase::Completed);
    assert!(capture.received_frame_count > 0);
    assert_eq!(service.session_history().unwrap().len(), 1);
    let clips = service
        .replay_clip_inventory()
        .expect("inspect the isolated Replay clip store");
    assert_eq!(clips.len(), 1, "one saved Replay clip should be indexed");
    assert!(clips[0].bytes > 0);
    let extension = match output_format {
        redunar_daemon::ReplayOutputFormat::Matroska => ".mkv",
        redunar_daemon::ReplayOutputFormat::Mp4 => ".mp4",
    };
    assert!(clips[0].file_name.ends_with(extension));
    let clip_path = service
        .replay_save_directory()
        .unwrap()
        .join(&clips[0].file_name);
    // Decode the real saved output, with a finite subprocess deadline. Merely
    // producing a non-empty container is not encoder acceptance.
    let mut decoder = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-xerror", "-i"])
        .arg(clip_path)
        .args(["-map", "0:v:0", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("ffmpeg for decoded output validation");
    let decode_deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = decoder.try_wait().unwrap() {
            assert!(status.success(), "recorded video did not decode cleanly");
            break;
        }
        if std::time::Instant::now() >= decode_deadline {
            let _ = decoder.kill();
            let _ = decoder.wait();
            panic!("recorded video decode timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    redunar_daemon::diagnostic_log::shutdown();
}

include!("sessions_overlay_tests.rs");

#[test]
fn external_steam_uses_existing_process_and_end_retains_lock_until_exit() {
    let mut f = Fixture::new();
    let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let identity =
        redunar_daemon::ProcessIdentity::capture(Path::new("/proc"), child.id()).unwrap();
    let marker = f.root.join("unexpected-launch");
    let mut plan = f.plan("/usr/bin/touch", &[marker.to_str().unwrap()]);
    plan.ownership = GameLaunchProcessOwnership::ForwardedSteam { app_id: Some(42) };
    f.engine
        .start_with_process(plan, Some(LaunchProcess::Steam(identity)))
        .unwrap();
    assert!(
        !marker.exists(),
        "attaching must never spawn the saved Steam launch command"
    );
    assert!(f
        .engine
        .attach_steam(redunar_daemon::SteamAppId::new(43).unwrap(), child.id())
        .is_err());
    let mut monitor = MonitorSnapshot::default();
    monitor.diagnostics.game_scans = 1;
    monitor.games = vec![GameProcess {
        pid: child.id(),
        comm: "Fixture".into(),
        executable: "/fixture/game".into(),
        steam_app_id: Some(42),
        game_mode_active: false,
    }]
    .into();
    f.engine.tick(&monitor);
    f.engine.finish().unwrap();
    f.engine.finish().unwrap();
    assert!(f.engine.launch_locked());
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
    monitor.games = vec![].into();
    f.engine.tick(&monitor);
    assert!(
        f.engine.launch_locked(),
        "cached scan cannot end a live external game"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    monitor.games = vec![].into();
    f.engine.tick(&monitor);
    assert!(!f.engine.launch_locked());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
}

#[test]
fn external_steam_natural_exit_releases_session_and_notifies_background_owner() {
    let mut f = Fixture::new();
    // These are separate test-owned processes, as with a user-manager-owned
    // app and Steam's game reaper. No real app, Steam or renderer is launched.
    let mut owner = Command::new("/bin/sleep").arg("3").spawn().unwrap();
    let mut game = Command::new("/bin/cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let identity = redunar_daemon::ProcessIdentity::capture(Path::new("/proc"), game.id()).unwrap();
    let mut plan = f.plan("/bin/true", &[]);
    plan.ownership = GameLaunchProcessOwnership::ForwardedSteam { app_id: Some(42) };
    f.engine
        .start_with_process(plan, Some(LaunchProcess::Steam(identity)))
        .unwrap();
    let mut monitor = MonitorSnapshot::default();
    monitor.diagnostics.game_scans = 1;
    monitor.games = vec![GameProcess {
        pid: game.id(),
        comm: "Fixture".into(),
        executable: "/fixture/game".into(),
        steam_app_id: Some(42),
        game_mode_active: false,
    }]
    .into();
    f.engine.tick(&monitor);
    assert!(f.engine.launch_locked());
    drop(game.stdin.take()); // EOF gives the fake game a natural, successful exit.
    assert!(game.wait().unwrap().success());
    monitor.games = vec![].into();
    monitor.diagnostics.game_scans += 1;
    f.engine.tick(&monitor);
    f.engine.tick(&monitor);
    assert!(!f.engine.launch_locked());
    assert_eq!(f.engine.service.session_history().unwrap().len(), 1);
    assert_eq!(
        f.engine.service.game_session_coordinator().status().phase,
        GameSessionPhase::Ended
    );
    assert_eq!(
        f.engine.lifecycle.pending.take().unwrap().outcome,
        crate::session_lifecycle::Outcome::Finished
    );
    assert!(owner.try_wait().unwrap().is_none());
    owner.kill().unwrap();
    owner.wait().unwrap();
}

#[test]
fn external_process_identity_rejects_reused_pid_and_treats_disappearance_as_exit() {
    let root = TempRoot(std::env::temp_dir().join(format!(
        "rdstmproc-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let proc = root.0.join("42");
    std::fs::create_dir_all(&proc).unwrap();
    let write_stat = |start| {
        let mut fields = vec!["0".to_string(); 20];
        fields[0] = "S".into();
        fields[19] = format!("{start}");
        std::fs::write(
            proc.join("stat"),
            format!("42 (fixture) {}", fields.join(" ")),
        )
        .unwrap();
    };
    write_stat(100);
    let identity = redunar_daemon::ProcessIdentity::capture(&root.0, 42).unwrap();
    let mut process = LaunchProcess::Steam(identity);
    assert!(matches!(
        process.observe().0,
        CaptureLaunchProcessState::Running
    ));
    write_stat(101);
    assert!(matches!(
        process.observe().0,
        CaptureLaunchProcessState::Exited(_)
    ));
    std::fs::remove_file(proc.join("stat")).unwrap();
    assert!(matches!(
        process.observe().0,
        CaptureLaunchProcessState::Exited(_)
    ));
}

#[test]
fn update_restart_claim_is_serialized_with_launch_and_blocks_new_sessions() {
    let mut f = Fixture::new();
    f.engine.start(f.plan("/bin/sleep", &["30"])).unwrap();
    assert!(f.engine.claim_update_restart().is_err());
    assert!(!f.engine.shutdown);
    if let LaunchProcess::Child(child) = &mut f.engine.active.as_mut().unwrap().child {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    f.engine.tick(&MonitorSnapshot::default());
    assert!(f.engine.active.is_none());
    f.engine.claim_update_restart().unwrap();
    assert!(f.engine.shutdown);
    assert!(f.engine.start(f.plan("/bin/true", &[])).is_err());
}
