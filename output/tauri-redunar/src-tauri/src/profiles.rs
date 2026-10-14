use redunar_daemon::RedunarService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize)]
pub struct GlobalSettingsInput {
    overlay: bool,
    preset: String,
    layout: String,
    palette: String,
    #[serde(default = "default_branding_input")]
    branding: bool,
    position: String,
    scale: u8,
    opacity: u8,
    metrics: Vec<String>,
    #[serde(rename = "captureMetrics")]
    capture_metrics: bool,
    #[serde(rename = "replayEnabled")]
    replay_enabled: bool,
    fps: u16,
    quality: String,
    format: String,
    #[serde(rename = "storageLimit")]
    #[serde(default = "default_storage_limit_input")]
    storage_limit: String,
    shortcuts: std::collections::HashMap<String, String>,
}

fn default_branding_input() -> bool {
    true
}

fn replay_duration(seconds: u16) -> Result<redunar_core::ReplayDuration, String> {
    Ok(match seconds {
        15 => redunar_core::ReplayDuration::Seconds15,
        30 => redunar_core::ReplayDuration::Seconds30,
        60 => redunar_core::ReplayDuration::Seconds60,
        120 => redunar_core::ReplayDuration::Seconds120,
        180 => redunar_core::ReplayDuration::Seconds180,
        300 => redunar_core::ReplayDuration::Seconds300,
        600 => redunar_core::ReplayDuration::Seconds600,
        900 => redunar_core::ReplayDuration::Seconds900,
        _ => return Err(format!("Unsupported replay duration: {seconds}s")),
    })
}

fn global_profile_from_input(
    input: &GlobalSettingsInput,
    current: redunar_core::GlobalGameProfile,
) -> Result<redunar_core::GlobalGameProfile, String> {
    use redunar_core::{
        OverlayCorner, OverlayLayout, OverlayMetricSet, OverlayOpacity, OverlayPalette,
        OverlayPreset, OverlayScale, ReplayFrameRate, ReplayQuality,
    };
    let preset = match input.preset.as_str() {
        "Compact" => OverlayPreset::Compact,
        "FPS only" => OverlayPreset::FpsOnly,
        "Detailed" => OverlayPreset::Detailed,
        "Custom" => OverlayPreset::Custom,
        value => return Err(format!("Unsupported overlay preset: {value}")),
    };
    let corner = match input.position.as_str() {
        "Top left" => OverlayCorner::TopLeft,
        "Top right" => OverlayCorner::TopRight,
        "Bottom left" => OverlayCorner::BottomLeft,
        "Bottom right" => OverlayCorner::BottomRight,
        value => return Err(format!("Unsupported overlay position: {value}")),
    };
    let layout = match input.layout.as_str() {
        "Grid" => OverlayLayout::Grid,
        "Ribbon" => OverlayLayout::Ribbon,
        "Telemetry" => OverlayLayout::Telemetry,
        value => return Err(format!("Unsupported overlay layout: {value}")),
    };
    let palette = match input.palette.as_str() {
        "Redunar" => OverlayPalette::Redunar,
        "Glacier" => OverlayPalette::Glacier,
        "Ember" => OverlayPalette::Ember,
        "Mint" => OverlayPalette::Mint,
        "Mono" => OverlayPalette::Mono,
        "Amethyst" => OverlayPalette::Amethyst,
        "Solar" => OverlayPalette::Solar,
        "Rose" => OverlayPalette::Rose,
        value => return Err(format!("Unsupported overlay palette: {value}")),
    };
    let mut bits = 0u16;
    for metric in &input.metrics {
        bits |= match metric.as_str() {
            "FPS" => OverlayMetricSet::FPS,
            "Frame time" => OverlayMetricSet::FRAME_TIME,
            "1% low" => OverlayMetricSet::ONE_PERCENT_LOW,
            "0.1% low" => OverlayMetricSet::POINT_ONE_PERCENT_LOW,
            "GPU" => OverlayMetricSet::GPU_LOAD,
            "GPU temperature" => OverlayMetricSet::GPU_TEMPERATURE,
            "CPU" => OverlayMetricSet::CPU_LOAD,
            "CPU temperature" => OverlayMetricSet::CPU_TEMPERATURE,
            value => return Err(format!("Unsupported displayed metric: {value}")),
        };
    }
    let metrics = OverlayMetricSet::from_bits(bits).map_err(|error| error.to_string())?;
    let opacity = OverlayOpacity::new(input.opacity).map_err(|error| error.to_string())?;
    let scale = OverlayScale::new(input.scale)
        .ok_or_else(|| "Overlay scale is outside the supported range".to_owned())?;
    let frame_rate = match input.fps {
        30 => ReplayFrameRate::Fps30,
        60 => ReplayFrameRate::Fps60,
        120 => ReplayFrameRate::Fps120,
        // UI transport value; the encoder ceiling depends on dimensions.
        240 => ReplayFrameRate::Variable,
        value => return Err(format!("Unsupported replay frame rate: {value}")),
    };
    let quality = match input.quality.as_str() {
        "Efficient" => ReplayQuality::Efficient,
        "Balanced" => ReplayQuality::Balanced,
        "High" => ReplayQuality::High,
        value => return Err(format!("Unsupported replay quality: {value}")),
    };
    // A visibility save must preserve recording fields that this form does not
    // edit. An unrelated recording change would correctly trip the active
    // recording lock before the live overlay update can run.
    let storage_limit = current.replay.storage_limit;
    Ok(redunar_core::GlobalGameProfile {
        capture_metrics: input.capture_metrics,
        overlay_visible: input.overlay,
        overlay_preset: preset,
        overlay_layout: layout,
        overlay_palette: palette,
        overlay_branding: input.branding,
        overlay_metrics: metrics,
        overlay_corner: corner,
        overlay_opacity: opacity,
        overlay_scale: scale,
        gamescope_enabled: current.gamescope_enabled,
        gamemode_enabled: current.gamemode_enabled,
        replay: redunar_core::ReplaySettings {
            duration: current.replay.duration,
            frame_rate,
            quality,
            storage_limit,
        },
        // Replay activation is automatic in launch_plan. Preserve the stored
        // value so a visibility-only save never changes a locked recording field.
        instant_replay: current.instant_replay,
    })
}

#[derive(Serialize)]
pub struct GlobalWorkspace {
    values: GlobalSettingsInput,
    revision: String,
    #[serde(rename = "liveNotice", skip_serializing_if = "Option::is_none")]
    live_notice: Option<String>,
}

fn workspace(service: &RedunarService) -> Result<GlobalWorkspace, String> {
    use redunar_core::{
        OverlayCorner as C, OverlayLayout as L, OverlayMetricSet as M, OverlayPalette as A,
        OverlayPreset as P, ReplayQuality as Q,
    };
    let profile = service
        .load_game_catalog()
        .map_err(|e| e.to_string())?
        .global_profile;
    let preferences = service.replay_preferences().map_err(|e| e.to_string())?;
    let mut shortcuts = HashMap::from([("overlay".into(), preferences.overlay_shortcut.clone())]);
    for binding in &preferences.save_shortcuts {
        shortcuts.insert(
            binding.duration.seconds().to_string(),
            binding.shortcut.clone(),
        );
    }
    Ok(GlobalWorkspace {
        revision: format!("{profile:?}/{:?}", preferences.output_format),
        values: GlobalSettingsInput {
            overlay: profile.overlay_visible,
            preset: match profile.overlay_preset {
                P::Compact => "Compact",
                P::FpsOnly => "FPS only",
                P::Detailed => "Detailed",
                P::Custom => "Custom",
            }
            .into(),
            layout: match profile.overlay_layout {
                L::Grid => "Grid",
                L::Ribbon => "Ribbon",
                L::Telemetry => "Telemetry",
            }
            .into(),
            palette: match profile.overlay_palette {
                A::Redunar => "Redunar",
                A::Glacier => "Glacier",
                A::Ember => "Ember",
                A::Mint => "Mint",
                A::Mono => "Mono",
                A::Amethyst => "Amethyst",
                A::Solar => "Solar",
                A::Rose => "Rose",
            }
            .into(),
            branding: profile.overlay_branding,
            position: match profile.overlay_corner {
                C::TopLeft => "Top left",
                C::TopRight => "Top right",
                C::BottomLeft => "Bottom left",
                C::BottomRight => "Bottom right",
            }
            .into(),
            scale: profile.overlay_scale.percent(),
            opacity: profile.overlay_opacity.percent(),
            metrics: [
                ("FPS", M::FPS),
                ("Frame time", M::FRAME_TIME),
                ("1% low", M::ONE_PERCENT_LOW),
                ("0.1% low", M::POINT_ONE_PERCENT_LOW),
                ("GPU", M::GPU_LOAD),
                ("CPU", M::CPU_LOAD),
                ("GPU temperature", M::GPU_TEMPERATURE),
                ("CPU temperature", M::CPU_TEMPERATURE),
            ]
            .into_iter()
            .filter(|(_, bit)| profile.overlay_metrics.contains(*bit))
            .map(|(label, _)| label.into())
            .collect(),
            capture_metrics: profile.capture_metrics,
            replay_enabled: true,
            fps: replay_frame_rate_input(profile.replay.frame_rate),
            quality: match profile.replay.quality {
                Q::Efficient => "Efficient",
                Q::Balanced => "Balanced",
                Q::High => "High",
            }
            .into(),
            format: preferences.output_format.extension().to_uppercase(),
            storage_limit: "Unlimited".into(),
            shortcuts,
        },
        live_notice: None,
    })
}

fn default_storage_limit_input() -> String {
    "Unlimited".into()
}

#[tauri::command]
pub fn global_settings() -> Result<GlobalWorkspace, String> {
    workspace(&crate::backend::service())
}

#[tauri::command]
pub fn save_global_settings(
    input: GlobalSettingsInput,
    expected_revision: String,
    sessions: tauri::State<'_, crate::sessions::Sessions>,
) -> Result<GlobalWorkspace, String> {
    crate::backend::ensure_write_access()?;
    let service = crate::backend::service();
    let catalog = service.load_game_catalog().map_err(|e| e.to_string())?;
    let expected_format = service
        .replay_preferences()
        .map_err(|e| e.to_string())?
        .output_format;
    if format!("{:?}/{expected_format:?}", catalog.global_profile) != expected_revision {
        return Err("Global settings changed elsewhere. Reload defaults before saving.".into());
    }
    let requested = global_profile_from_input(&input, catalog.global_profile)?;
    let format = match input.format.as_str() {
        "MKV" => redunar_daemon::ReplayOutputFormat::Matroska,
        "MP4" => redunar_daemon::ReplayOutputFormat::Mp4,
        _ => return Err("Unsupported clip format".into()),
    };
    // Shortcuts have their own atomic backend write and separate Save action.
    // A shortcut failure must never leave a partially saved profile behind.
    service.save_global_workspace(catalog.global_profile, requested, expected_format, format)?;
    let live_notice = overlay_save_notice(
        "Global defaults",
        sessions.update_active_overlay_config(&requested),
    );
    let mut result = workspace(&service)?;
    result.live_notice = Some(live_notice);
    Ok(result)
}

pub(crate) fn overlay_save_notice(subject: &str, result: Result<Option<bool>, String>) -> String {
    match result {
        Ok(Some(true)) => format!("{subject} saved. The running game's overlay is set to show."),
        Ok(Some(false)) => format!("{subject} saved. The running game's overlay is set to hide."),
        Ok(None) => format!("{subject} saved for the next game launch."),
        Err(error) => {
            format!("{subject} saved, but the running overlay could not be updated: {error}")
        }
    }
}

#[tauri::command]
pub fn save_shortcuts(
    shortcuts: HashMap<String, String>,
    expected: HashMap<String, String>,
) -> Result<GlobalWorkspace, String> {
    crate::backend::ensure_write_access()?;
    let service = crate::backend::service();
    if workspace(&service)?.values.shortcuts != expected {
        return Err("Shortcuts changed elsewhere. Reload defaults before saving.".into());
    }
    let overlay = shortcuts.get("overlay").cloned().unwrap_or_default();
    let mut bindings = Vec::new();
    for (key, value) in &shortcuts {
        if key == "overlay" || value.trim().is_empty() {
            continue;
        }
        let seconds = key.parse::<u16>().map_err(|_| "Invalid replay duration")?;
        bindings.push(redunar_daemon::ReplayShortcutBinding {
            shortcut: value.clone(),
            duration: replay_duration(seconds)?,
        });
    }
    service
        .set_replay_hotkeys(overlay, bindings)
        .map_err(|e| e.to_string())?;
    workspace(&service)
}

// Keep the UI transport distinct from the nominal encoder frame rate.
const fn replay_frame_rate_input(rate: redunar_core::ReplayFrameRate) -> u16 {
    match rate {
        redunar_core::ReplayFrameRate::Variable => 240,
        fixed => fixed.frames_per_second(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> GlobalSettingsInput {
        GlobalSettingsInput {
            overlay: true,
            preset: "Custom".into(),
            layout: "Telemetry".into(),
            palette: "Glacier".into(),
            branding: true,
            position: "Top right".into(),
            scale: 100,
            opacity: 50,
            metrics: vec!["CPU temperature".into(), "GPU".into()],
            capture_metrics: true,
            replay_enabled: false,
            fps: 60,
            quality: "Balanced".into(),
            format: "MKV".into(),
            storage_limit: "10 GiB".into(),
            shortcuts: HashMap::new(),
        }
    }
    #[test]
    fn profile_edit_preserves_unrepresented_authorities() {
        let current = redunar_core::GlobalGameProfile {
            gamemode_enabled: true,
            capture_metrics: false,
            ..Default::default()
        };
        let result = global_profile_from_input(&input(), current).unwrap();
        assert!(result.gamemode_enabled);
        assert!(result.capture_metrics);
        assert_eq!(
            result.overlay_layout,
            redunar_core::OverlayLayout::Telemetry
        );
        assert_eq!(
            result.overlay_palette,
            redunar_core::OverlayPalette::Glacier
        );
        assert!(result
            .overlay_metrics
            .contains(redunar_core::OverlayMetricSet::CPU_TEMPERATURE));
        assert!(!result
            .overlay_metrics
            .contains(redunar_core::OverlayMetricSet::CPU_LOAD));
    }
    #[test]
    fn overlay_save_during_a_game_preserves_unedited_recording_settings() {
        use redunar_core::{ReplayFrameRate, ReplayStorageLimit};
        use redunar_daemon::{GameSessionRequest, ReplayRuntimeStatus};
        let root =
            std::env::temp_dir().join(format!("redunar-overlay-save-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let service = RedunarService::with_state_directory(root.join("state"));
        let mut value = input();
        let mut current = global_profile_from_input(&value, Default::default()).unwrap();
        current.replay.storage_limit = ReplayStorageLimit::GiB10;
        current.instant_replay = false;
        service.update_global_game_profile(current).unwrap();
        let format = service.replay_preferences().unwrap().output_format;
        let coordinator = service.game_session_coordinator();
        coordinator
            .begin(
                GameSessionRequest {
                    game_id: None,
                    game_name: "Fixture".into(),
                    metrics: false.into(),
                    overlay: false.into(),
                    replay: false.into(),
                },
                None,
                ReplayRuntimeStatus::unavailable(current.replay),
            )
            .unwrap();
        assert!(service.replay_recording_settings_locked());
        for visible in [false, true, false, true] {
            value.overlay = visible;
            let requested = global_profile_from_input(&value, current).unwrap();
            service
                .save_global_workspace(current, requested, format, format)
                .unwrap();
            current = service.load_game_catalog().unwrap().global_profile;
            assert_eq!(current.overlay_visible, visible);
            assert_eq!(current.replay.storage_limit, ReplayStorageLimit::GiB10);
            assert!(
                !current.instant_replay,
                "the stored replay flag is not rewritten by visibility"
            );
        }
        let mut recording_change = current;
        recording_change.replay.frame_rate = ReplayFrameRate::Fps30;
        assert!(service
            .save_global_workspace(current, recording_change, format, format)
            .is_err());
        assert_eq!(service.load_game_catalog().unwrap().global_profile, current);
        drop(coordinator);
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_profile_and_duration() {
        let mut value = input();
        value.fps = 90;
        assert!(global_profile_from_input(&value, Default::default()).is_err());
        value.fps = 60;
        value.metrics = vec!["unknown".into()];
        assert!(global_profile_from_input(&value, Default::default()).is_err());
        assert!(replay_duration(42).is_err());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn reads_actual_saved_profile_without_inventing_shortcuts() {
        let directory =
            std::env::temp_dir().join(format!("redunar-tauri-profile-{}", std::process::id()));
        let service = RedunarService::with_state_directory(&directory);
        let initial = workspace(&service).unwrap();
        assert_eq!(initial.values.shortcuts.len(), 1);
        assert_eq!(
            initial.values.shortcuts.get("overlay").map(String::as_str),
            Some("Alt+Shift+Z")
        );
        assert!(!initial.values.shortcuts.contains_key("15"));
        let mut profile = service.load_game_catalog().unwrap().global_profile;
        profile.overlay_opacity = redunar_core::OverlayOpacity::new(0).unwrap();
        profile.overlay_scale = redunar_core::OverlayScale::new(200).unwrap();
        service.update_global_game_profile(profile).unwrap();
        let reloaded = workspace(&service).unwrap();
        assert_eq!(reloaded.values.opacity, 0);
        assert_eq!(reloaded.values.scale, 200);
        assert_ne!(reloaded.revision, initial.revision);
        service.set_replay_hotkeys("".into(), Vec::new()).unwrap();
        assert_eq!(
            workspace(&service).unwrap().values.shortcuts,
            HashMap::from([("overlay".into(), String::new())])
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
