use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager;

pub(super) fn run(handle: tauri::AppHandle, host: Option<super::host::Host>) {
    let path = super::backend::service().desktop_activation_socket();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let activation = super::activation::Activation::bind(path.clone()).unwrap();
    let owner = handle.clone();
    activation.ready(move || {
        let app = owner.clone();
        let _ = owner.run_on_main_thread(move || super::tray::show_window(&app));
    });
    let original = handle.get_webview_window("main").unwrap();
    original.hide().unwrap();
    let complete = Arc::new(AtomicBool::new(false));
    let observed = complete.clone();
    std::thread::spawn(move || {
        // Exercise a separate real process that exits before GTK; simultaneous
        // requests also retain one native main webview and its background state.
        let binary = std::env::current_exe().unwrap();
        let clients = (0..4)
            .map(|_| {
                std::process::Command::new(&binary)
                    .arg("--activation-client")
                    .arg(&path)
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for mut client in clients {
            assert!(client.wait().unwrap().success());
        }
        observed.store(true, Ordering::Release);
    });
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1_000), move || {
        assert!(complete.load(Ordering::Acquire));
        assert_eq!(handle.webview_windows().len(), 1);
        assert!(handle
            .get_webview_window("main")
            .unwrap()
            .is_visible()
            .unwrap());
        assert!(!handle
            .state::<super::background_start::State>()
            .may_auto_exit());
        assert_eq!(
            super::tray::has_item(&handle),
            super::backend::service()
                .app_preferences()
                .unwrap()
                .close_to_tray
        );
        // Reopen an already visible/minimized main through the same production
        // action; the private X server has no WM to assert compositor focus.
        original.minimize().unwrap();
        super::tray::show_window(&handle);
        assert!(original.is_visible().unwrap());
        assert_eq!(handle.webview_windows().len(), 1);
        drop(activation);
        println!("PASS: cross-process reopening restores the original main, retains background owner and preserves tray preference");
        super::finish(&handle, host);
    });
}
