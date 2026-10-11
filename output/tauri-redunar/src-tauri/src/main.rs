#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod artwork;
mod backend;
mod background_start;
mod catalog;
mod clip_export;
mod clip_metadata;
mod diagnostic_startup;
mod hotkeys;
mod installation;
mod launch_plan;
mod media;
mod media_tools;
mod playback;
mod profiles;
mod runtime;
mod session_lifecycle;
mod sessions;
mod tray;
mod tray_native;
mod updates;
mod window_lifecycle;
use tauri::Manager;

const APP_ID: &str = "com.redunar.Redunar";
#[cfg(target_os = "linux")]
const WEBKIT_DMABUF_ENV: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

fn persistable_window_state(
    physical_width: u32,
    physical_height: u32,
    scale_factor: f64,
    maximized: bool,
) -> Option<(i32, i32, bool)> {
    if physical_width == 0
        || physical_height == 0
        || !scale_factor.is_finite()
        || scale_factor <= 0.0
    {
        return None;
    }
    // Tauri reports the live inner size in physical pixels, while the saved
    // window preference and set_size below use logical desktop coordinates.
    let logical =
        tauri::PhysicalSize::new(physical_width, physical_height).to_logical::<i32>(scale_factor);
    if logical.width <= 0 || logical.height <= 0 {
        return None;
    }
    Some((logical.width, logical.height, maximized))
}

fn restorable_window_size(
    saved_width: i32,
    saved_height: i32,
    legacy_physical: bool,
    scale_factor: f64,
) -> Option<(i32, i32)> {
    if saved_width <= 0 || saved_height <= 0 || !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    if !legacy_physical {
        return Some((saved_width, saved_height));
    }
    let logical = tauri::PhysicalSize::new(saved_width as u32, saved_height as u32)
        .to_logical::<i32>(scale_factor);
    // Older builds could persist a tiny physical window on HiDPI desktops.
    // Do not reproduce an unusably small viewport during the one-time migration.
    if logical.width < 800 || logical.height < 600 {
        let defaults = redunar_daemon::AppPreferences::default();
        Some((defaults.window_width, defaults.window_height))
    } else {
        Some((logical.width, logical.height))
    }
}

fn persist_window_state(window: &tauri::Window) {
    if backend::other_owner() {
        return;
    }
    let Ok(size) = window.inner_size() else {
        return;
    };
    let Ok(scale_factor) = window.scale_factor() else {
        return;
    };
    let Ok(maximized) = window.is_maximized() else {
        return;
    };
    let Some((width, height, maximized)) =
        persistable_window_state(size.width, size.height, scale_factor, maximized)
    else {
        return;
    };
    let _ = backend::service().set_window_state(width, height, maximized);
}

fn tauri_context(read_only_instance: bool, background: bool) -> tauri::Context<tauri::Wry> {
    let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    if background {
        if let Some(window) = context.config_mut().app.windows.first_mut() {
            window.visible = false;
            window.focus = false;
        }
    }
    if read_only_instance {
        // A second GTK application with the primary app ID reactivates the
        // first Tauri event loop. Tauri then runs setup twice and panics while
        // recreating its `main` webview. Keep the same product identity but
        // register only this read-only secondary as a non-unique GTK app.
        context.config_mut().app.enable_gtk_app_id = false;
    }
    context
}

#[cfg(target_os = "linux")]
fn configure_webkit_renderer() {
    // WebKitGTK's DMA-BUF path can crash while its WebKitWebProcess is
    // tearing down the GBM/DRM device. Keep an explicit user override, but
    // prefer the software-backed renderer for Redunar's short-lived overlay
    // and settings webviews so closing the app does not produce a system
    // crash report.
    if std::env::var_os(WEBKIT_DMABUF_ENV).is_none() {
        std::env::set_var(WEBKIT_DMABUF_ENV, "1");
    }
}

#[cfg(test)]
mod tests {
    use super::{persistable_window_state, restorable_window_size};

    #[test]
    fn window_state_persists_maximized_flag() {
        assert_eq!(
            persistable_window_state(1_280, 720, 1.0, true),
            Some((1_280, 720, true))
        );
        assert_eq!(
            persistable_window_state(1_280, 720, 1.0, false),
            Some((1_280, 720, false))
        );
    }

    #[test]
    fn zero_sized_window_is_not_persisted() {
        assert_eq!(persistable_window_state(0, 720, 1.0, false), None);
        assert_eq!(persistable_window_state(1_280, 0, 1.0, true), None);
    }

    #[test]
    fn scaled_physical_size_saves_logical_desktop_size() {
        assert_eq!(
            persistable_window_state(2_560, 1_440, 2.0, false),
            Some((1_280, 720, false))
        );
        assert_eq!(persistable_window_state(1_280, 720, 0.0, false), None);
    }

    #[test]
    fn legacy_physical_window_size_migrates_without_shrinking_the_viewport() {
        assert_eq!(
            restorable_window_size(2_560, 1_440, true, 2.0),
            Some((1_280, 720))
        );
        assert_eq!(
            restorable_window_size(1_280, 720, true, 2.0),
            Some((1_280, 720))
        );
        assert_eq!(
            restorable_window_size(1_280, 720, false, 2.0),
            Some((1_280, 720))
        );
        assert_eq!(restorable_window_size(1_280, 720, false, 0.0), None);
    }
}

fn main() {
    let background = background_start::requested(std::env::args_os().skip(1));
    if let Err(message) = backend::initialize() {
        eprintln!("{message}");
        return;
    }
    // Concurrent Steam starters must never create a secondary GTK app or
    // compete with the existing native session owner.
    if background && backend::other_owner() {
        backend::shutdown();
        return;
    }
    #[cfg(target_os = "linux")]
    configure_webkit_renderer();
    // GTK 3 otherwise advertises the executable name ("redunar-tauri") as
    // the xdg_toplevel app_id on Wayland. KDE cannot match that surface to
    // com.redunar.Redunar.desktop and falls back to its generic Wayland
    // icon. This must run before Tauri initializes GTK or creates a window.
    #[cfg(target_os = "linux")]
    gtk::glib::set_prgname(Some(APP_ID));
    // Opt-in diagnostic logging starts before the first startup message so a
    // shared report includes the whole session from process start.
    backend::service().start_diagnostic_log_if_enabled();
    redunar_daemon::diagnostic_log::log(concat!("app version=", env!("CARGO_PKG_VERSION")));
    diagnostic_startup::start();
    // A previous instance that crashed or was killed cannot run its own
    // cleanup: private capture-session sockets and multi-hundred-megabyte
    // playback copies stay behind. Sweep them once, but only while no other
    // live Redunar owns the backend session; another instance may be
    // mid-conversion, and the ownership probe caches its answer anyway.
    if !backend::other_owner() {
        let removed = backend::service().sweep_stale_capture_sessions();
        playback::sweep_stale_playback_copies();
        if removed > 0 {
            redunar_daemon::log_op!(
                "Redunar startup: removed {removed} stale capture session directories"
            );
        }
    }
    // Load the saved replay configuration once, outside the polling path.
    let _ = backend::service().replay_runtime_status();
    let monitor = backend::service().start_monitor();
    let sessions = sessions::Sessions::new(backend::service(), monitor.reader())
        .expect("Could not start the session supervisor");
    let app = tauri::Builder::default()
        .setup(move |app| {
            tray::setup(app)?;
            app.state::<hotkeys::ShortcutMonitor>()
                .set_app_handle(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(saved) = backend::service().app_preferences() {
                    if saved.window_maximized {
                        let _ = window.maximize();
                    } else if let Ok(scale_factor) = window.scale_factor() {
                        if let Some((width, height)) = restorable_window_size(
                            saved.window_width,
                            saved.window_height,
                            saved.window_size_is_physical,
                            scale_factor,
                        ) {
                            let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                                f64::from(width),
                                f64::from(height),
                            )));
                        }
                    }
                }
            }
            if background {
                // Restore geometry before hidden Steam/tray startup.
                background_start::prepare_window(app.handle());
            } else {
                background_start::track_window_input(app.handle());
            }
            session_lifecycle::install(app.handle());
            // Shortcut monitoring is native and app-lifetime scoped. If the
            // user's evdev permissions are unavailable, the rest of the app
            // remains usable and the UI reports the monitor as unavailable.
            if let Err(error) = app.state::<hotkeys::ShortcutMonitor>().activate() {
                redunar_daemon::log_op!("Redunar shortcuts unavailable: {error}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Do not write preferences from every native move/resize
                // notification. GTK can emit those events repeatedly while
                // the user drags a border, and synchronous preference I/O in
                // that path makes the compositor interaction feel sticky.
                // Capture the final usable geometry once when the window is
                // actually closing instead.
                persist_window_state(window);
                // A live game keeps its session and Replay owner even when the
                // foreground window closes. Idle close still follows preference.
                if window.label() == "main"
                    && window
                        .app_handle()
                        .state::<sessions::Sessions>()
                        .read(|engine| engine.has_live_game())
                {
                    api.prevent_close();
                    window_lifecycle::close_for_game(
                        window.app_handle(),
                        session_lifecycle::settle_closed_game,
                    );
                } else if tray::close_to_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .manage(clip_export::ClipExports::default())
        .manage(hotkeys::ShortcutMonitor::default())
        .manage(playback::Playback::default())
        .manage(backend::Monitor(std::sync::Mutex::new(monitor)))
        .manage(sessions)
        .manage(background_start::State::new(background))
        .invoke_handler(tauri::generate_handler![
            backend::daemon_status,
            backend::diagnostics_snapshot,
            runtime::app_preferences,
            runtime::set_close_to_tray,
            runtime::set_diagnostic_log,
            runtime::set_beta_access,
            runtime::open_diagnostic_log_folder,
            runtime::set_automatic_updates,
            runtime::check_for_updates,
            runtime::install_update,
            runtime::replay_preferences,
            runtime::replay_display_capability,
            runtime::set_replay_save_parent,
            runtime::set_replay_microphone,
            runtime::set_replay_overlay_behavior,
            runtime::set_replay_initial_save_duration,
            runtime::hide_replay_menu,
            clip_metadata::clip_metadata,
            catalog::catalog_games,
            installation::game_installation_statuses,
            artwork::game_poster,
            artwork::game_banner,
            catalog::pick_executable,
            catalog::add_game,
            catalog::discover_games,
            catalog::import_discovered_games,
            catalog::update_game_launch,
            catalog::remove_game,
            catalog::steam_setup_status,
            catalog::save_game_profile,
            media::replay_clips,
            media::clip_playback_path,
            media::cancel_clip_playback,
            media::clip_thumbnail,
            media::open_replay_folder,
            media::open_clip_external,
            clip_export::clip_export_status,
            clip_export::export_replay_clip,
            clip_export::cancel_clip_export,
            hotkeys::shortcut_status,
            hotkeys::activate_shortcuts,
            hotkeys::deactivate_shortcuts,
            profiles::global_settings,
            profiles::save_global_settings,
            profiles::save_shortcuts,
            runtime::session_history,
            runtime::replay_storage_status,
            runtime::module_statuses,
            runtime::session_status,
            runtime::end_session,
            sessions::launch_game,
            runtime::replay_runtime_status,
            runtime::save_replay,
            runtime::delete_replay_clip,
        ])
        .build(tauri_context(backend::other_owner(), background))
        .expect("Could not open Redunar");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { ref api, .. } = event {
            // Keep the app visible if session cleanup needs user attention.
            if let Err(error) = app.state::<sessions::Sessions>().end_for_exit() {
                api.prevent_exit();
                tray::show_window(app);
                if let Some(window) = app.get_webview_window("main") {
                    let message = serde_json::to_string(&format!(
                        "Session cleanup requires attention: {error}"
                    ))
                    .unwrap_or_default();
                    let _ = window.eval(format!(
                        "window.dispatchEvent(new CustomEvent('native-error',{{detail:{message}}}))"
                    ));
                }
            }
        }
        if let tauri::RunEvent::Exit = event {
            tray::shutdown(app);
            app.state::<sessions::Sessions>().shutdown();
            app.state::<clip_export::ClipExports>().shutdown();
            app.state::<hotkeys::ShortcutMonitor>().shutdown();
            app.state::<playback::Playback>().shutdown();
            backend::shutdown();
            if let Ok(mut monitor) = app.state::<backend::Monitor>().0.lock() {
                monitor.shutdown();
            }
            redunar_daemon::diagnostic_log::log("app event=shutdown_complete");
            redunar_daemon::diagnostic_log::shutdown();
        }
    });
}

#[cfg(test)]
mod identity_tests {
    use super::APP_ID;

    #[test]
    fn runtime_identity_matches_the_tauri_identifier() {
        let primary = super::tauri_context(false, false);
        assert_eq!(primary.config().identifier, APP_ID);
        assert!(primary.config().app.enable_gtk_app_id);
        assert!(primary.config().app.windows[0].visible);
        let background = super::tauri_context(false, true);
        assert!(!background.config().app.windows[0].visible);
        assert!(!background.config().app.windows[0].focus);
        let secondary = super::tauri_context(true, false);
        assert_eq!(secondary.config().identifier, APP_ID);
        assert!(!secondary.config().app.enable_gtk_app_id);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn webkit_renderer_workaround_uses_the_supported_environment_key() {
        assert_eq!(super::WEBKIT_DMABUF_ENV, "WEBKIT_DISABLE_DMABUF_RENDERER");
    }
}
