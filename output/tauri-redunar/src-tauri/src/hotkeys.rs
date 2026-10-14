//! Native global Replay shortcut activation for the Tauri shell.
//!
//! Redunar owns shortcut monitoring through its bounded evdev helper. The
//! helper runs as the same logged-in user as this app, so shortcut activation
//! never invokes Polkit or asks the desktop environment to own the bindings.
use serde::Serialize;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use tauri::Manager;

const INSTALLED_HELPER: &str = "/usr/libexec/redunar-hotkey-helper";

pub struct ShortcutMonitor {
    state: Mutex<State>,
    app_handle: Mutex<Option<tauri::AppHandle>>,
    dispatch_pending: Arc<AtomicBool>,
}

struct State {
    child: Option<Child>,
    input: Option<ChildStdin>,
    event_sender: Option<SyncSender<HelperEvent>>,
    events: Option<Receiver<HelperEvent>>,
    state: String,
    message: Option<String>,
    last_action: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            child: None,
            input: None,
            event_sender: None,
            events: None,
            state: "Inactive".into(),
            message: None,
            last_action: None,
        }
    }
}

impl Default for ShortcutMonitor {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            app_handle: Mutex::new(None),
            dispatch_pending: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum HelperEvent {
    Ready,
    KeyboardUnavailable,
    MenuOpened,
    MenuOpenedWithoutPointer,
    MenuClosed,
    Activated(u16),
    Stopped,
    Rejected(String),
    Error(String),
}

#[derive(Debug, Serialize)]
pub struct ShortcutStatusDto {
    pub(crate) state: String,
    pub(crate) message: Option<String>,
    pub(crate) last_action: Option<String>,
}

impl ShortcutMonitor {
    pub fn set_app_handle(&self, app_handle: tauri::AppHandle) {
        *lock(&self.app_handle) = Some(app_handle);
    }

    pub fn activate(&self) -> Result<ShortcutStatusDto, String> {
        crate::backend::ensure_write_access()?;
        let mut state = lock(&self.state);
        stop_locked(&mut state);
        state.state = "Starting".into();
        state.message = None;
        state.last_action = None;
        let preferences = crate::backend::service()
            .replay_preferences()
            .map_err(|error| error.to_string())?;
        if preferences.overlay_shortcut.is_empty() && preferences.save_shortcuts.is_empty() {
            state.state = "Inactive".into();
            state.message = Some("No shortcuts assigned. Save replays from the app.".into());
            return Ok(status_locked(&mut state));
        }
        let (sender, events) = mpsc::sync_channel(32);
        state.events = Some(events);
        state.event_sender = Some(sender.clone());
        if let Err(error) = start_helper_locked(
            &mut state,
            sender,
            lock(&self.app_handle).clone(),
            self.dispatch_pending.clone(),
        ) {
            state.event_sender.take();
            state.events.take();
            state.state = "Unavailable".into();
            state.message = Some(error.clone());
            return Err(error);
        }
        Ok(status_locked(&mut state))
    }

    pub fn status(&self) -> ShortcutStatusDto {
        let mut state = lock(&self.state);
        status_locked(&mut state)
    }

    fn dispatch(&self) {
        let mut state = lock(&self.state);
        let app = lock(&self.app_handle).clone();
        drain_events(&mut state, app.as_ref());
    }

    /// Return the last observed helper state without draining events or
    /// dispatching actions. Diagnostics must remain read-only.
    pub fn snapshot(&self) -> ShortcutStatusDto {
        let state = lock(&self.state);
        ShortcutStatusDto {
            state: state.state.clone(),
            message: state.message.clone(),
            last_action: state.last_action.clone(),
        }
    }

    pub fn deactivate(&self) -> ShortcutStatusDto {
        let mut state = lock(&self.state);
        stop_locked(&mut state);
        state.state = "Inactive".into();
        state.message = None;
        status_locked(&mut state)
    }

    pub fn shutdown(&self) {
        let mut state = lock(&self.state);
        stop_locked(&mut state);
    }
}

#[tauri::command]
pub fn shortcut_status(monitor: tauri::State<'_, ShortcutMonitor>) -> ShortcutStatusDto {
    monitor.status()
}

#[tauri::command]
pub fn activate_shortcuts(
    monitor: tauri::State<'_, ShortcutMonitor>,
) -> Result<ShortcutStatusDto, String> {
    monitor.activate()
}

#[tauri::command]
pub fn deactivate_shortcuts(monitor: tauri::State<'_, ShortcutMonitor>) -> ShortcutStatusDto {
    monitor.deactivate()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn status_locked(state: &mut State) -> ShortcutStatusDto {
    ShortcutStatusDto {
        state: state.state.clone(),
        message: state.message.clone(),
        last_action: state.last_action.clone(),
    }
}

fn start_helper_locked(
    state: &mut State,
    sender: SyncSender<HelperEvent>,
    app: Option<tauri::AppHandle>,
    pending: Arc<AtomicBool>,
) -> Result<(), String> {
    let preferences = crate::backend::service()
        .replay_preferences()
        .map_err(|error| error.to_string())?;
    let helper = helper_path().ok_or_else(|| {
        "Replay shortcuts are unavailable because the native helper is not installed".to_owned()
    })?;
    let mut child = Command::new(helper)
        .arg("--monitor-v1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Shortcut monitor could not start: {error}"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or("Shortcut configuration pipe is unavailable")?;
    write_bindings(&mut input, &preferences)?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Shortcut status pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Shortcut error pipe is unavailable")?;
    spawn_reader(stdout, sender.clone(), false, app.clone(), pending.clone());
    spawn_reader(stderr, sender, true, app, pending);
    state.input = Some(input);
    state.child = Some(child);
    state.state = "Starting".into();
    Ok(())
}

fn write_bindings(
    input: &mut impl Write,
    preferences: &redunar_daemon::ReplayPreferences,
) -> Result<(), String> {
    if !preferences.overlay_shortcut.is_empty() {
        writeln!(input, "TOGGLE {}", preferences.overlay_shortcut)
            .map_err(|error| format!("Shortcut configuration failed: {error}"))?;
    }
    for binding in &preferences.save_shortcuts {
        writeln!(
            input,
            "BIND {} {}",
            binding.shortcut,
            binding.duration.seconds()
        )
        .map_err(|error| format!("Shortcut configuration failed: {error}"))?;
    }
    writeln!(input, "START")
        .and_then(|()| input.flush())
        .map_err(|error| format!("Shortcut configuration failed: {error}"))
}

fn drain_events(state: &mut State, app_handle: Option<&tauri::AppHandle>) {
    let events = state
        .events
        .as_ref()
        .map(|receiver| receiver.try_iter().take(32).collect::<Vec<_>>())
        .unwrap_or_default();
    for event in events {
        match event {
            HelperEvent::Ready => {
                state.state = "Active".into();
                state.message = None;
            }
            HelperEvent::KeyboardUnavailable => {
                // Keep the helper alive: device/ACL discovery retries natively,
                // including while the webview is hidden.
                state.state = "Unavailable".into();
                state.message = Some(
                    "No readable keyboard input device. Waiting for keyboard access to return."
                        .into(),
                );
            }
            HelperEvent::MenuOpened => {
                // The helper owns the whole in-game menu session: it toggles
                // the daemon state, captures mice, and streams pointer events.
                // The panel is rendered into the captured game by the Vulkan
                // layer, so there is no desktop window to open here.
                state.last_action = Some("Replay menu opened".into());
                state.message = None;
            }
            HelperEvent::MenuOpenedWithoutPointer => {
                state.last_action = Some("Replay menu opened · View only".into());
                state.message = Some(
                    "Pointer control is unavailable for this session. Press the assigned Replay menu shortcut again to close it; save shortcuts remain active."
                        .into(),
                );
            }
            HelperEvent::MenuClosed => {
                state.last_action = Some("Replay menu closed".into());
            }
            HelperEvent::Activated(seconds) => {
                state.last_action = Some(format!("Saved replay · {seconds}s"));
                state.message = None;
                if let Some(app_handle) = app_handle {
                    crate::tray::set_tooltip(
                        app_handle,
                        &format!("Redunar · Replay saved ({seconds}s)"),
                    );
                }
            }
            HelperEvent::Stopped => {
                state.state = "Unavailable".into();
                // EOF can race the more useful permission/startup error. Keep
                // that reason so the owner can correct access and retry.
                state
                    .message
                    .get_or_insert_with(|| "Shortcut helper stopped".into());
            }
            HelperEvent::Rejected(error) => state.message = Some(error),
            HelperEvent::Error(error) => state.message = Some(error),
        }
    }
    if let Some(child) = state.child.as_mut() {
        if let Ok(Some(status)) = child.try_wait() {
            state.input.take();
            state.child.take();
            state.events.take();
            state.state = if matches!(status.code(), Some(126 | 127)) {
                "Denied".into()
            } else {
                "Unavailable".into()
            };
            if state.message.is_none() {
                state.message = Some(if state.state == "Denied" {
                    "Shortcut input access is unavailable for this user".into()
                } else {
                    "The native shortcut helper stopped".into()
                });
            }
        }
    }
}

fn stop_locked(state: &mut State) {
    state.input.take();
    if let Some(mut child) = state.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    state.event_sender.take();
    state.events.take();
}

fn helper_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("REDUNAR_HOTKEY_HELPER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    packaged_helper_path(env::current_exe().ok().as_deref())
}

fn packaged_helper_path(executable: Option<&std::path::Path>) -> Option<PathBuf> {
    // Steam clears game-specific overrides before background startup. Resolve
    // the matching build/package helper from the app's own location, so an
    // unpacked candidate cannot silently use a different installed version.
    let directory = executable.and_then(std::path::Path::parent);
    let adjacent = directory.map(|path| path.join("redunar-hotkey-helper"));
    let packaged = directory
        .and_then(std::path::Path::parent)
        .map(|path| path.join("libexec/redunar-hotkey-helper"));
    adjacent
        .into_iter()
        .chain(packaged)
        .chain(std::iter::once(PathBuf::from(INSTALLED_HELPER)))
        .find(|path| path.is_file())
}

fn spawn_reader(
    output: impl Read + Send + 'static,
    sender: SyncSender<HelperEvent>,
    errors: bool,
    app: Option<tauri::AppHandle>,
    pending: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            let event = parse_helper_event(line, errors);
            if sender.send(event).is_err() {
                break;
            }
            wake_dispatcher(app.as_ref(), &pending);
        }
        if !errors && sender.send(HelperEvent::Stopped).is_ok() {
            wake_dispatcher(app.as_ref(), &pending);
        }
    });
}

fn parse_helper_event(line: String, errors: bool) -> HelperEvent {
    if errors {
        HelperEvent::Error(line)
    } else if line.starts_with("READY ") {
        HelperEvent::Ready
    } else if line == "WAITING KEYBOARD" {
        HelperEvent::KeyboardUnavailable
    } else if line == "MENU OPENED" {
        HelperEvent::MenuOpened
    } else if line == "MENU OPENED VIEW ONLY" {
        HelperEvent::MenuOpenedWithoutPointer
    } else if line == "MENU CLOSED" {
        HelperEvent::MenuClosed
    } else if let Some(seconds) = line
        .strip_prefix("ACTIVATED ")
        .and_then(|value| value.parse::<u16>().ok())
    {
        HelperEvent::Activated(seconds)
    } else if let Some(error) = line.strip_prefix("REJECTED ") {
        HelperEvent::Rejected(error.to_owned())
    } else {
        HelperEvent::Error(line)
    }
}

fn wake_dispatcher(app: Option<&tauri::AppHandle>, pending: &Arc<AtomicBool>) {
    let Some(app) = app else {
        return;
    };
    if pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let handle = app.clone();
    let scheduled = pending.clone();
    if app
        .run_on_main_thread(move || {
            // Clear before draining so a concurrent producer can schedule the next
            // bounded batch. At most one additional callback waits on the UI loop.
            scheduled.store(false, Ordering::Release);
            handle.state::<ShortcutMonitor>().dispatch();
        })
        .is_err()
    {
        pending.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_helper_event, write_bindings, HelperEvent, ShortcutMonitor};

    #[test]
    fn background_start_uses_the_matching_local_or_unpacked_helper() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rd-helper-{}-{unique}", std::process::id()));
        let bin = root.join("usr/bin");
        let libexec = root.join("usr/libexec");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&libexec).unwrap();
        let executable = bin.join("redunar-tauri");
        let adjacent = bin.join("redunar-hotkey-helper");
        let packaged = libexec.join("redunar-hotkey-helper");
        std::fs::write(&packaged, b"fixture").unwrap();
        assert_eq!(
            super::packaged_helper_path(Some(&executable)),
            Some(packaged.clone())
        );
        std::fs::write(&adjacent, b"fixture").unwrap();
        assert_eq!(
            super::packaged_helper_path(Some(&executable)),
            Some(adjacent.clone())
        );
        std::fs::remove_file(&adjacent).unwrap();
        std::fs::create_dir(&adjacent).unwrap();
        assert_eq!(
            super::packaged_helper_path(Some(&executable)),
            Some(packaged)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn helper_exit_preserves_the_activation_failure_reason() {
        let monitor = ShortcutMonitor::default();
        let (sender, events) = std::sync::mpsc::sync_channel(32);
        super::lock(&monitor.state).events = Some(events);
        sender
            .send(HelperEvent::Error(
                "No readable keyboard input device".into(),
            ))
            .unwrap();
        sender.send(HelperEvent::Stopped).unwrap();
        monitor.dispatch();
        let status = monitor.status();
        assert_eq!(status.state, "Unavailable");
        assert_eq!(
            status.message.as_deref(),
            Some("No readable keyboard input device")
        );
    }

    #[test]
    fn keyboard_loss_and_recovery_update_status_without_webview_polling() {
        let monitor = ShortcutMonitor::default();
        let (sender, events) = std::sync::mpsc::sync_channel(32);
        super::lock(&monitor.state).events = Some(events);
        for (line, expected) in [
            ("READY 2", "Active"),
            ("WAITING KEYBOARD", "Unavailable"),
            ("READY 1", "Active"),
        ] {
            sender.send(parse_helper_event(line.into(), false)).unwrap();
            monitor.dispatch();
            let status = monitor.snapshot();
            assert_eq!(status.state, expected);
            assert_eq!(status.message.is_some(), expected == "Unavailable");
        }
    }

    #[test]
    fn diagnostic_snapshot_is_inactive_without_starting_the_helper() {
        let status = ShortcutMonitor::default().snapshot();
        assert_eq!(status.state, "Inactive");
        assert!(status.message.is_none());
        assert!(status.last_action.is_none());
    }

    #[test]
    fn dispatch_processes_events_without_a_status_read() {
        let monitor = ShortcutMonitor::default();
        let (sender, events) = std::sync::mpsc::sync_channel(32);
        super::lock(&monitor.state).events = Some(events);
        sender.send(super::HelperEvent::Ready).unwrap();
        assert_eq!(monitor.status().state, "Inactive", "status is read-only");
        monitor.dispatch();
        assert_eq!(monitor.snapshot().state, "Active");
        sender.send(super::HelperEvent::Stopped).unwrap();
        monitor.dispatch();
        assert_eq!(monitor.snapshot().state, "Unavailable");
    }

    #[test]
    fn optional_menu_and_save_assignments_do_not_emit_empty_bindings() {
        let mut preferences = redunar_daemon::ReplayPreferences::default();
        preferences.overlay_shortcut.clear();
        preferences
            .save_shortcuts
            .push(redunar_daemon::ReplayShortcutBinding {
                duration: redunar_core::ReplayDuration::Seconds30,
                shortcut: "Alt+Z".into(),
            });
        let mut protocol = Vec::new();
        write_bindings(&mut protocol, &preferences).unwrap();
        assert_eq!(
            String::from_utf8(protocol).unwrap(),
            "BIND Alt+Z 30\nSTART\n"
        );
        preferences.save_shortcuts.clear();
        let mut protocol = Vec::new();
        write_bindings(&mut protocol, &preferences).unwrap();
        assert_eq!(String::from_utf8(protocol).unwrap(), "START\n");
    }

    #[test]
    fn helper_protocol_uses_the_saved_redunar_bindings() {
        let preferences = redunar_daemon::ReplayPreferences::default();
        let mut protocol = Vec::new();
        write_bindings(&mut protocol, &preferences).expect("helper protocol");
        assert_eq!(
            String::from_utf8(protocol).expect("UTF-8 protocol"),
            "TOGGLE Alt+Shift+Z\nSTART\n"
        );
    }

    #[test]
    fn replay_shortcut_requires_runtime_capability() {
        assert!(redunar_daemon::ModuleStatus::Enabled.allows_runtime());
        assert!(!redunar_daemon::ModuleStatus::UnavailableOnSystem {
            reason: "no encoder".into(),
        }
        .allows_runtime());
        assert!(!redunar_daemon::ModuleStatus::PlannedUnavailable {
            reason: "not supported".into(),
        }
        .allows_runtime());
    }

    #[test]
    fn helper_reports_the_view_only_menu_fallback_separately() {
        assert_eq!(
            parse_helper_event("MENU OPENED VIEW ONLY".into(), false),
            HelperEvent::MenuOpenedWithoutPointer
        );
        assert_eq!(
            parse_helper_event("MENU OPENED".into(), false),
            HelperEvent::MenuOpened
        );
    }
}
