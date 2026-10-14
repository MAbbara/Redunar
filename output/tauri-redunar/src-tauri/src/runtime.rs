use serde::Serialize;

fn module_status_dto(statuses: redunar_daemon::ModuleStatuses) -> ModuleStatusDto {
    ModuleStatusDto {
        frame_metrics: format!("{:?}", statuses.frame_metrics),
        in_game_overlay: format!("{:?}", statuses.in_game_overlay),
        instant_replay: format!("{:?}", statuses.instant_replay),
    }
}

#[derive(Debug, Serialize)]
pub struct ModuleStatusDto {
    frame_metrics: String,
    in_game_overlay: String,
    instant_replay: String,
}

#[tauri::command]
pub fn module_statuses() -> Result<ModuleStatusDto, String> {
    let statuses = crate::backend::service().module_statuses();
    Ok(module_status_dto(statuses))
}

#[derive(Debug, Serialize)]
pub struct SessionHistoryItem {
    id: u64,
    game: String,
    started_unix: u64,
    duration_seconds: u32,
    average_fps: Option<f64>,
    one_percent_low_fps: Option<f64>,
    point_one_percent_low_fps: Option<f64>,
    frame_intervals_ns: Vec<u64>,
    timeline: Vec<SessionTelemetrySampleDto>,
}

#[derive(Debug, Serialize)]
pub struct SessionTelemetrySampleDto {
    elapsed_seconds: u32,
    fps: Option<f64>,
    frame_time_ms: Option<f64>,
    cpu_temperature_celsius: Option<f64>,
    gpu_temperature_celsius: Option<f64>,
    cpu_utilization_percent: Option<f64>,
    gpu_utilization_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ReplayRuntimeDto {
    phase: String,
    capability: String,
    buffered_seconds: f64,
    completed_save_revision: u64,
    backend_ready: bool,
    recorder_health: String,
    can_save: bool,
    failure: Option<String>,
    unavailable_reason: Option<String>,
    pending_reason: Option<String>,
    received_frame_count: u64,
    encoded_packet_count: u64,
    audio_packet_count: u64,
    audio_byte_count: u64,
    audio_active: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionStatusDto {
    elapsed_seconds: Option<u64>,
    captures_saved: Option<u64>,
    profile_label: Option<String>,
    feature_summary: Option<String>,
    phase: String,
    game: Option<String>,
    failure: Option<String>,
    message: Option<String>,
    launch_locked: bool,
    can_end: bool,
    history_revision: u64,
    measurements: Option<SessionMeasurements>,
}

#[derive(Debug, Serialize)]
pub struct SessionMeasurements {
    revision: u64,
    average_fps: Option<f64>,
    frame_time_ms: Option<f64>,
    one_percent_low_fps: Option<f64>,
    point_one_percent_low_fps: Option<f64>,
    frame_intervals_ns: Vec<u64>,
    phase: String,
}

#[tauri::command]
pub fn session_status(sessions: tauri::State<'_, crate::sessions::Sessions>) -> SessionStatusDto {
    sessions.read(|engine| {
        let status = crate::backend::service()
            .game_session_coordinator()
            .status();
        SessionStatusDto {
            elapsed_seconds: engine.elapsed_seconds(),
            captures_saved: engine.captures_saved(),
            profile_label: engine.elapsed_seconds().map(|_| "Session profile".into()),
            feature_summary: engine.feature_summary(),
            phase: format!("{:?}", status.phase),
            game: status.game_name,
            failure: status.failure,
            message: engine.message.clone(),
            launch_locked: engine.launch_locked(),
            can_end: engine.can_end(),
            history_revision: engine.history_revision,
            measurements: engine.capture.as_ref().map(|snapshot| SessionMeasurements {
                revision: snapshot.revision,
                average_fps: snapshot.metrics.as_ref().map(|m| m.average_fps),
                frame_time_ms: snapshot.metrics.as_ref().map(|m| m.newest_frame_time_ms),
                one_percent_low_fps: snapshot.metrics.as_ref().map(|m| m.one_percent_low_fps),
                point_one_percent_low_fps: snapshot
                    .metrics
                    .as_ref()
                    .map(|m| m.point_one_percent_low_fps),
                frame_intervals_ns: snapshot
                    .recent_frame_intervals_ns
                    .iter()
                    .copied()
                    .take(240)
                    .collect(),
                phase: format!("{:?}", snapshot.phase),
            }),
        }
    })
}

#[tauri::command]
pub fn end_session(sessions: tauri::State<'_, crate::sessions::Sessions>) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    sessions.end()
}

#[tauri::command]
pub fn replay_runtime_status() -> Result<ReplayRuntimeDto, String> {
    // Poll the cached runtime; this path must not reread the catalog or
    // reconfigure replay at the monitor cadence.
    let coordinator = crate::backend::service().game_session_coordinator();
    let status = coordinator.replay_runtime().status();
    let capture = if matches!(
        status.phase,
        redunar_daemon::ReplayPhase::Unavailable | redunar_daemon::ReplayPhase::Inactive
    ) {
        coordinator.capture_snapshot()
    } else {
        None
    };
    let rejection = capture
        .as_ref()
        .and_then(|snapshot| snapshot.replay_source_rejection);
    let active_game = matches!(
        coordinator.status().phase,
        redunar_daemon::GameSessionPhase::Launching | redunar_daemon::GameSessionPhase::Running
    );
    let reasons = replay_status_reasons(rejection, active_game, status.phase);
    Ok(ReplayRuntimeDto {
        phase: format!("{:?}", status.phase),
        capability: format!("{:?}", status.capability),
        buffered_seconds: status.buffered_duration_ns as f64 / 1_000_000_000.0,
        completed_save_revision: status.completed_save_revision,
        backend_ready: status.backend_readiness.validation_allowed(),
        recorder_health: format!("{:?}", status.recorder_health),
        can_save: status.phase == redunar_daemon::ReplayPhase::Buffering
            && status.recorder_health == redunar_daemon::ReplayRecorderHealth::Healthy
            && status.buffered_duration_ns >= 1_000_000_000,
        failure: status.last_failure.map(replay_failure_copy),
        unavailable_reason: reasons.unavailable.map(str::to_owned),
        pending_reason: reasons.pending.map(str::to_owned),
        received_frame_count: status.received_frame_count,
        encoded_packet_count: status.encoded_packet_count,
        audio_packet_count: status.audio_packet_count,
        audio_byte_count: status.audio_byte_count,
        audio_active: status.audio_active,
    })
}

fn replay_failure_copy(failure: redunar_daemon::ReplayFailure) -> String {
    use redunar_daemon::ReplayFailure;
    match failure {
        ReplayFailure::FrameSourceLost => {
            "The game stopped sending Replay frames. Restart the game through Redunar.".into()
        }
        ReplayFailure::EncoderFailed => {
            "The video encoder failed. Check Replay hardware support, then relaunch the game."
                .into()
        }
        ReplayFailure::ContainerFailed => {
            "Replay could not package the recorded video. Relaunch the game to restart recording."
                .into()
        }
        ReplayFailure::StorageFailed => {
            "The replay could not be saved. Check the replay folder and free disk space.".into()
        }
        ReplayFailure::ResourceBudgetExceeded => {
            "Replay exceeded its resource limit. Lower the recording resolution or frame rate."
                .into()
        }
    }
}

struct ReplayStatusReasons {
    unavailable: Option<&'static str>,
    pending: Option<&'static str>,
}

fn replay_status_reasons(
    rejection: Option<redunar_daemon::ReplaySourceRejection>,
    active_game: bool,
    phase: redunar_daemon::ReplayPhase,
) -> ReplayStatusReasons {
    use redunar_daemon::ReplayPhase;
    // A missing first frame is pending, not proof of incompatible hardware.
    // Explicit source rejection remains an error even with an inactive recorder.
    let unavailable = (phase == ReplayPhase::Unavailable
        || (phase == ReplayPhase::Inactive && rejection.is_some()))
    .then(|| replay_unavailable_copy(rejection, active_game, phase));
    let pending = (phase == ReplayPhase::Inactive && active_game && unavailable.is_none())
        .then_some(
        "Waiting for recordable game frames. Game startup or a display change can pause Replay.",
    );
    ReplayStatusReasons {
        unavailable,
        pending,
    }
}

fn replay_unavailable_copy(
    rejection: Option<redunar_daemon::ReplaySourceRejection>,
    active_game: bool,
    phase: redunar_daemon::ReplayPhase,
) -> &'static str {
    use redunar_daemon::ReplaySourceRejection;
    match rejection {
        Some(ReplaySourceRejection::DimensionsUnsupported) => {
            "This game resolution cannot be recorded at the selected frame rate. Lower the game's resolution or recording frame rate."
        }
        Some(ReplaySourceRejection::PixelFormatUnsupported) => {
            "This game's pixel format cannot be recorded. Try a standard 8-bit display mode."
        }
        Some(ReplaySourceRejection::ExternalMemoryUnsupported) => {
            "This OpenGL driver cannot share Replay frames with the encoder. Check graphics driver support."
        }
        Some(ReplaySourceRejection::GpuIdentityUnavailable) => {
            "Replay could not identify the GPU used by this game. Check graphics driver support and relaunch the game."
        }
        Some(
            ReplaySourceRejection::EncoderBackendUnavailable
            | ReplaySourceRejection::VideoEncodeUnsupported,
        ) => "The hardware video encoder is unavailable. Check Replay hardware support.",
        Some(ReplaySourceRejection::ProtectedSwapchain) => {
            "This game's protected display cannot be recorded."
        }
        Some(
            ReplaySourceRejection::TransferSourceUsageMissing
            | ReplaySourceRejection::ArrayLayersUnsupported
            | ReplaySourceRejection::InvalidStructure,
        ) => "This game's graphics surface cannot be recorded. Try another display mode.",
        None if active_game && phase == redunar_daemon::ReplayPhase::Unavailable => {
            "Replay hardware is unavailable for this session. Check GPU and driver support; metrics remain available."
        }
        None => "Launch a supported game through Redunar to start Replay.",
    }
}

#[cfg(test)]
mod replay_copy_tests {
    use redunar_daemon::{ReplayFailure, ReplayPhase, ReplaySourceRejection};

    #[test]
    fn awaiting_frames_is_pending_but_source_rejection_remains_unavailable() {
        let waiting = super::replay_status_reasons(None, true, ReplayPhase::Inactive);
        assert!(waiting.pending.is_some());
        assert!(waiting.unavailable.is_none());
        let idle = super::replay_status_reasons(None, false, ReplayPhase::Inactive);
        assert!(idle.pending.is_none());
        assert!(idle.unavailable.is_none());
        for phase in [ReplayPhase::Inactive, ReplayPhase::Unavailable] {
            let rejected = super::replay_status_reasons(
                Some(ReplaySourceRejection::DimensionsUnsupported),
                true,
                phase,
            );
            assert!(rejected.pending.is_none());
            assert!(rejected.unavailable.unwrap().contains("resolution"));
        }
        let unsupported = super::replay_status_reasons(None, true, ReplayPhase::Unavailable);
        assert!(unsupported.pending.is_none());
        assert!(unsupported.unavailable.unwrap().contains("hardware"));
        for phase in [
            ReplayPhase::Buffering,
            ReplayPhase::Saving,
            ReplayPhase::Failed,
        ] {
            let status = super::replay_status_reasons(None, true, phase);
            assert!(status.pending.is_none());
            assert!(status.unavailable.is_none());
        }
    }

    #[test]
    fn container_failure_is_not_reported_as_an_encoder_failure() {
        assert!(super::replay_failure_copy(ReplayFailure::EncoderFailed).contains("encoder failed"));
        let packaging = super::replay_failure_copy(ReplayFailure::ContainerFailed);
        assert!(packaging.contains("package the recorded video"));
        assert!(!packaging.contains("encoder"));
    }

    #[test]
    fn rejected_dimensions_explain_resolution_and_rate() {
        assert_eq!(
            super::replay_unavailable_copy(
                Some(redunar_daemon::ReplaySourceRejection::DimensionsUnsupported),
                true,
                redunar_daemon::ReplayPhase::Unavailable,
            ),
            "This game resolution cannot be recorded at the selected frame rate. Lower the game's resolution or recording frame rate."
        );
    }
}

#[tauri::command]
pub async fn save_replay(duration_seconds: u16) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    use redunar_core::ReplayDuration;
    let duration = match duration_seconds {
        15 => ReplayDuration::Seconds15,
        30 => ReplayDuration::Seconds30,
        60 => ReplayDuration::Seconds60,
        120 => ReplayDuration::Seconds120,
        180 => ReplayDuration::Seconds180,
        300 => ReplayDuration::Seconds300,
        600 => ReplayDuration::Seconds600,
        900 => ReplayDuration::Seconds900,
        _ => return Err(format!("Unsupported replay duration: {duration_seconds}s")),
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::backend::service()
            .save_replay(duration)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Replay save preparation stopped".to_owned())?
}

#[tauri::command]
pub fn delete_replay_clip(file_name: String) -> Result<(), String> {
    crate::backend::ensure_write_access()?;
    crate::backend::service()
        .delete_replay_clip(&file_name)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn session_history() -> Result<Vec<SessionHistoryItem>, String> {
    let records = crate::backend::service()
        .session_history()
        .map_err(|error| error.to_string())?;
    Ok(records
        .into_iter()
        .map(|item| SessionHistoryItem {
            id: item.id,
            game: item.game,
            started_unix: item.started_unix,
            duration_seconds: item.duration_seconds,
            average_fps: item.average_fps,
            one_percent_low_fps: item.one_percent_low_fps,
            point_one_percent_low_fps: item.point_one_percent_low_fps,
            frame_intervals_ns: item.frame_intervals_ns,
            timeline: item
                .timeline
                .into_iter()
                .map(|sample| SessionTelemetrySampleDto {
                    elapsed_seconds: sample.elapsed_seconds,
                    fps: sample.fps,
                    frame_time_ms: sample.frame_time_ms,
                    cpu_temperature_celsius: sample.cpu_temperature_celsius,
                    gpu_temperature_celsius: sample.gpu_temperature_celsius,
                    cpu_utilization_percent: sample.cpu_utilization_percent,
                    gpu_utilization_percent: sample.gpu_utilization_percent,
                })
                .collect(),
        })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct ReplayStorageStatusDto {
    initialized: bool,
    owned_clips: u32,
    used_bytes: u64,
    storage_limit_bytes: u64,
    bytes_over_limit: u64,
    available_bytes: Option<u64>,
    total_bytes: Option<u64>,
}

#[tauri::command]
pub fn replay_storage_status() -> Result<ReplayStorageStatusDto, String> {
    let status = crate::backend::service()
        .replay_storage_status()
        .map_err(|error| error.to_string())?;
    let capacity = redunar_daemon::ReplayClipStore::filesystem_capacity(&status.directory);
    Ok(ReplayStorageStatusDto {
        initialized: status.initialized,
        owned_clips: status.owned_clips,
        used_bytes: status.used_bytes,
        storage_limit_bytes: status.storage_limit_bytes,
        bytes_over_limit: status.bytes_over_limit,
        available_bytes: capacity.map(|c| c.0),
        total_bytes: capacity.map(|c| c.1),
    })
}

#[derive(Debug, Serialize)]
pub struct AppPreferencesDto {
    close_to_tray: bool,
    automatic_updates: bool,
    window_width: i32,
    window_height: i32,
    window_maximized: bool,
    diagnostic_log: bool,
    beta_access: bool,
    diagnostic_log_path: Option<String>,
    diagnostic_log_status: &'static str,
}

impl From<redunar_daemon::AppPreferences> for AppPreferencesDto {
    fn from(value: redunar_daemon::AppPreferences) -> Self {
        Self {
            close_to_tray: value.close_to_tray,
            automatic_updates: value.automatic_updates,
            window_width: value.window_width,
            window_height: value.window_height,
            window_maximized: value.window_maximized,
            diagnostic_log: value.diagnostic_log,
            beta_access: value.beta_access,
            diagnostic_log_path: None,
            diagnostic_log_status: redunar_daemon::diagnostic_log::status().code(),
        }
    }
}

#[tauri::command]
pub fn set_automatic_updates(enabled: bool) -> Result<AppPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    crate::backend::service()
        .set_automatic_updates_enabled(enabled)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
pub struct UpdateCheckStatusDto {
    current_version: String,
    state: String,
    message: String,
    latest_version: Option<String>,
    package_kind: Option<String>,
    asset_name: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates(refresh_pending: Option<bool>) -> UpdateCheckStatusDto {
    // Reconciliation and refresh replace shared cache files, so only the
    // process that owns the backend session may perform an update check.
    if let Err(message) = crate::backend::ensure_write_access() {
        return UpdateCheckStatusDto {
            current_version: env!("CARGO_PKG_VERSION").into(),
            state: "error".into(),
            message,
            latest_version: None,
            package_kind: None,
            asset_name: None,
        };
    }
    match tauri::async_runtime::spawn_blocking(move || {
        crate::updates::check_for_updates(refresh_pending.unwrap_or(false))
    })
    .await
    {
        Ok(status) => UpdateCheckStatusDto {
            current_version: status.current_version,
            state: status.state,
            message: status.message,
            latest_version: status.latest_version,
            package_kind: status.package_kind,
            asset_name: status.asset_name,
        },
        Err(error) => UpdateCheckStatusDto {
            current_version: env!("CARGO_PKG_VERSION").into(),
            state: "error".into(),
            message: format!("Update check could not complete: {error}"),
            latest_version: None,
            package_kind: None,
            asset_name: None,
        },
    }
}

#[derive(Debug, Serialize)]
pub struct UpdateInstallStatusDto {
    state: String,
    message: String,
}

#[tauri::command]
pub async fn install_update() -> UpdateInstallStatusDto {
    if let Err(message) = crate::backend::ensure_write_access() {
        return UpdateInstallStatusDto {
            state: "error".into(),
            message,
        };
    }
    match tauri::async_runtime::spawn_blocking(crate::updates::install_update).await {
        Ok(status) => UpdateInstallStatusDto {
            state: status.state,
            message: status.message,
        },
        Err(error) => UpdateInstallStatusDto {
            state: "error".into(),
            message: format!("Update installation could not start: {error}"),
        },
    }
}

#[tauri::command]
pub fn app_preferences() -> Result<AppPreferencesDto, String> {
    let service = crate::backend::service();
    let mut dto: AppPreferencesDto = service
        .app_preferences()
        .map(Into::into)
        .map_err(|error| error.to_string())?;
    let path = service.diagnostic_log_path();
    if path.is_file() {
        dto.diagnostic_log_path = Some(path.display().to_string());
    }
    Ok(dto)
}

#[tauri::command]
pub fn set_diagnostic_log(enabled: bool) -> Result<AppPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    crate::backend::service()
        .set_diagnostic_log_enabled(enabled)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_beta_access(enabled: bool) -> Result<AppPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    crate::backend::service()
        .set_beta_access_enabled(enabled)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

/// Open the folder containing the diagnostic log so the owner can attach it
/// to a report.
#[tauri::command]
pub fn open_diagnostic_log_folder() -> Result<(), String> {
    let path = crate::backend::service().diagnostic_log_path();
    let Some(parent) = path.parent() else {
        return Err("The diagnostic log has no folder yet".to_string());
    };
    if !path.is_file() || !parent.is_dir() {
        return Err(
            "No diagnostic log exists yet. Enable logging and restart Redunar first.".to_string(),
        );
    }
    let mut child = std::process::Command::new("xdg-open")
        .arg(parent)
        .spawn()
        .map_err(|_| "The desktop could not open the diagnostic log folder")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
pub fn set_close_to_tray(
    enabled: bool,
    app: tauri::AppHandle,
) -> Result<AppPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    let service = crate::backend::service();
    let previous = service
        .app_preferences()
        .map_err(|error| error.to_string())?
        .close_to_tray;
    crate::tray::set_enabled(&app, enabled)?;
    match service.set_close_to_tray_enabled(enabled) {
        Ok(preferences) => Ok(preferences.into()),
        Err(error) => {
            let _ = crate::tray::set_enabled(&app, previous);
            Err(error.to_string())
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReplayPreferencesDto {
    custom_save_parent: Option<String>,
    resolved_directory: String,
    output_format: String,
    overlay_shortcut: String,
    close_overlay_on_outside_click: bool,
    initial_save_duration_seconds: u16,
    microphone_mode: String,
    microphone_source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReplayDisplayCapabilityDto {
    connected_outputs: usize,
    compatible_120_modes: usize,
    maximum_width: Option<u32>,
    maximum_height: Option<u32>,
    status: String,
}

#[tauri::command]
pub fn replay_display_capability() -> ReplayDisplayCapabilityDto {
    let capability = crate::backend::service().replay_display_capability();
    ReplayDisplayCapabilityDto {
        connected_outputs: capability.connected_outputs,
        compatible_120_modes: capability.compatible_120_modes,
        maximum_width: capability.maximum_width,
        maximum_height: capability.maximum_height,
        status: capability.error.unwrap_or_else(|| {
            if capability.compatible_120_modes > 0 {
                "A compatible 120 FPS display mode was detected".into()
            } else {
                "No compatible 120 FPS display mode was detected".into()
            }
        }),
    }
}

fn replay_preferences_dto() -> Result<ReplayPreferencesDto, String> {
    let service = crate::backend::service();
    let preferences = service
        .replay_preferences()
        .map_err(|error| error.to_string())?;
    let resolved = service
        .replay_save_directory()
        .map_err(|error| error.to_string())?;
    Ok(ReplayPreferencesDto {
        custom_save_parent: preferences
            .custom_save_parent
            .map(|path| path.to_string_lossy().into_owned()),
        resolved_directory: resolved.to_string_lossy().into_owned(),
        output_format: preferences.output_format.extension().to_uppercase(),
        overlay_shortcut: preferences.overlay_shortcut,
        close_overlay_on_outside_click: preferences.close_overlay_on_outside_click,
        initial_save_duration_seconds: preferences.initial_save_duration.seconds(),
        microphone_mode: match preferences.microphone {
            redunar_daemon::MicrophoneMode::Off => "off",
            redunar_daemon::MicrophoneMode::DefaultInput => "default",
            redunar_daemon::MicrophoneMode::SelectedInput(_) => "selected",
        }
        .to_owned(),
        microphone_source: match preferences.microphone {
            redunar_daemon::MicrophoneMode::SelectedInput(source) => Some(source),
            _ => None,
        },
    })
}

#[tauri::command]
pub fn replay_preferences() -> Result<ReplayPreferencesDto, String> {
    replay_preferences_dto()
}

#[tauri::command]
pub fn set_replay_microphone(
    mode: String,
    source: Option<String>,
) -> Result<ReplayPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    let microphone = match mode.as_str() {
        "off" => redunar_daemon::MicrophoneMode::Off,
        "default" => redunar_daemon::MicrophoneMode::DefaultInput,
        "selected" => redunar_daemon::MicrophoneMode::SelectedInput(
            source.ok_or_else(|| "A microphone source is required".to_owned())?,
        ),
        _ => return Err("Unsupported microphone mode".to_owned()),
    };
    crate::backend::service()
        .set_replay_microphone(microphone)
        .map_err(|error| error.to_string())?;
    replay_preferences_dto()
}

#[tauri::command]
pub fn set_replay_save_parent(
    parent: Option<String>,
    exports: tauri::State<'_, crate::clip_export::ClipExports>,
) -> Result<ReplayPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    exports.with_idle(|| {
        crate::backend::service()
            .set_replay_save_parent(parent.map(std::path::PathBuf::from))
            .map_err(|e| e.to_string())?;
        replay_preferences_dto()
    })
}

#[tauri::command]
pub fn set_replay_overlay_behavior(
    close_on_outside_click: bool,
) -> Result<ReplayPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    crate::backend::service()
        .set_replay_outside_click(close_on_outside_click)
        .map_err(|error| error.to_string())?;
    replay_preferences_dto()
}

#[tauri::command]
pub fn set_replay_initial_save_duration(
    duration_seconds: u16,
) -> Result<ReplayPreferencesDto, String> {
    crate::backend::ensure_write_access()?;
    let duration = match duration_seconds {
        15 => redunar_core::ReplayDuration::Seconds15,
        30 => redunar_core::ReplayDuration::Seconds30,
        60 => redunar_core::ReplayDuration::Seconds60,
        120 => redunar_core::ReplayDuration::Seconds120,
        180 => redunar_core::ReplayDuration::Seconds180,
        300 => redunar_core::ReplayDuration::Seconds300,
        600 => redunar_core::ReplayDuration::Seconds600,
        900 => redunar_core::ReplayDuration::Seconds900,
        _ => return Err(format!("Unsupported replay duration: {duration_seconds}s")),
    };
    crate::backend::service()
        .set_replay_initial_save_duration(duration)
        .map_err(|error| error.to_string())?;
    replay_preferences_dto()
}

#[tauri::command]
pub fn hide_replay_menu(window: tauri::Window) -> Result<(), String> {
    // Keep the daemon's input state in lockstep with the Tauri surface. A
    // hidden window must not leave the Replay menu marked visible, otherwise
    // the next global shortcut can toggle the state in the wrong direction.
    crate::backend::service()
        .game_session_coordinator()
        .close_replay_menu();
    window.hide().map_err(|error| error.to_string())
}

#[derive(Serialize)]
pub struct MicrophoneInputDto {
    id: String,
    label: String,
}
#[tauri::command]
pub async fn microphone_inputs() -> Result<Vec<MicrophoneInputDto>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::backend::service()
            .microphone_inputs()
            .map(|inputs| {
                inputs
                    .into_iter()
                    .map(|input| MicrophoneInputDto {
                        id: input.id,
                        label: input.label,
                    })
                    .collect()
            })
            .map_err(|_| {
                "Microphone inputs are unavailable. Check the sound server and audio utilities."
                    .to_owned()
            })
    })
    .await
    .map_err(|_| "Microphone discovery stopped unexpectedly".to_owned())?
}

#[tauri::command]
pub fn pick_replay_folder() -> Result<Option<String>, String> {
    use gtk::prelude::*;
    crate::backend::ensure_write_access()?;
    let dialog = gtk::FileChooserNative::new(
        Some("Choose a replay folder"),
        None::<&gtk::Window>,
        gtk::FileChooserAction::SelectFolder,
        Some("Choose"),
        Some("Cancel"),
    );
    dialog.set_select_multiple(false);
    let result = if dialog.run() == gtk::ResponseType::Accept {
        dialog
            .filename()
            .filter(|p| p.is_absolute() && p.is_dir())
            .map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    dialog.destroy();
    Ok(result)
}
