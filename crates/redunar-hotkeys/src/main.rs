use evdev::{Device, EventType, KeyCode, RelativeAxisCode};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

mod keyboards;
use keyboards::{KeyboardEvent, Keyboards};

mod pointer_capture;
use pointer_capture::{grab_all, release_all};

const SESSION_ARGUMENT: &str = "--monitor-v1";
const TRUSTED_PARENTS: &[&str] = &["/usr/bin/redunar-tauri"];
const MAX_BINDINGS: usize = 9;
const MAX_KEYBOARDS: usize = 16;
const MAX_MICE: usize = 8;
const MAX_DEVICE_LINKS_SCANNED: usize = 64;
const INPUT_EVENT_BYTES: usize = 24;
const EV_KEY: u16 = 1;
const KEY_ESC: u16 = 1;
const BTN_LEFT: u16 = 272;
const MOTION_LIMIT: i32 = 4_096;
const POINTER_BASE_GAIN_MILLI: i64 = 8_500;
const POINTER_SPEED_GAIN_MILLI: i64 = 300;
const POINTER_SPEED_LIMIT: i64 = 32;
const POINTER_VERTICAL_RATIO_MILLI: i64 = 1_568;
const KEYBOARD_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_TICK: Duration = Duration::from_millis(8);
const CHORD_BATCH_WINDOW: Duration = Duration::from_millis(4);
const MAX_CHORD_BATCH: usize = 32;
const MOTION_INTERVAL: Duration = Duration::from_millis(8);
const MENU_PING_INTERVAL: Duration = Duration::from_millis(500);
const MENU_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(30);
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_LEFTALT: u16 = 56;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_RIGHTALT: u16 = 100;
const KEY_RIGHTMETA: u16 = 126;
const MOD_CONTROL: u8 = 1 << 0;
const MOD_SHIFT: u8 = 1 << 1;
const MOD_ALT: u8 = 1 << 2;
const MOD_META: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyChord {
    key: u16,
    modifiers: u8,
    action: HotkeyAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HotkeyAction {
    ToggleOverlay,
    Save(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeyEvent {
    device: usize,
    code: u16,
    value: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerEvent {
    MotionX(i32),
    MotionY(i32),
    LeftButton(bool),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PendingMotion {
    x: i32,
    y: i32,
}

impl PendingMotion {
    fn add(&mut self, event: PointerEvent) {
        match event {
            PointerEvent::MotionX(value) => self.x = bounded_motion(self.x, value),
            PointerEvent::MotionY(value) => self.y = bounded_motion(self.y, value),
            PointerEvent::LeftButton(_) => {}
        }
    }

    fn take(&mut self) -> Option<(i32, i32)> {
        let motion = (self.x, self.y);
        *self = Self::default();
        (motion != (0, 0)).then_some(motion)
    }
}

struct MenuCapture {
    open: bool,
    pointer_grabbed: bool,
    control: Option<UnixStream>,
    pending_motion: PendingMotion,
    last_motion_sent: Instant,
    last_input: Instant,
    last_ping: Instant,
}

impl MenuCapture {
    fn closed(now: Instant) -> Self {
        Self {
            open: false,
            pointer_grabbed: false,
            control: None,
            pending_motion: PendingMotion::default(),
            last_motion_sent: now,
            last_input: now,
            last_ping: now,
        }
    }

    fn opened(&mut self, control: UnixStream, now: Instant, pointer_grabbed: bool) {
        self.open = true;
        self.pointer_grabbed = pointer_grabbed;
        self.control = Some(control);
        self.pending_motion = PendingMotion::default();
        self.last_motion_sent = now;
        self.last_input = now;
        self.last_ping = now;
    }

    fn closed_by_release(&mut self) {
        self.open = false;
        self.pointer_grabbed = false;
        self.control = None;
        self.pending_motion = PendingMotion::default();
    }

    fn request(&mut self, command: &[u8]) -> Result<String, String> {
        let stream = self
            .control
            .as_mut()
            .ok_or_else(|| "Replay menu control session is unavailable".to_owned())?;
        request_on_stream(stream, command)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Redunar hotkey helper refused to start: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(SESSION_ARGUMENT))
        || arguments.next().is_some()
    {
        return Err("an authenticated monitor argument is required".to_owned());
    }
    let uid = authenticated_parent_uid()?;
    let mut input = BufReader::new(io::stdin());
    let bindings = read_bindings(&mut input)?;
    let mut mice = Vec::new();

    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel(128);
    let keyboard_stop = Arc::clone(&stop);
    let keyboard_worker = thread::spawn(move || {
        supervise_keyboards(&keyboard_stop, &sender);
    });
    let input_stop = Arc::clone(&stop);
    let input_worker = thread::spawn(move || {
        let mut sink = [0_u8; 64];
        while input.read(&mut sink).is_ok_and(|read| read != 0) {}
        input_stop.store(true, Ordering::Release);
    });

    let control_path = PathBuf::from(format!("/run/user/{uid}/redunar/replay-control-v1.sock"));
    let result = monitor_events(
        &receiver,
        &stop,
        &bindings,
        &control_path,
        &mut mice,
        || open_mouse_devices(&mouse_devices()),
    );
    release_all(&mut mice);
    stop.store(true, Ordering::Release);
    // Dropping the bounded queue wakes a supervisor blocked on a full queue.
    drop(receiver);
    let _ = keyboard_worker.join();
    // stdin is owned by a blocking EOF watcher; the process exit closes it.
    drop(input_worker);
    result
}

fn authenticated_parent_uid() -> Result<u32, String> {
    let status = fs::read_to_string("/proc/self/status")
        .map_err(|_| "process credentials are unavailable".to_owned())?;
    let (effective_uid, parent_pid) = parse_process_identity(&status)?;
    if effective_uid == 0 {
        return Err("the shortcut helper must run as the logged-in user".to_owned());
    }
    let parent_status = fs::read_to_string(format!("/proc/{parent_pid}/status"))
        .map_err(|_| "parent process credentials are unavailable".to_owned())?;
    let (caller_uid, _) = parse_process_identity(&parent_status)?;
    if caller_uid != effective_uid {
        return Err("the shortcut helper and Redunar app must use the same user".to_owned());
    }
    let parent_executable = fs::read_link(format!("/proc/{parent_pid}/exe"))
        .map_err(|_| "parent executable is unavailable".to_owned())?;
    let trusted_parent =
        trusted_parent(&parent_executable, std::env::current_exe().ok().as_deref());
    if !trusted_parent {
        return Err("the helper was not started by the installed Redunar app".to_owned());
    }
    Ok(effective_uid)
}

fn trusted_parent(parent_executable: &Path, helper_executable: Option<&Path>) -> bool {
    if TRUSTED_PARENTS
        .iter()
        .map(Path::new)
        .any(|path| parent_executable == path)
    {
        return true;
    }

    // Local builds and unpacked packages retain the fixed app/helper layout.
    // Require this running helper's inode, not just a similarly named file.
    if parent_executable
        .file_name()
        .is_none_or(|name| name != "redunar-tauri")
    {
        return false;
    }
    let Some(helper) = helper_executable.and_then(|path| fs::metadata(path).ok()) else {
        return false;
    };
    let Some(directory) = parent_executable.parent() else {
        return false;
    };
    let adjacent = directory.join("redunar-hotkey-helper");
    let packaged = directory
        .parent()
        .map(|path| path.join("libexec/redunar-hotkey-helper"));
    std::iter::once(adjacent).chain(packaged).any(|path| {
        fs::metadata(path).is_ok_and(|candidate| {
            candidate.is_file()
                && candidate.dev() == helper.dev()
                && candidate.ino() == helper.ino()
        })
    })
}

fn parse_process_identity(status: &str) -> Result<(u32, u32), String> {
    let effective_uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|values| values.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "effective user identity is unavailable".to_owned())?;
    let parent_pid = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or_else(|| "parent process identity is unavailable".to_owned())?;
    Ok((effective_uid, parent_pid))
}

fn read_bindings(input: &mut impl BufRead) -> Result<Vec<KeyChord>, String> {
    let mut bindings = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = input
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("shortcut configuration ended before START".to_owned());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line == "START" {
            break;
        }
        if bindings.len() >= MAX_BINDINGS {
            return Err("too many shortcut bindings".to_owned());
        }
        let (shortcut, action) = if let Some(shortcut) = line.strip_prefix("TOGGLE ") {
            (shortcut, HotkeyAction::ToggleOverlay)
        } else {
            let (shortcut, seconds) = line
                .strip_prefix("BIND ")
                .and_then(|value| value.rsplit_once(' '))
                .ok_or_else(|| "invalid shortcut binding".to_owned())?;
            let seconds = seconds
                .parse::<u16>()
                .ok()
                .filter(|seconds| matches!(seconds, 15 | 30 | 60 | 120 | 180 | 300 | 600 | 900))
                .ok_or_else(|| "unsupported Replay duration".to_owned())?;
            (shortcut, HotkeyAction::Save(seconds))
        };
        let mut chord = parse_chord(shortcut)?;
        chord.action = action;
        if bindings.iter().any(|existing: &KeyChord| {
            existing.key == chord.key && existing.modifiers == chord.modifiers
        }) {
            return Err("duplicate shortcut binding".to_owned());
        }
        bindings.push(chord);
    }
    if bindings.is_empty() {
        return Err("at least one shortcut binding is required".to_owned());
    }
    if bindings
        .iter()
        .filter(|binding| binding.action == HotkeyAction::ToggleOverlay)
        .count()
        > 1
    {
        return Err("at most one Replay overlay shortcut is allowed".to_owned());
    }
    Ok(bindings)
}

fn parse_chord(shortcut: &str) -> Result<KeyChord, String> {
    let normalized = shortcut.replace(['<', '>'], "+").replace(' ', "");
    let mut chord = KeyChord {
        key: 0,
        modifiers: 0,
        action: HotkeyAction::ToggleOverlay,
    };
    for part in normalized.split('+').filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => chord.modifiers |= MOD_CONTROL,
            "shift" => chord.modifiers |= MOD_SHIFT,
            "alt" => chord.modifiers |= MOD_ALT,
            "super" | "meta" | "win" => chord.modifiers |= MOD_META,
            key if chord.key == 0 => {
                chord.key = key_code(key).ok_or_else(|| "unsupported shortcut key".to_owned())?;
            }
            _ => return Err("shortcut contains more than one non-modifier key".to_owned()),
        }
    }
    if chord.key == 0 {
        return Err("shortcut has no key".to_owned());
    }
    Ok(chord)
}

fn key_code(key: &str) -> Option<u16> {
    let code = match key {
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "0" => 11,
        "tab" => 15,
        "q" => 16,
        "w" => 17,
        "e" => 18,
        "r" => 19,
        "t" => 20,
        "y" => 21,
        "u" => 22,
        "i" => 23,
        "o" => 24,
        "p" => 25,
        "a" => 30,
        "s" => 31,
        "d" => 32,
        "f" => 33,
        "g" => 34,
        "h" => 35,
        "j" => 36,
        "k" => 37,
        "l" => 38,
        "z" => 44,
        "x" => 45,
        "c" => 46,
        "v" => 47,
        "b" => 48,
        "n" => 49,
        "m" => 50,
        "space" => 57,
        "enter" | "return" => 28,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        _ => return None,
    };
    Some(code)
}

fn keyboard_devices() -> Vec<PathBuf> {
    let mut devices = HashSet::new();
    for directory in ["/dev/input/by-id", "/dev/input/by-path"] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().take(MAX_DEVICE_LINKS_SCANNED) {
            let name = entry.file_name();
            if !name.to_string_lossy().ends_with("-event-kbd") {
                continue;
            }
            if let Ok(path) = fs::canonicalize(entry.path())
                && path.starts_with("/dev/input")
                // The directory entries can exist even when the logged-in
                // user has no ACL/group access to the evdev node. Filter
                // those nodes before announcing READY so the UI cannot show
                // an active monitor that will never receive key events.
                && File::open(&path).is_ok()
            {
                devices.insert(path);
            }
            if devices.len() >= MAX_KEYBOARDS {
                break;
            }
        }
    }
    let mut devices = devices.into_iter().collect::<Vec<_>>();
    devices.sort();
    devices
}

fn mouse_devices() -> Vec<PathBuf> {
    input_devices_with_suffix("-event-mouse", MAX_MICE)
}

fn input_devices_with_suffix(suffix: &str, limit: usize) -> Vec<PathBuf> {
    let mut devices = HashSet::new();
    for directory in ["/dev/input/by-id", "/dev/input/by-path"] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().take(MAX_DEVICE_LINKS_SCANNED) {
            if !device_link_name_matches(&entry.file_name().to_string_lossy(), suffix) {
                continue;
            }
            if let Ok(path) = fs::canonicalize(entry.path())
                && path.starts_with("/dev/input")
            {
                devices.insert(path);
            }
            if devices.len() >= limit {
                break;
            }
        }
    }
    let mut devices = devices.into_iter().collect::<Vec<_>>();
    devices.sort();
    devices.truncate(limit);
    devices
}

fn device_link_name_matches(name: &str, suffix: &str) -> bool {
    !name.contains('/') && name.ends_with(suffix)
}

fn open_mouse_devices(paths: &[PathBuf]) -> Vec<Device> {
    paths
        .iter()
        .filter_map(|path| {
            let device = Device::open(path).ok()?;
            let has_motion = device.supported_relative_axes().is_some_and(|axes| {
                axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y)
            });
            let has_left = device
                .supported_keys()
                .is_some_and(|keys| keys.contains(KeyCode::BTN_LEFT));
            (has_motion && has_left && device.set_nonblocking(true).is_ok()).then_some(device)
        })
        .take(MAX_MICE)
        .collect()
}

fn supervise_keyboards(stop: &AtomicBool, sender: &mpsc::SyncSender<KeyboardEvent>) {
    supervise_keyboard_input(
        stop,
        sender,
        || {
            keyboard_devices()
                .into_iter()
                .filter_map(|path| {
                    keyboards::identity(&path)
                        .ok()
                        .map(|identity| (path, identity))
                })
                .collect()
        },
        keyboards::open,
    );
}

fn supervise_keyboard_input<I: Read>(
    stop: &AtomicBool,
    sender: &mpsc::SyncSender<KeyboardEvent>,
    mut discover: impl FnMut() -> Vec<(PathBuf, keyboards::Identity)>,
    mut open: impl FnMut(&Path) -> io::Result<(keyboards::Identity, I)>,
) {
    let mut keyboards = Keyboards::new();
    let mut next_refresh = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let mut events = Vec::new();
        if now >= next_refresh {
            let candidates = discover();
            events.extend(keyboards.refresh(&candidates, &mut open));
            next_refresh = now + KEYBOARD_REFRESH_INTERVAL;
        }
        events.extend(keyboards.poll());
        for event in events {
            if stop.load(Ordering::Acquire) || sender.send(event).is_err() {
                return;
            }
        }
        thread::sleep(EVENT_TICK);
    }
}

fn monitor_events(
    receiver: &mpsc::Receiver<KeyboardEvent>,
    stop: &AtomicBool,
    bindings: &[KeyChord],
    control_path: &Path,
    mice: &mut Vec<Device>,
    mut refresh_mice: impl FnMut() -> Vec<Device>,
) -> Result<(), String> {
    let mut pressed = HashSet::new();
    let mut menu = MenuCapture::closed(Instant::now());
    while !stop.load(Ordering::Acquire) {
        match receiver.recv_timeout(EVENT_TICK) {
            Ok(event) => {
                let events = chord_event_batch(receiver, event);
                for event in events {
                    let event = match event {
                        KeyboardEvent::Key(event) => event,
                        KeyboardEvent::Removed(device) => {
                            // A missed release must not suppress the replacement
                            // keyboard or strand the menu's exclusive mouse grab.
                            pressed.retain(|&(generation, _)| generation != device);
                            if menu.open {
                                close_menu(control_path, mice, &mut menu, b"MENU ESCAPE\n");
                            }
                            continue;
                        }
                        KeyboardEvent::Availability(count) => {
                            if count == 0 {
                                println!("WAITING KEYBOARD");
                            } else {
                                println!("READY {count}");
                            }
                            io::stdout().flush().map_err(|error| error.to_string())?;
                            continue;
                        }
                    };
                    let identity = (event.device, event.code);
                    // Escape is the menu's recovery path as well as its normal
                    // dismissal key. Handle it before composite-device press
                    // suppression so a missed release on one keyboard interface
                    // cannot strand an open menu. Some evdev devices report an
                    // already-held key as value 2, which must close it too.
                    if menu.open && escape_closes_menu(event) {
                        if event.value == 1 {
                            pressed.insert(identity);
                        }
                        close_menu(control_path, mice, &mut menu, b"MENU ESCAPE\n");
                        continue;
                    }
                    match event.value {
                        0 => {
                            pressed.remove(&identity);
                        }
                        1 => {
                            let first_logical_press = logical_press_is_new(&pressed, event.code);
                            pressed.insert(identity);
                            // Composite USB keyboards can expose the same physical
                            // keys through more than one `-event-kbd` interface.
                            // Treat those duplicate edges as one logical press so
                            // an overlay toggle cannot immediately toggle itself
                            // closed and a save cannot be submitted twice.
                            if !first_logical_press {
                                continue;
                            }
                            let modifiers = active_modifiers(&pressed);
                            let action = bindings
                                .iter()
                                .find(|binding| {
                                    binding.key == event.code && binding.modifiers == modifiers
                                })
                                .map(|binding| binding.action);
                            // The Replay menu is mouse-only. Any unrelated
                            // non-modifier key closes it before that same event is
                            // handled by the game or desktop (notably Alt+Tab), so
                            // an evdev mouse grab can never strand app switching.
                            if menu.open
                                && action != Some(HotkeyAction::ToggleOverlay)
                                && !is_modifier_key(event.code)
                            {
                                close_menu(control_path, mice, &mut menu, b"MENU ESCAPE\n");
                            }
                            if let Some(action) = action {
                                match action {
                                    HotkeyAction::ToggleOverlay => {
                                        if !menu.open {
                                            // Input nodes can disappear while
                                            // a wireless mouse reconnects.
                                            // Replace ungrabbed handles at
                                            // each opening, so a dead startup
                                            // handle cannot cancel the menu.
                                            *mice = refresh_mice();
                                        }
                                        // The helper owns the whole in-game
                                        // menu session: it asks the daemon to
                                        // toggle, grabs every mouse on success
                                        // so the game receives no pointer input,
                                        // and streams MENU MOVE/BUTTON updates
                                        // until the menu closes.
                                        toggle_menu(control_path, mice, &mut menu);
                                    }
                                    HotkeyAction::Save(seconds) => {
                                        let command = format!("SAVE {seconds}\n");
                                        match request_save(control_path, command.as_bytes()) {
                                            Ok(()) => println!("ACTIVATED {seconds}"),
                                            Err(error) => println!("REJECTED {error}"),
                                        }
                                    }
                                }
                                io::stdout().flush().map_err(|error| error.to_string())?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if menu.open && !process_mouse_events(mice, &mut menu) {
            // A failed pointer request used to clear only the helper's local
            // state, leaving the Vulkan menu visible while Escape was ignored.
            // Close through the owner so both sides make the same transition.
            close_menu(control_path, mice, &mut menu, b"MENU ESCAPE\n");
        }
        if menu.open && watchdog_action(&menu, Instant::now()) != WatchdogAction::None {
            close_menu(control_path, mice, &mut menu, b"MENU ESCAPE\n");
        }
    }
    release_all(mice);
    Ok(())
}

fn request_control(control_path: &Path, command: &[u8]) -> Result<String, String> {
    let mut socket = UnixStream::connect(control_path)
        .map_err(|error| format!("Replay control connection failed: {error}"))?;
    request_on_stream(&mut socket, command)
}

fn request_on_stream(socket: &mut UnixStream, command: &[u8]) -> Result<String, String> {
    socket
        .set_read_timeout(Some(CONTROL_TIMEOUT))
        .map_err(|error| error.to_string())?;
    socket
        .set_write_timeout(Some(CONTROL_TIMEOUT))
        .map_err(|error| error.to_string())?;
    socket
        .write_all(command)
        .map_err(|error| format!("Replay control request failed: {error}"))?;
    let mut response = String::new();
    BufReader::new(socket)
        .read_line(&mut response)
        .map_err(|error| format!("Replay control response failed: {error}"))?;
    if let Some(error) = response.strip_prefix("ERROR ") {
        return Err(error.trim_end().to_owned());
    }
    if response != "OK\n" {
        if response == "OK GRAB\n" || response == "OK RELEASE\n" {
            return Ok(response);
        }
        return Err("Replay control returned an invalid response".to_owned());
    }
    Ok(response)
}

fn request_save(control_path: &Path, command: &[u8]) -> Result<(), String> {
    match request_control(control_path, command)? {
        response if response == "OK\n" => Ok(()),
        _ => Err("Replay control returned an invalid save response".to_owned()),
    }
}

fn toggle_menu(control_path: &Path, mice: &mut [Device], menu: &mut MenuCapture) {
    if menu.open {
        close_menu(control_path, mice, menu, b"MENU TOGGLE\n");
        return;
    }
    let mut control = match UnixStream::connect(control_path) {
        Ok(control) => control,
        Err(error) => {
            println!("REJECTED Replay control connection failed: {error}");
            return;
        }
    };
    match request_on_stream(&mut control, b"MENU TOGGLE\n") {
        Ok(response) if response == "OK GRAB\n" => {
            if mice.is_empty() {
                // Keyboard devices commonly receive a logind ACL while raw
                // mouse event nodes do not. The menu is still useful as an
                // in-game status surface and the configured save shortcut
                // remains available, so lack of pointer access must not turn
                // a valid Shift+F8 press into a no-op.
                menu.opened(control, Instant::now(), false);
                println!("MENU OPENED VIEW ONLY");
            } else {
                match grab_all(mice) {
                    Ok(()) => {
                        menu.opened(control, Instant::now(), true);
                        // Report the transition so the app can show honest
                        // shortcut activity. The menu itself is rendered into
                        // the captured game by the Vulkan layer; no desktop
                        // window is involved.
                        println!("MENU OPENED");
                    }
                    Err(error) => {
                        // Pointer ownership is optional; the menu itself is
                        // rendered independently in the game's swapchain. A
                        // transient EVIOCGRAB failure must not erase a valid
                        // hotkey action or make the panel flash closed.
                        menu.opened(control, Instant::now(), false);
                        println!("MENU OPENED VIEW ONLY");
                        eprintln!("Replay menu pointer capture unavailable: {error}");
                    }
                }
            }
        }
        Ok(_) => println!("REJECTED Replay menu returned an invalid open response"),
        Err(error) => println!("REJECTED {error}"),
    }
}

fn chord_event_batch(
    receiver: &mpsc::Receiver<KeyboardEvent>,
    first: KeyboardEvent,
) -> Vec<KeyboardEvent> {
    let mut events = Vec::with_capacity(8);
    events.push(first);
    let deadline = Instant::now() + CHORD_BATCH_WINDOW;
    while events.len() < MAX_CHORD_BATCH {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(event) => events.push(event),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Composite keyboards may publish modifier and function-key edges from
    // different event interfaces. Reader-thread scheduling is not a chord
    // ordering contract, so apply modifier presses first and releases last
    // within this tiny bounded window.
    // Lifecycle messages are barriers: no press from an old generation may
    // be reordered past its removal. Sort only contiguous keyboard edges.
    let mut start = 0;
    for end in 0..=events.len() {
        if end == events.len() || !matches!(events[end], KeyboardEvent::Key(_)) {
            events[start..end].sort_by_key(|event| {
                let KeyboardEvent::Key(event) = event else {
                    unreachable!()
                };
                let priority = match (event.value, is_modifier_key(event.code)) {
                    (1, true) => 0,
                    (1, false) => 1,
                    (2, _) => 2,
                    (0, false) => 3,
                    (0, true) => 4,
                    _ => 5,
                };
                (priority, event.device, event.code)
            });
            start = end + 1;
        }
    }
    events
}

fn close_menu(control_path: &Path, mice: &mut [Device], menu: &mut MenuCapture, command: &[u8]) {
    // Release locally even when the UI/control owner has disappeared. The
    // kernel also releases EVIOCGRAB when these file descriptors are dropped.
    let _ = menu
        .request(command)
        .or_else(|_| request_control(control_path, command));
    release_all(mice);
    menu.closed_by_release();
    println!("MENU CLOSED");
}

fn process_mouse_events(mice: &mut [Device], menu: &mut MenuCapture) -> bool {
    process_pointer_input(menu, || {
        let mut pointer_events = Vec::new();
        for mouse in mice {
            match mouse.fetch_events() {
                Ok(events) => pointer_events.extend(events.filter_map(|event| {
                    parse_pointer_event(event.event_type().0, event.code(), event.value())
                })),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        Ok(pointer_events)
    })
}

fn process_pointer_input(
    menu: &mut MenuCapture,
    read_events: impl FnOnce() -> io::Result<Vec<PointerEvent>>,
) -> bool {
    // View-only sessions retain heartbeats, but never forward gameplay input
    // from mice that the helper does not own (including failed-grab fallback).
    let pointer_events = if menu.pointer_grabbed {
        match read_events() {
            Ok(events) => events,
            Err(_) => return false,
        }
    } else {
        Vec::new()
    };
    for event in pointer_events {
        menu.last_input = Instant::now();
        match event {
            PointerEvent::MotionX(_) | PointerEvent::MotionY(_) => menu.pending_motion.add(event),
            PointerEvent::LeftButton(pressed) => {
                if !send_menu_button(menu, pressed).unwrap_or(false) {
                    return false;
                }
            }
        }
    }
    let now = Instant::now();
    if now.duration_since(menu.last_motion_sent) >= MOTION_INTERVAL
        && let Some((x, y)) = menu.pending_motion.take()
    {
        let (x, y) = accelerated_pointer_motion(x, y);
        let command = format!("MENU MOVE {x} {y}\n");
        if !request_keeps_menu_open(menu, command.as_bytes()).unwrap_or(false) {
            return false;
        }
        menu.last_motion_sent = now;
    }
    if now.duration_since(menu.last_ping) >= MENU_PING_INTERVAL {
        if !request_keeps_menu_open(menu, b"MENU PING\n").unwrap_or(false) {
            return false;
        }
        menu.last_ping = now;
    }
    true
}

fn send_menu_button(menu: &mut MenuCapture, pressed: bool) -> Result<bool, String> {
    request_keeps_menu_open(
        menu,
        if pressed {
            b"MENU BUTTON 1\n"
        } else {
            b"MENU BUTTON 0\n"
        },
    )
}

fn request_keeps_menu_open(menu: &mut MenuCapture, command: &[u8]) -> Result<bool, String> {
    menu.request(command)
        .map(|response| response != "OK RELEASE\n")
}

fn parse_pointer_event(event_type: u16, code: u16, value: i32) -> Option<PointerEvent> {
    match (event_type, code, value) {
        (kind, 0, value) if kind == EventType::RELATIVE.0 => Some(PointerEvent::MotionX(value)),
        (kind, 1, value) if kind == EventType::RELATIVE.0 => Some(PointerEvent::MotionY(value)),
        (EV_KEY, BTN_LEFT, 0) => Some(PointerEvent::LeftButton(false)),
        (EV_KEY, BTN_LEFT, 1) => Some(PointerEvent::LeftButton(true)),
        _ => None,
    }
}

fn bounded_motion(current: i32, delta: i32) -> i32 {
    current
        .saturating_add(delta)
        .clamp(-MOTION_LIMIT, MOTION_LIMIT)
}

fn accelerated_pointer_motion(x: i32, y: i32) -> (i32, i32) {
    let speed = i64::from(x.unsigned_abs().max(y.unsigned_abs())).min(POINTER_SPEED_LIMIT);
    let gain = POINTER_BASE_GAIN_MILLI + speed * POINTER_SPEED_GAIN_MILLI;
    let scaled = |value: i32, axis_ratio: i64| {
        let numerator = i64::from(value)
            .saturating_mul(gain)
            .saturating_mul(axis_ratio);
        let denominator = 1_000_000_i64;
        let rounded = if numerator >= 0 {
            numerator.saturating_add(denominator / 2)
        } else {
            numerator.saturating_sub(denominator / 2)
        } / denominator;
        i32::try_from(rounded)
            .unwrap_or(if rounded < 0 { i32::MIN } else { i32::MAX })
            .clamp(-MOTION_LIMIT, MOTION_LIMIT)
    };
    (scaled(x, 1_000), scaled(y, POINTER_VERTICAL_RATIO_MILLI))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WatchdogAction {
    None,
    Inactive,
}

fn watchdog_action(menu: &MenuCapture, now: Instant) -> WatchdogAction {
    if menu.open && now.duration_since(menu.last_input) >= MENU_INACTIVITY_TIMEOUT {
        WatchdogAction::Inactive
    } else {
        WatchdogAction::None
    }
}

fn active_modifiers(pressed: &HashSet<(usize, u16)>) -> u8 {
    let mut active = HashMap::new();
    for &(_, code) in pressed {
        active.insert(code, true);
    }
    let mut modifiers = 0;
    if active.contains_key(&KEY_LEFTCTRL) || active.contains_key(&KEY_RIGHTCTRL) {
        modifiers |= MOD_CONTROL;
    }
    if active.contains_key(&KEY_LEFTSHIFT) || active.contains_key(&KEY_RIGHTSHIFT) {
        modifiers |= MOD_SHIFT;
    }
    if active.contains_key(&KEY_LEFTALT) || active.contains_key(&KEY_RIGHTALT) {
        modifiers |= MOD_ALT;
    }
    if active.contains_key(&KEY_LEFTMETA) || active.contains_key(&KEY_RIGHTMETA) {
        modifiers |= MOD_META;
    }
    modifiers
}

fn logical_press_is_new(pressed: &HashSet<(usize, u16)>, code: u16) -> bool {
    !pressed
        .iter()
        .any(|&(_, pressed_code)| pressed_code == code)
}

fn escape_closes_menu(event: KeyEvent) -> bool {
    event.code == KEY_ESC && matches!(event.value, 1 | 2)
}

fn is_modifier_key(code: u16) -> bool {
    matches!(
        code,
        KEY_LEFTCTRL
            | KEY_RIGHTCTRL
            | KEY_LEFTSHIFT
            | KEY_RIGHTSHIFT
            | KEY_LEFTALT
            | KEY_RIGHTALT
            | KEY_LEFTMETA
            | KEY_RIGHTMETA
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_parser_is_bounded_and_supports_function_key_chords() {
        assert_eq!(parse_chord("F8").expect("F8").key, 66);
        assert_eq!(parse_chord("Shift+Tab").expect("Shift+Tab").key, 15);
        let chord = parse_chord("Ctrl+Shift+F10").expect("modified key");
        assert_eq!(chord.modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(chord.key, 68);
        assert!(parse_chord("VolumeUp").is_err());
        assert!(parse_chord("F8+F9").is_err());
    }

    #[test]
    fn configuration_accepts_only_known_durations() {
        let mut valid = BufReader::new(&b"TOGGLE Shift+Tab\nBIND F8 30\nSTART\n"[..]);
        assert_eq!(
            read_bindings(&mut valid).expect("valid")[1].action,
            HotkeyAction::Save(30)
        );
        let mut invalid = BufReader::new(&b"TOGGLE Shift+Tab\nBIND F8 31\nSTART\n"[..]);
        assert!(read_bindings(&mut invalid).is_err());
        let mut missing_toggle = BufReader::new(&b"BIND F8 30\nSTART\n"[..]);
        assert_eq!(
            read_bindings(&mut missing_toggle).unwrap()[0].action,
            HotkeyAction::Save(30)
        );
        let mut menu_only = BufReader::new(&b"TOGGLE Shift+Tab\nSTART\n"[..]);
        assert_eq!(read_bindings(&mut menu_only).unwrap().len(), 1);
    }

    #[test]
    fn process_identity_reads_effective_uid_and_nonzero_parent() {
        let status = "Name:\tredunar-hotkey\nPPid:\t4321\nUid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_process_identity(status), Ok((1000, 4_321)));
        assert_eq!(
            parse_process_identity("Uid:\t0\t0\t0\t0\nPPid:\t1\n"),
            Ok((0, 1))
        );
        assert!(parse_process_identity("PPid:\t1\n").is_err());
    }

    #[test]
    fn checkout_release_parent_is_trusted_only_with_adjacent_helper() {
        let directory =
            std::env::temp_dir().join(format!("redunar-hotkey-trust-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("trust fixture directory");
        let parent = directory.join("redunar-tauri");
        let helper = directory.join("redunar-hotkey-helper");
        std::fs::write(&helper, b"fixture").expect("trust fixture helper");
        assert!(trusted_parent(&parent, Some(&helper)));
        let other = directory.join("different-helper");
        std::fs::write(&other, b"fixture").unwrap();
        assert!(!trusted_parent(&parent, Some(&other)));
        assert!(!trusted_parent(&directory.join("other-app"), Some(&helper)));
        std::fs::remove_file(other).unwrap();
        std::fs::remove_file(&helper).expect("remove trust fixture helper");
        assert!(!trusted_parent(&parent, Some(&helper)));
        std::fs::remove_dir(directory).expect("remove trust fixture directory");
    }

    #[test]
    fn unpacked_package_parent_requires_its_matching_libexec_helper() {
        let root = std::env::temp_dir().join(format!("rd-unpacked-{}", std::process::id()));
        let bin = root.join("usr/bin");
        let libexec = root.join("usr/libexec");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&libexec).unwrap();
        let helper = libexec.join("redunar-hotkey-helper");
        std::fs::write(&helper, b"fixture").unwrap();
        assert!(trusted_parent(&bin.join("redunar-tauri"), Some(&helper)));
        assert!(!trusted_parent(&bin.join("redunar-tauri"), None));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mouse_device_links_are_bounded_to_kernel_mouse_event_aliases() {
        assert!(device_link_name_matches(
            "usb-Example_Mouse-event-mouse",
            "-event-mouse"
        ));
        assert!(device_link_name_matches(
            "pci-0000:00:14.0-usb-0:2:1.0-event-mouse",
            "-event-mouse"
        ));
        assert!(!device_link_name_matches("mouse0", "-event-mouse"));
        assert!(!device_link_name_matches(
            "nested/device-event-mouse",
            "-event-mouse"
        ));
    }

    #[test]
    fn pointer_parser_accepts_only_relative_motion_and_left_button_edges() {
        assert_eq!(
            parse_pointer_event(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, 17),
            Some(PointerEvent::MotionX(17))
        );
        assert_eq!(
            parse_pointer_event(EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, -9),
            Some(PointerEvent::MotionY(-9))
        );
        assert_eq!(
            parse_pointer_event(EV_KEY, BTN_LEFT, 1),
            Some(PointerEvent::LeftButton(true))
        );
        assert_eq!(parse_pointer_event(EV_KEY, BTN_LEFT, 2), None);
        assert_eq!(parse_pointer_event(EV_KEY, 273, 1), None);
    }

    #[test]
    fn motion_is_coalesced_and_bounded_before_crossing_the_socket() {
        let mut pending = PendingMotion::default();
        pending.add(PointerEvent::MotionX(3_000));
        pending.add(PointerEvent::MotionX(3_000));
        pending.add(PointerEvent::MotionY(-5_000));
        assert_eq!(pending.take(), Some((4_096, -4_096)));
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn raw_mouse_motion_gets_continuous_aspect_corrected_acceleration() {
        let slow = accelerated_pointer_motion(1, 1);
        let medium = accelerated_pointer_motion(6, 6);
        let fast = accelerated_pointer_motion(12, 12);
        assert!(slow.0 > 0);
        assert!(slow.1 > slow.0);
        assert!(medium.0 > slow.0 * 6);
        assert!(fast.0 > medium.0 * 2);
        assert_eq!(accelerated_pointer_motion(i32::MAX, 0).0, MOTION_LIMIT);
    }

    #[test]
    fn menu_state_clears_pending_input_on_every_release() {
        let now = Instant::now();
        let mut menu = MenuCapture::closed(now);
        let (control, _peer) = UnixStream::pair().expect("local control pair");
        menu.opened(control, now, true);
        menu.pending_motion.add(PointerEvent::MotionX(4));
        menu.closed_by_release();
        assert!(!menu.open);
        assert!(!menu.pointer_grabbed);
        assert_eq!(menu.pending_motion.take(), None);
    }

    #[test]
    fn view_only_session_pings_without_reading_unowned_pointer_input() {
        let now = Instant::now();
        let (control, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(CONTROL_TIMEOUT)).unwrap();
        let server = thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(&mut peer).read_line(&mut request).unwrap();
            assert_eq!(request, "MENU PING\n");
            peer.write_all(b"OK GRAB\n").unwrap();
        });
        let mut menu = MenuCapture::closed(now);
        menu.opened(control, now.checked_sub(MENU_PING_INTERVAL).unwrap(), false);
        assert!(process_pointer_input(&mut menu, || {
            panic!("view-only fallback must not read or forward gameplay clicks")
        }));
        assert!(menu.open);
        assert!(!menu.pointer_grabbed);
        server.join().unwrap();
    }

    #[test]
    fn owned_pointer_input_is_forwarded_and_failures_request_cleanup() {
        let now = Instant::now();
        let (control, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(CONTROL_TIMEOUT)).unwrap();
        let server = thread::spawn(move || {
            let mut reader = BufReader::new(&mut peer);
            for expected in ["MENU BUTTON 1\n", "MENU BUTTON 0\n"] {
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, expected);
                reader.get_mut().write_all(b"OK GRAB\n").unwrap();
            }
        });
        let mut menu = MenuCapture::closed(now);
        menu.opened(control, now, true);
        assert!(process_pointer_input(&mut menu, || Ok(vec![
            PointerEvent::LeftButton(true),
            PointerEvent::LeftButton(false),
        ])));
        assert!(menu.open);
        server.join().unwrap();
        assert!(!process_pointer_input(&mut menu, || Err(
            io::ErrorKind::NotConnected.into()
        )));
    }

    #[test]
    fn duplicate_composite_keyboard_edges_trigger_once() {
        let mut pressed = HashSet::new();
        assert!(logical_press_is_new(&pressed, 66));
        pressed.insert((0, 66));
        assert!(!logical_press_is_new(&pressed, 66));
        pressed.insert((1, 66));
        pressed.remove(&(0, 66));
        assert!(!logical_press_is_new(&pressed, 66));
        pressed.remove(&(1, 66));
        assert!(logical_press_is_new(&pressed, 66));
    }

    #[test]
    fn composite_keyboard_batch_applies_modifier_press_before_function_key() {
        let (sender, receiver) = mpsc::sync_channel(4);
        sender
            .send(KeyboardEvent::Key(KeyEvent {
                device: 0,
                code: 66,
                value: 1,
            }))
            .unwrap();
        sender
            .send(KeyboardEvent::Key(KeyEvent {
                device: 1,
                code: KEY_LEFTSHIFT,
                value: 1,
            }))
            .unwrap();
        drop(sender);
        let first = receiver.recv().unwrap();
        let events = chord_event_batch(&receiver, first);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            KeyboardEvent::Key(KeyEvent {
                code: KEY_LEFTSHIFT,
                ..
            })
        ));
        assert!(matches!(
            events[1],
            KeyboardEvent::Key(KeyEvent { code: 66, .. })
        ));
    }

    #[test]
    fn alt_shift_z_opens_once_survives_key_release_and_toggles_closed() {
        let directory = std::env::temp_dir().join(format!("rd-menu-chord-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let (sender, receiver) = mpsc::sync_channel(16);
        let chord = parse_chord("Alt+Shift+Z").unwrap();
        let send_chord = move |sender: &mpsc::SyncSender<KeyboardEvent>| {
            // Include duplicate Z edges from a composite keyboard, and release
            // both modifiers. None of these may immediately dismiss the menu.
            for value in [1, 0] {
                for (device, code) in [
                    (0, KEY_LEFTALT),
                    (1, KEY_LEFTSHIFT),
                    (0, chord.key),
                    (1, chord.key),
                ] {
                    sender
                        .send(KeyboardEvent::Key(KeyEvent {
                            device,
                            code,
                            value,
                        }))
                        .unwrap();
                }
            }
        };
        send_chord(&sender);
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "menu shortcut did not open");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("menu fixture accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            for (expected, response) in [
                ("MENU TOGGLE\n", &b"OK GRAB\n"[..]),
                ("MENU PING\n", &b"OK GRAB\n"[..]),
                ("MENU TOGGLE\n", &b"OK RELEASE\n"[..]),
            ] {
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, expected);
                reader.get_mut().write_all(response).unwrap();
                if expected == "MENU PING\n" {
                    send_chord(&sender);
                }
            }
            server_stop.store(true, Ordering::Release);
        });
        let mut refreshes = 0;
        let result = monitor_events(&receiver, &stop, &[chord], &path, &mut Vec::new(), || {
            refreshes += 1;
            Vec::new()
        });
        let server_result = server.join();
        fs::remove_dir_all(directory).unwrap();
        result.unwrap();
        server_result.unwrap();
        assert_eq!(
            refreshes, 1,
            "refresh on opening, never during an owned menu"
        );
    }

    #[test]
    fn keyboard_supervisor_joins_when_shutdown_drops_a_full_queue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        // Metadata is a private regular file; opening is replaced entirely.
        let directory = std::env::temp_dir().join(format!("rd-key-stop-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("identity");
        fs::write(&path, []).unwrap();
        let identity = keyboards::identity(&path).unwrap();
        let mut bytes = Vec::new();
        for _ in 0..32 {
            let mut event = [0; INPUT_EVENT_BYTES];
            event[16..18].copy_from_slice(&EV_KEY.to_ne_bytes());
            event[18..20].copy_from_slice(&44_u16.to_ne_bytes());
            event[20..24].copy_from_slice(&1_i32.to_ne_bytes());
            bytes.extend_from_slice(&event);
        }
        let (done_sender, done_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            supervise_keyboard_input(
                &worker_stop,
                &sender,
                || vec![(path.clone(), identity)],
                |_| Ok((identity, io::Cursor::new(bytes.clone()))),
            );
            done_sender.send(()).unwrap();
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            KeyboardEvent::Availability(1)
        );
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            KeyboardEvent::Key(_)
        ));
        stop.store(true, Ordering::Release);
        drop(receiver);
        done_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("supervisor stranded on queue or input");
        worker.join().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn lost_keyboard_clears_modifiers_and_duplicate_state_before_replacement_save() {
        let directory = std::env::temp_dir().join(format!("rd-key-save-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let (sender, receiver) = mpsc::sync_channel(16);
        for event in [
            KeyboardEvent::Key(KeyEvent {
                device: 0,
                code: KEY_LEFTSHIFT,
                value: 1,
            }),
            KeyboardEvent::Key(KeyEvent {
                device: 0,
                code: 44,
                value: 1,
            }),
            KeyboardEvent::Removed(0),
            KeyboardEvent::Availability(0),
            KeyboardEvent::Availability(2),
            KeyboardEvent::Key(KeyEvent {
                device: 1,
                code: KEY_LEFTALT,
                value: 1,
            }),
            KeyboardEvent::Key(KeyEvent {
                device: 1,
                code: 44,
                value: 1,
            }),
            KeyboardEvent::Key(KeyEvent {
                device: 2,
                code: 44,
                value: 1,
            }),
        ] {
            sender.send(event).unwrap();
        }
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut saved = false;
            let mut quiet_until = deadline;
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        assert!(!saved, "composite keyboard submitted duplicate save");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = String::new();
                        BufReader::new(&mut stream).read_line(&mut request).unwrap();
                        assert_eq!(request, "SAVE 30\n");
                        stream.write_all(b"OK\n").unwrap();
                        saved = true;
                        quiet_until = Instant::now() + Duration::from_millis(80);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if saved && Instant::now() >= quiet_until {
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "replacement keyboard did not save"
                        );
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("save fixture failed: {error}"),
                }
            }
            server_stop.store(true, Ordering::Release);
        });
        let mut chord = parse_chord("Alt+Z").unwrap();
        chord.action = HotkeyAction::Save(30);
        let result = monitor_events(&receiver, &stop, &[chord], &path, &mut Vec::new(), Vec::new);
        let server_result = server.join();
        fs::remove_dir_all(directory).unwrap();
        result.unwrap();
        server_result.unwrap();
    }

    #[test]
    fn keyboard_removal_closes_menu_through_owner_even_without_pointer_access() {
        let directory = std::env::temp_dir().join(format!("rd-key-menu-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let (sender, receiver) = mpsc::sync_channel(4);
        sender
            .send(KeyboardEvent::Key(KeyEvent {
                device: 0,
                code: 66,
                value: 1,
            }))
            .unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "menu did not open");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("menu fixture failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "MENU TOGGLE\n");
            reader.get_mut().write_all(b"OK GRAB\n").unwrap();
            sender.send(KeyboardEvent::Removed(0)).unwrap();
            request.clear();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "MENU ESCAPE\n");
            reader.get_mut().write_all(b"OK RELEASE\n").unwrap();
            server_stop.store(true, Ordering::Release);
        });
        let result = monitor_events(
            &receiver,
            &stop,
            &[parse_chord("F8").unwrap()],
            &path,
            &mut Vec::new(),
            Vec::new,
        );
        let server_result = server.join();
        fs::remove_dir_all(directory).unwrap();
        result.unwrap();
        server_result.unwrap();
    }

    #[test]
    fn watchdog_closes_only_an_open_inactive_menu() {
        let now = Instant::now();
        let mut menu = MenuCapture::closed(now);
        assert_eq!(
            watchdog_action(&menu, now + MENU_INACTIVITY_TIMEOUT),
            WatchdogAction::None
        );
        let (control, _peer) = UnixStream::pair().expect("local control pair");
        menu.opened(control, now, false);
        assert_eq!(
            watchdog_action(
                &menu,
                (now + MENU_INACTIVITY_TIMEOUT)
                    .checked_sub(Duration::from_millis(1))
                    .expect("one millisecond is below the watchdog duration"),
            ),
            WatchdogAction::None
        );
        assert_eq!(
            watchdog_action(&menu, now + MENU_INACTIVITY_TIMEOUT),
            WatchdogAction::Inactive
        );
    }

    #[test]
    fn only_modifier_keys_may_leave_the_mouse_menu_open_for_a_chord() {
        assert!(is_modifier_key(KEY_LEFTALT));
        assert!(is_modifier_key(KEY_RIGHTSHIFT));
        assert!(!is_modifier_key(15));
        assert!(!is_modifier_key(KEY_ESC));
    }

    #[test]
    fn escape_press_and_repeat_are_reliable_menu_close_edges() {
        let event = |value| KeyEvent {
            device: 0,
            code: KEY_ESC,
            value,
        };
        assert!(escape_closes_menu(event(1)));
        assert!(escape_closes_menu(event(2)));
        assert!(!escape_closes_menu(event(0)));
        assert!(!escape_closes_menu(KeyEvent {
            device: 0,
            code: 2,
            value: 1,
        }));
    }

    #[test]
    fn menu_opens_without_a_readable_mouse_device() {
        let directory = std::env::temp_dir().join(format!(
            "redunar-hotkey-view-only-menu-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("menu fixture directory");
        let path = directory.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("control fixture");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("menu request");
            let mut request = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut request)
                .expect("read menu request");
            assert_eq!(request, "MENU TOGGLE\n");
            stream.write_all(b"OK GRAB\n").expect("menu response");
        });

        let mut menu = MenuCapture::closed(Instant::now());
        toggle_menu(&path, &mut [], &mut menu);
        server.join().expect("control fixture worker");
        assert!(
            menu.open,
            "the in-game menu remains available without pointer access"
        );
        assert!(menu.control.is_some());

        drop(menu);
        std::fs::remove_file(&path).expect("remove control fixture");
        std::fs::remove_dir(&directory).expect("remove menu fixture directory");
    }
}
