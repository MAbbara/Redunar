//! Production startup/window calls on a private D-Bus/Xvfb session. No game,
//! monitor, input device, user state, update source or installed app is touched.
#![allow(dead_code)]
#[path = "../src/activation.rs"]
mod activation;
#[path = "../src/background_start.rs"]
mod background_start;
#[path = "steam_background/host.rs"]
mod host;
#[path = "steam_background/parking.rs"]
mod parking;
#[path = "steam_background/reopening.rs"]
mod reopening;
#[path = "../src/tray.rs"]
mod tray;
#[path = "../src/tray_native/mod.rs"]
mod tray_native;
#[path = "../src/window_lifecycle.rs"]
mod window_lifecycle;

use gtk::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

mod backend {
    pub fn service() -> &'static redunar_daemon::RedunarService {
        static SERVICE: std::sync::OnceLock<redunar_daemon::RedunarService> =
            std::sync::OnceLock::new();
        SERVICE.get_or_init(|| {
            redunar_daemon::RedunarService::with_state_directory(
                std::env::temp_dir().join(format!("redunar-steam-window-{}", std::process::id())),
            )
        })
    }
}

static PASSED: AtomicBool = AtomicBool::new(false);

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--activation-client") {
        let path = std::env::args_os().nth(2).unwrap();
        activation::request(std::path::Path::new(&path)).unwrap();
        return;
    }
    let reopen = std::env::args().any(|a| a == "--reopen");
    let park = std::env::args().any(|argument| argument == "--park");
    let cancel = std::env::args().any(|argument| argument == "--cancel-close");
    let automatic = !std::env::args().any(|argument| argument == "--manual-owner");
    let (tray_enabled, auto_focus, input, with_host) = match std::env::args().nth(1).as_deref() {
        Some("--tray-off") => (false, false, None, false),
        Some("--tray-fallback") => (true, false, None, false),
        Some("--tray-off-auto-focus") => (false, true, None, false),
        Some("--tray-off-key") => (false, true, Some("key"), false),
        Some("--tray-off-click") => (false, true, Some("click"), false),
        Some("--tray-off-host") => (false, false, None, true),
        Some("--tray-on-host") => (true, false, None, true),
        _ => {
            panic!("Use a documented --tray-off/--tray-fallback fixture mode on private D-Bus/Xvfb")
        }
    };
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    backend::service()
        .set_close_to_tray_enabled(tray_enabled)
        .unwrap();
    let mut context = tauri::generate_context!();
    for window in &mut context.config_mut().app.windows {
        window.visible = false;
        window.focus = false;
        window.url = tauri::WebviewUrl::External("about:blank".parse().unwrap());
    }
    let app = tauri::Builder::default()
        .manage(background_start::State::new(automatic))
        .setup(move |app| {
            let host = with_host.then(host::start);
            tray::setup(app)?;
            // Xvfb has no WM: pointer-root focus can activate the first mapped
            // surface. Anchor actual focus on an existing fixture window before
            // mapping Redunar, matching Steam/game startup on a desktop.
            let anchor = gtk::Window::new(gtk::WindowType::Toplevel);
            anchor.set_title("Existing fixture window");
            anchor.set_default_size(200, 120);
            anchor.show_all();
            anchor.window().unwrap().focus(0); // GDK_CURRENT_TIME
            let startup = app.handle().clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
                if automatic {
                    background_start::prepare_window(&startup);
                } else {
                    background_start::track_window_input(&startup);
                    tray::show_window(&startup);
                }
            });
            let handle = app.handle().clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1_500), move || {
                let window = handle.get_webview_window("main").unwrap();
                assert_eq!(window.is_visible().unwrap(), !with_host || !automatic, "usable tray keeps automatic startup hidden; fallback/manual stays reachable");
                assert_eq!(tray::registered(&handle), with_host && (automatic || tray_enabled));
                if with_host && automatic {
                    assert!(tray::has_item(&handle), "Steam startup owns an icon regardless of saved close behavior");
                } else if !tray_enabled {
                    assert!(!tray::has_item(&handle), "reachable fallback removes the temporary tray item");
                }
                let native = window.gtk_window().unwrap();
                if !with_host {
                    assert!(!native.gets_focus_on_map(), "fallback must not request focus");
                }
                if automatic {
                    assert!(!window.is_focused().unwrap(), "startup must remain unfocused");
                    assert!(anchor.has_toplevel_focus(), "existing window keeps focus");
                }
                assert_eq!(handle.state::<background_start::State>().may_auto_exit(), automatic);
                assert_eq!(backend::service().app_preferences().unwrap().close_to_tray, tray_enabled);
                println!("PASS: reachable unfocused startup, preference unchanged, tray_enabled={tray_enabled}, native_state={:?}", native.window().map(|window| window.state()));
                if auto_focus {
                    window.set_focus().unwrap();
                    // A bare X server has no WM to fulfill GTK's activation
                    // request. Deliver real X focus for subsequent input tests.
                    native.window().unwrap().focus(0);
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                        assert!(handle.get_webview_window("main").unwrap().is_focused().unwrap());
                        assert!(handle.state::<background_start::State>().may_auto_exit(), "compositor focus alone must not retain the automatic owner");
                        println!("PASS: automatic focus leaves background exit eligible");
                        if let Some(input) = input {
                            check_input(&handle, input);
                        } else {
                            finish(&handle, None);
                        }
                    });
                } else {
                    if reopen {
                        reopening::run(handle,host);
                    } else if park || cancel {
                        tray::show_window(&handle);
                        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                            assert!(!handle.state::<background_start::State>().may_auto_exit());
                            assert_eq!(tray::has_item(&handle), tray_enabled);
                            parking::run(handle, host, cancel);
                        });
                    } else {
                        finish(&handle, host);
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    app.run_return(|_, _| {});
    assert!(
        PASSED.load(Ordering::Acquire),
        "startup fixture did not complete"
    );
    std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("redunar-steam-window-{}", std::process::id())),
    )
    .unwrap();
}

fn finish(handle: &tauri::AppHandle, host: Option<host::Host>) {
    tray::show_window(handle);
    let handle = handle.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
        assert!(
            !handle.state::<background_start::State>().may_auto_exit(),
            "explicit Open retains the automatic owner"
        );
        assert_eq!(
            tray::has_item(&handle),
            backend::service().app_preferences().unwrap().close_to_tray,
            "Open restores the saved tray preference"
        );
        assert!(handle
            .get_webview_window("main")
            .unwrap()
            .is_visible()
            .unwrap());
        exit_probe(&handle, host);
    });
}

fn exit_probe(handle: &tauri::AppHandle, host: Option<host::Host>) {
    tray::shutdown(handle);
    drop(host);
    PASSED.store(true, Ordering::Release);
    handle.exit(0);
}

fn check_input(handle: &tauri::AppHandle, input: &'static str) {
    let window = handle.get_webview_window("main").unwrap();
    window
        .with_webview(move |webview| {
            let view = webview.inner();
            view.grab_focus();
            let native = view.toplevel().unwrap().downcast::<gtk::Window>().unwrap();
            let (x, y) = native.position();
            let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tools/send-fixture-window-input.py");
            assert!(std::process::Command::new("python3")
                .arg(helper)
                .arg(input)
                .arg((x + 100).to_string())
                .arg((y + 100).to_string())
                .status()
                .unwrap()
                .success());
        })
        .unwrap();
    let handle = handle.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
        assert!(
            !handle.state::<background_start::State>().may_auto_exit(),
            "native {input} input must retain the automatic owner"
        );
        println!("PASS: native {input} input retains the app without an explicit Open action");
        finish(&handle, None);
    });
}
