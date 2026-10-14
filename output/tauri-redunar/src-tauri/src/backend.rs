use redunar_daemon::{MonitorHandle, RedunarService};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

// The production service owns its runtime. Creating one per command would lose
// session identity and start competing replay-control servers on every poll.
static SERVICE: OnceLock<Mutex<Option<RedunarService>>> = OnceLock::new();
pub fn initialize() -> Result<(), String> {
    let service = RedunarService::for_tauri().map_err(|_| {
        "Redunar could not safely acquire backend ownership. Check the state directory permissions."
    })?;
    SERVICE
        .set(Mutex::new(Some(service)))
        .map_err(|_| "Backend already initialized".to_owned())
}
pub fn service() -> RedunarService {
    SERVICE
        .get()
        .expect("backend ownership acquired before use")
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .expect("backend is available until shutdown")
        .clone()
}
pub fn shutdown() {
    if let Some(slot) = SERVICE.get() {
        let service = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(service); // Join replay control and remove only this instance's socket.
    }
}

pub fn retry_ownership() -> Result<bool, String> {
    let next = RedunarService::for_tauri()
        .map_err(|_| "Redunar could not safely reacquire backend ownership".to_owned())?;
    if next.is_read_only() {
        return Ok(false);
    }
    *SERVICE
        .get()
        .expect("initialized")
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(next);
    Ok(true)
}

pub struct Monitor(pub Mutex<MonitorHandle>);
#[derive(Serialize)]
pub struct HardwareStatus {
    revision: u64,
    cpu_model: Option<String>,
    cpu_temperature_celsius: Option<f64>,
    cpu_utilization_percent: Option<f64>,
    gpu_model: Option<String>,
    gpu_temperature_celsius: Option<f64>,
    gpu_utilization_percent: Option<f64>,
    gpu_clock_mhz: Option<f64>,
    vram_used_bytes: Option<u64>,
    vram_total_bytes: Option<u64>,
    ram_used_bytes: Option<u64>,
    ram_total_bytes: Option<u64>,
    available: bool,
    read_only: bool,
    diagnostic_log_status: &'static str,
}
#[tauri::command]
pub fn daemon_status(monitor: tauri::State<'_, Monitor>) -> Result<HardwareStatus, String> {
    let snapshot = monitor
        .0
        .lock()
        .map_err(|_| "Monitor is unavailable")?
        .snapshot();
    let hardware = snapshot.hardware.as_deref();
    let gpu = hardware.and_then(|s| s.gpus.first());
    Ok(HardwareStatus {
        revision: snapshot.revision,
        read_only: other_owner(),
        diagnostic_log_status: redunar_daemon::diagnostic_log::status().code(),
        cpu_model: hardware.map(|s| s.cpu.model.clone()),
        cpu_temperature_celsius: hardware.and_then(|s| s.cpu.temperature_celsius),
        cpu_utilization_percent: hardware.and_then(|s| s.cpu.utilization_percent),
        gpu_model: gpu.map(|g| g.model.clone()),
        gpu_temperature_celsius: gpu.and_then(|g| g.temperature_celsius),
        gpu_utilization_percent: gpu.and_then(|g| g.utilization_percent),
        gpu_clock_mhz: gpu.and_then(|g| g.clock_mhz),
        vram_used_bytes: gpu.and_then(|g| g.vram_used_bytes),
        vram_total_bytes: gpu.and_then(|g| g.vram_total_bytes),
        ram_used_bytes: hardware.and_then(|s| s.memory.map(|m| m.used_bytes)),
        ram_total_bytes: hardware.and_then(|s| s.memory.map(|m| m.total_bytes)),
        available: hardware.is_some() && snapshot.diagnostics.telemetry_error.is_none(),
    })
}

#[derive(Serialize)]
pub struct DiagnosticsStatus {
    monitor_revision: u64,
    captured_at_unix_ms: Option<u64>,
    hardware_samples: u64,
    game_scans: u64,
    hardware_overruns: u64,
    game_scan_overruns: u64,
    last_hardware_sample_ms: Option<u64>,
    last_game_scan_ms: Option<u64>,
    telemetry_error: Option<String>,
    game_detection_error: Option<String>,
    capture_runtime: String,
    received_frame_count: u64,
    encoded_packet_count: u64,
    audio_packet_count: u64,
    audio_byte_count: u64,
    shortcut_state: String,
    shortcut_message: Option<String>,
    replay_readiness: ReplayReadinessStatus,
    encoder_candidates: Vec<EncoderCandidateStatus>,
}

#[derive(Serialize)]
struct ReplayReadinessStatus {
    encoded_packet_ring: bool,
    atomic_clip_store: bool,
    live_frame_transfer: bool,
    hardware_encoder: bool,
    playable_container: bool,
    performance_budget: bool,
}

#[derive(Serialize)]
struct EncoderCandidateStatus {
    driver: String,
    accessible: bool,
}

fn duration_millis(value: Option<std::time::Duration>) -> Option<u64> {
    value.and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[tauri::command]
pub fn diagnostics_snapshot(
    monitor: tauri::State<'_, Monitor>,
    shortcuts: tauri::State<'_, crate::hotkeys::ShortcutMonitor>,
) -> Result<DiagnosticsStatus, String> {
    let snapshot = monitor
        .0
        .lock()
        .map_err(|_| "Monitor is unavailable")?
        .snapshot();
    let diagnostics = &snapshot.diagnostics;
    let captured_at_unix_ms = snapshot
        .captured_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok());
    let service = service();
    let readiness = service.replay_backend_readiness();
    let probe = service.replay_hardware_encoder_probe();
    let capture_runtime = format!("{:?}", service.capture_runtime_status());
    let shortcut_status = shortcuts.snapshot();
    let replay_runtime = service
        .replay_runtime_status()
        .map_err(|error| error.to_string())?;
    Ok(DiagnosticsStatus {
        monitor_revision: snapshot.revision,
        captured_at_unix_ms,
        hardware_samples: diagnostics.hardware_samples,
        game_scans: diagnostics.game_scans,
        hardware_overruns: diagnostics.hardware_overruns,
        game_scan_overruns: diagnostics.game_scan_overruns,
        last_hardware_sample_ms: duration_millis(diagnostics.last_hardware_sample_duration),
        last_game_scan_ms: duration_millis(diagnostics.last_game_scan_duration),
        telemetry_error: diagnostics.telemetry_error.clone(),
        game_detection_error: diagnostics.game_detection_error.clone(),
        capture_runtime,
        received_frame_count: replay_runtime.received_frame_count,
        encoded_packet_count: replay_runtime.encoded_packet_count,
        audio_packet_count: replay_runtime.audio_packet_count,
        audio_byte_count: replay_runtime.audio_byte_count,
        shortcut_state: shortcut_status.state,
        shortcut_message: shortcut_status.message,
        replay_readiness: ReplayReadinessStatus {
            encoded_packet_ring: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::EncodedPacketRing),
            atomic_clip_store: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::AtomicClipStore),
            live_frame_transfer: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::LiveFrameTransfer),
            hardware_encoder: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::HardwareEncoder),
            playable_container: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::PlayableContainer),
            performance_budget: readiness
                .is_verified(redunar_daemon::ReplayBackendComponent::PerformanceBudget),
        },
        encoder_candidates: probe
            .candidates()
            .iter()
            .map(|candidate| EncoderCandidateStatus {
                driver: candidate.driver().to_owned(),
                accessible: candidate.is_accessible(),
            })
            .collect(),
    })
}

// A second process must remain read-only while the production app owns the
// control socket, preserving the active session's settings lock.
pub fn other_owner() -> bool {
    service().is_read_only()
}
pub fn ensure_write_access() -> Result<(), String> {
    if other_owner() {
        Err("Another Redunar app owns the backend session. Close it, then reopen this app before editing saved data.".into())
    } else {
        Ok(())
    }
}
