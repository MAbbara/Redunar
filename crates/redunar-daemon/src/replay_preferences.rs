use redunar_core::ReplayDuration;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

const PREFERENCES_FILE: &str = "replay-preferences-v1.txt";
const PREFERENCES_HEADER_V1: &str = "redunar-replay-preferences-v1";
const PREFERENCES_HEADER_V2: &str = "redunar-replay-preferences-v2";
const PREFERENCES_HEADER_V3: &str = "redunar-replay-preferences-v3";
const PREFERENCES_HEADER_V4: &str = "redunar-replay-preferences-v4";
const PREFERENCES_HEADER_V5: &str = "redunar-replay-preferences-v5";
const PREFERENCES_HEADER_V6: &str = "redunar-replay-preferences-v6";
const DEFAULT_SHORTCUT: &str = "F8";
const DEFAULT_OVERLAY_SHORTCUT: &str = "Shift+Tab";
const REPLAY_DIRECTORY_NAME: &str = "Redunar Replays";
const MAX_PREFERENCES_BYTES: u64 = 8 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_SHORTCUT_BYTES: usize = 64;
pub const MAX_REPLAY_SHORTCUTS: usize = 8;
static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);
static PREFERENCES_OPERATIONS: Mutex<()> = Mutex::new(());

pub(crate) fn saved_preferences_exist(state_directory: &Path) -> bool {
    fs::symlink_metadata(state_directory.join(PREFERENCES_FILE))
        .is_ok_and(|metadata| metadata.file_type().is_file())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayShortcutBinding {
    pub shortcut: String,
    pub duration: ReplayDuration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReplayOutputFormat {
    #[default]
    Matroska,
    Mp4,
}

impl ReplayOutputFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Matroska => "mkv",
            Self::Mp4 => "mp4",
        }
    }
}

/// Replay behavior that is not part of a game profile or encoder budget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayPreferences {
    /// Optional user-selected parent. Redunar owns only its fixed child
    /// directory, never the selected parent itself.
    pub custom_save_parent: Option<PathBuf>,
    pub output_format: ReplayOutputFormat,
    pub overlay_shortcut: String,
    /// Close the game-contained mouse menu when its outside area is clicked.
    pub close_overlay_on_outside_click: bool,
    /// Duration selected when the user opens the Replay save control.
    pub initial_save_duration: ReplayDuration,
    pub save_shortcuts: Vec<ReplayShortcutBinding>,
}

impl Default for ReplayPreferences {
    fn default() -> Self {
        Self {
            custom_save_parent: None,
            output_format: ReplayOutputFormat::Matroska,
            overlay_shortcut: DEFAULT_OVERLAY_SHORTCUT.to_owned(),
            close_overlay_on_outside_click: true,
            initial_save_duration: ReplayDuration::Seconds30,
            save_shortcuts: vec![ReplayShortcutBinding {
                shortcut: DEFAULT_SHORTCUT.to_owned(),
                duration: ReplayDuration::Seconds30,
            }],
        }
    }
}

pub(crate) fn set_initial_save_duration(
    state_directory: &Path,
    duration: ReplayDuration,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    preferences.initial_save_duration = duration;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

impl ReplayPreferences {
    #[must_use]
    pub fn resolved_directory(&self, home: Option<&Path>, state_directory: &Path) -> PathBuf {
        if let Some(parent) = &self.custom_save_parent {
            return parent.join(REPLAY_DIRECTORY_NAME);
        }
        home.filter(|path| path.is_absolute()).map_or_else(
            || state_directory.join("replays-v1"),
            |path| path.join("Videos").join(REPLAY_DIRECTORY_NAME),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayPreferencesError {
    message: String,
}

impl ReplayPreferencesError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ReplayPreferencesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ReplayPreferencesError {}

pub(crate) fn load(state_directory: &Path) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    load_unlocked(state_directory)
}

pub(crate) fn set_save_parent(
    state_directory: &Path,
    parent: Option<PathBuf>,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    if let Some(parent) = parent.as_deref() {
        validate_parent(parent)?;
    }
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    preferences.custom_save_parent = parent;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

pub(crate) fn set_save_shortcuts(
    state_directory: &Path,
    bindings: Vec<ReplayShortcutBinding>,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    validate_hotkeys(&preferences.overlay_shortcut, &bindings)?;
    preferences.save_shortcuts = bindings;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

pub(crate) fn set_hotkeys(
    state_directory: &Path,
    overlay_shortcut: String,
    bindings: Vec<ReplayShortcutBinding>,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    validate_hotkeys(&overlay_shortcut, &bindings)?;
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    preferences.overlay_shortcut = overlay_shortcut;
    preferences.save_shortcuts = bindings;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

pub(crate) fn set_overlay_behavior(
    state_directory: &Path,
    overlay_shortcut: String,
    close_on_outside_click: bool,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    validate_hotkeys(&overlay_shortcut, &preferences.save_shortcuts)?;
    preferences.overlay_shortcut = overlay_shortcut;
    preferences.close_overlay_on_outside_click = close_on_outside_click;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

// Dismissal owns only its own field, never a possibly stale shortcut value.
pub(crate) fn set_outside_click(
    state_directory: &Path,
    enabled: bool,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    preferences.close_overlay_on_outside_click = enabled;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

pub(crate) fn set_output_format(
    state_directory: &Path,
    output_format: ReplayOutputFormat,
) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let _operation = lock_operations();
    let mut preferences = load_unlocked(state_directory)?;
    preferences.output_format = output_format;
    save_unlocked(state_directory, &preferences)?;
    Ok(preferences)
}

fn validate_parent(parent: &Path) -> Result<(), ReplayPreferencesError> {
    if !parent.is_absolute() || parent.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
        return Err(ReplayPreferencesError::new(
            "replay save parent must be an absolute path within the supported length",
        ));
    }
    Ok(())
}

fn validate_shortcut(shortcut: &str) -> Result<(), ReplayPreferencesError> {
    if shortcut.is_empty()
        || shortcut.len() > MAX_SHORTCUT_BYTES
        || !shortcut.is_ascii()
        || shortcut.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ReplayPreferencesError::new(
            "replay shortcut must be 1–64 printable ASCII characters",
        ));
    }
    Ok(())
}

fn validate_shortcuts(bindings: &[ReplayShortcutBinding]) -> Result<(), ReplayPreferencesError> {
    if bindings.len() > MAX_REPLAY_SHORTCUTS {
        return Err(ReplayPreferencesError::new(
            "Replay supports up to eight save shortcuts",
        ));
    }
    let mut unique = HashSet::with_capacity(bindings.len());
    for binding in bindings {
        validate_shortcut(&binding.shortcut)?;
        if !unique.insert(binding.shortcut.as_str()) {
            return Err(ReplayPreferencesError::new(
                "Replay save shortcuts must be unique",
            ));
        }
    }
    Ok(())
}

fn validate_hotkeys(
    overlay_shortcut: &str,
    bindings: &[ReplayShortcutBinding],
) -> Result<(), ReplayPreferencesError> {
    if !overlay_shortcut.is_empty() {
        validate_shortcut(overlay_shortcut)?;
    }
    validate_shortcuts(bindings)?;
    if bindings
        .iter()
        .any(|binding| binding.shortcut == overlay_shortcut)
    {
        return Err(ReplayPreferencesError::new(
            "Replay overlay and save shortcuts must be unique",
        ));
    }
    Ok(())
}

fn load_unlocked(state_directory: &Path) -> Result<ReplayPreferences, ReplayPreferencesError> {
    let path = state_directory.join(PREFERENCES_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReplayPreferences::default());
        }
        Err(error) => return Err(read_error(&path, &error)),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ReplayPreferencesError::new(format!(
            "could not load Replay preferences from {}: state is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > MAX_PREFERENCES_BYTES {
        return Err(ReplayPreferencesError::new(format!(
            "could not load Replay preferences from {}: state is too large",
            path.display()
        )));
    }
    let mut contents = String::new();
    File::open(&path)
        .and_then(|file| {
            file.take(MAX_PREFERENCES_BYTES + 1)
                .read_to_string(&mut contents)
        })
        .map_err(|error| read_error(&path, &error))?;
    parse(&contents).map_err(|message| {
        ReplayPreferencesError::new(format!(
            "could not load Replay preferences from {}: {message}",
            path.display()
        ))
    })
}

fn save_unlocked(
    state_directory: &Path,
    preferences: &ReplayPreferences,
) -> Result<(), ReplayPreferencesError> {
    fs::create_dir_all(state_directory).map_err(|error| {
        ReplayPreferencesError::new(format!(
            "could not create Redunar state directory {}: {error}",
            state_directory.display()
        ))
    })?;
    fs::set_permissions(state_directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
        ReplayPreferencesError::new(format!(
            "could not make Redunar state directory private at {}: {error}",
            state_directory.display()
        ))
    })?;
    let destination = state_directory.join(PREFERENCES_FILE);
    atomic_replace(&destination, serialize(preferences).as_bytes()).map_err(|error| {
        ReplayPreferencesError::new(format!(
            "could not save Replay preferences to {}: {error}",
            destination.display()
        ))
    })
}

fn serialize(preferences: &ReplayPreferences) -> String {
    let parent = preferences.custom_save_parent.as_deref().map_or_else(
        || "default".to_owned(),
        |path| hex(path.as_os_str().as_bytes()),
    );
    let mut output = format!(
        "{PREFERENCES_HEADER_V6}\nsave-parent={parent}\noutput-format={}\noverlay-shortcut={}\nclose-on-outside-click={}\ninitial-save-duration={}\nbinding-count={}\n",
        output_format_token(preferences.output_format),
        preferences.overlay_shortcut,
        preferences.close_overlay_on_outside_click,
        duration_token(preferences.initial_save_duration),
        preferences.save_shortcuts.len()
    );
    for binding in &preferences.save_shortcuts {
        output.push_str("binding=");
        output.push_str(duration_token(binding.duration));
        output.push(':');
        output.push_str(&binding.shortcut);
        output.push('\n');
    }
    output
}

fn parse(contents: &str) -> Result<ReplayPreferences, &'static str> {
    let mut lines = contents.lines();
    let version = lines.next().ok_or("preferences are empty")?;
    if !matches!(
        version,
        PREFERENCES_HEADER_V1
            | PREFERENCES_HEADER_V2
            | PREFERENCES_HEADER_V3
            | PREFERENCES_HEADER_V4
            | PREFERENCES_HEADER_V5
            | PREFERENCES_HEADER_V6
    ) {
        return Err("unsupported preferences version");
    }
    let parent = field(lines.next(), "save-parent")?;
    let custom_save_parent = if parent == "default" {
        None
    } else {
        let bytes = unhex(parent)?;
        if bytes.len() > MAX_PATH_BYTES {
            return Err("save parent exceeds the supported length");
        }
        let path = PathBuf::from(OsString::from_vec(bytes));
        validate_parent(&path).map_err(|_| "save parent is invalid")?;
        Some(path)
    };
    let output_format = if matches!(
        version,
        PREFERENCES_HEADER_V3
            | PREFERENCES_HEADER_V4
            | PREFERENCES_HEADER_V5
            | PREFERENCES_HEADER_V6
    ) {
        parse_output_format(field(lines.next(), "output-format")?)?
    } else {
        ReplayOutputFormat::Matroska
    };
    let overlay_shortcut = if matches!(
        version,
        PREFERENCES_HEADER_V4 | PREFERENCES_HEADER_V5 | PREFERENCES_HEADER_V6
    ) {
        field(lines.next(), "overlay-shortcut")?.to_owned()
    } else {
        DEFAULT_OVERLAY_SHORTCUT.to_owned()
    };
    let close_overlay_on_outside_click =
        if matches!(version, PREFERENCES_HEADER_V5 | PREFERENCES_HEADER_V6) {
            parse_bool(field(lines.next(), "close-on-outside-click")?)?
        } else {
            true
        };
    let initial_save_duration = if version == PREFERENCES_HEADER_V6 {
        parse_duration_token(field(lines.next(), "initial-save-duration")?)?
    } else {
        ReplayDuration::Seconds30
    };
    let save_shortcuts = if version == PREFERENCES_HEADER_V1 {
        let shortcut = field(lines.next(), "save-shortcut")?;
        if lines.next().is_some() {
            return Err("preferences contain unknown fields");
        }
        vec![ReplayShortcutBinding {
            shortcut: shortcut.to_owned(),
            duration: ReplayDuration::Seconds30,
        }]
    } else {
        let count = field(lines.next(), "binding-count")?
            .parse::<usize>()
            .map_err(|_| "shortcut binding count is invalid")?;
        if count > MAX_REPLAY_SHORTCUTS {
            return Err("shortcut binding count is invalid");
        }
        let mut bindings = Vec::with_capacity(count);
        for _ in 0..count {
            let value = field(lines.next(), "binding")?;
            let (duration, shortcut) =
                value.split_once(':').ok_or("shortcut binding is invalid")?;
            bindings.push(ReplayShortcutBinding {
                shortcut: shortcut.to_owned(),
                duration: parse_duration_token(duration)?,
            });
        }
        if lines.next().is_some() {
            return Err("preferences contain unknown fields");
        }
        bindings
    };
    validate_hotkeys(&overlay_shortcut, &save_shortcuts)
        .map_err(|_| "Replay shortcuts are invalid")?;
    Ok(ReplayPreferences {
        custom_save_parent,
        output_format,
        overlay_shortcut,
        close_overlay_on_outside_click,
        initial_save_duration,
        save_shortcuts,
    })
}

fn parse_bool(value: &str) -> Result<bool, &'static str> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Replay overlay behavior is invalid"),
    }
}

const fn output_format_token(format: ReplayOutputFormat) -> &'static str {
    match format {
        ReplayOutputFormat::Matroska => "mkv",
        ReplayOutputFormat::Mp4 => "mp4",
    }
}

fn parse_output_format(value: &str) -> Result<ReplayOutputFormat, &'static str> {
    match value {
        "mkv" => Ok(ReplayOutputFormat::Matroska),
        "mp4" => Ok(ReplayOutputFormat::Mp4),
        _ => Err("Replay output format is invalid"),
    }
}

const fn duration_token(duration: ReplayDuration) -> &'static str {
    match duration {
        ReplayDuration::Seconds15 => "15",
        ReplayDuration::Seconds30 => "30",
        ReplayDuration::Seconds60 => "60",
        ReplayDuration::Seconds120 => "120",
        ReplayDuration::Seconds180 => "180",
        ReplayDuration::Seconds300 => "300",
        ReplayDuration::Seconds600 => "600",
        ReplayDuration::Seconds900 => "900",
    }
}

fn parse_duration_token(value: &str) -> Result<ReplayDuration, &'static str> {
    match value {
        "15" => Ok(ReplayDuration::Seconds15),
        "30" => Ok(ReplayDuration::Seconds30),
        "60" => Ok(ReplayDuration::Seconds60),
        "120" => Ok(ReplayDuration::Seconds120),
        "180" => Ok(ReplayDuration::Seconds180),
        "300" => Ok(ReplayDuration::Seconds300),
        "600" => Ok(ReplayDuration::Seconds600),
        "900" => Ok(ReplayDuration::Seconds900),
        _ => Err("shortcut duration is invalid"),
    }
}

fn field<'a>(line: Option<&'a str>, expected: &str) -> Result<&'a str, &'static str> {
    let (name, value) = line
        .and_then(|line| line.split_once('='))
        .ok_or("preferences contain a missing field")?;
    if name != expected {
        return Err("preferences contain missing or reordered fields");
    }
    Ok(value)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn unhex(value: &str) -> Result<Vec<u8>, &'static str> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err("save parent encoding is invalid");
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = digit(pair[0])?;
            let low = digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

const fn digit(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("save parent encoding is invalid"),
    }
}

fn atomic_replace(destination: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent"))?;
    let mut last_collision = None;
    for _ in 0..32 {
        let id = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".{PREFERENCES_FILE}.{}.{id}.tmp",
            std::process::id()
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
                continue;
            }
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(contents)?;
            file.sync_all()?;
            fs::rename(&temporary, destination)?;
            File::open(directory)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate state file",
        )
    }))
}

fn lock_operations() -> MutexGuard<'static, ()> {
    PREFERENCES_OPERATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_error(path: &Path, error: &io::Error) -> ReplayPreferencesError {
    ReplayPreferencesError::new(format!(
        "could not load Replay preferences from {}: {error}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> PathBuf {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "redunar-replay-preferences-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn cleared_and_partial_hotkeys_round_trip_without_default_reactivation() {
        let state = fixture();
        for (overlay, bindings) in [
            ("", vec![]),
            (
                "",
                vec![ReplayShortcutBinding {
                    shortcut: "F8".into(),
                    duration: ReplayDuration::Seconds30,
                }],
            ),
            ("Ctrl+Tab", vec![]),
        ] {
            set_hotkeys(&state, overlay.into(), bindings.clone()).unwrap();
            let saved = load(&state).unwrap();
            assert_eq!(saved.overlay_shortcut, overlay);
            assert_eq!(saved.save_shortcuts, bindings);
            set_outside_click(&state, false).unwrap();
            assert_eq!(load(&state).unwrap().overlay_shortcut, overlay);
        }
        set_hotkeys(&state, "Ctrl+Shift+R".into(), vec![]).unwrap();
        set_hotkeys(&state, "Ctrl+Shift+T".into(), vec![]).unwrap();
        set_outside_click(&state, true).unwrap();
        assert_eq!(load(&state).unwrap().overlay_shortcut, "Ctrl+Shift+T");
        assert!(set_hotkeys(&state, "bad\nshortcut".into(), vec![]).is_err());
        fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn default_resolves_to_videos_with_state_fallback() {
        let preferences = ReplayPreferences::default();
        assert_eq!(preferences.overlay_shortcut, "Shift+Tab");
        assert!(preferences.close_overlay_on_outside_click);
        assert_eq!(
            preferences.resolved_directory(Some(Path::new("/home/player")), Path::new("/state")),
            PathBuf::from("/home/player/Videos/Redunar Replays")
        );
        assert_eq!(
            preferences.resolved_directory(None, Path::new("/state")),
            PathBuf::from("/state/replays-v1")
        );
    }

    #[test]
    fn updates_round_trip_without_creating_the_selected_parent() {
        let state = fixture();
        let parent = state.with_file_name("redunar-replay-selected-parent-does-not-exist");
        let preferences = set_save_parent(&state, Some(parent.clone())).expect("save parent");
        assert!(!parent.exists());
        assert_eq!(
            preferences.resolved_directory(Some(Path::new("/home/player")), &state),
            parent.join(REPLAY_DIRECTORY_NAME)
        );
        set_save_shortcuts(
            &state,
            vec![
                ReplayShortcutBinding {
                    shortcut: "Super+F10".to_owned(),
                    duration: ReplayDuration::Seconds60,
                },
                ReplayShortcutBinding {
                    shortcut: "Super+F11".to_owned(),
                    duration: ReplayDuration::Seconds300,
                },
            ],
        )
        .expect("save shortcuts");
        set_output_format(&state, ReplayOutputFormat::Mp4).expect("save format");
        set_initial_save_duration(&state, ReplayDuration::Seconds120).expect("save duration");
        let loaded = load(&state).expect("reload preferences");
        assert_eq!(loaded.custom_save_parent, Some(parent));
        assert_eq!(loaded.output_format, ReplayOutputFormat::Mp4);
        assert_eq!(loaded.overlay_shortcut, "Shift+Tab");
        assert_eq!(loaded.initial_save_duration, ReplayDuration::Seconds120);
        assert_eq!(loaded.save_shortcuts.len(), 2);
        assert_eq!(
            loaded.save_shortcuts[1].duration,
            ReplayDuration::Seconds300
        );
        let _ = fs::remove_dir_all(state);
    }

    #[test]
    fn unsafe_parent_shortcut_and_state_are_rejected() {
        let state = fixture();
        assert!(set_save_parent(&state, Some(PathBuf::from("relative"))).is_err());
        assert!(
            set_save_shortcuts(
                &state,
                vec![ReplayShortcutBinding {
                    shortcut: "bad\nshortcut".to_owned(),
                    duration: ReplayDuration::Seconds30,
                }]
            )
            .is_err()
        );
        fs::create_dir_all(&state).expect("create state");
        fs::write(
            state.join(PREFERENCES_FILE),
            "redunar-replay-preferences-v1\nsave-parent=zz\nsave-shortcut=F10\n",
        )
        .expect("write malformed preferences");
        assert!(load(&state).is_err());
        let _ = fs::remove_dir_all(state);
    }

    #[test]
    fn version_one_single_shortcut_migrates_to_a_thirty_second_binding() {
        let parsed = parse(
            "redunar-replay-preferences-v1\nsave-parent=default\nsave-shortcut=Ctrl+Shift+S\n",
        )
        .expect("parse v1");
        assert_eq!(
            parsed.save_shortcuts,
            vec![ReplayShortcutBinding {
                shortcut: "Ctrl+Shift+S".to_owned(),
                duration: ReplayDuration::Seconds30,
            }]
        );
        assert_eq!(parsed.output_format, ReplayOutputFormat::Matroska);
    }

    #[test]
    fn older_versions_migrate_and_version_five_round_trips_overlay_behavior() {
        let version_two = parse(
            "redunar-replay-preferences-v2\nsave-parent=default\nbinding-count=1\nbinding=30:F8\n",
        )
        .expect("parse v2");
        assert_eq!(version_two.output_format, ReplayOutputFormat::Matroska);
        assert_eq!(version_two.overlay_shortcut, "Shift+Tab");
        let mut version_four = version_two;
        version_four.output_format = ReplayOutputFormat::Mp4;
        version_four.overlay_shortcut = "Alt+F9".to_owned();
        version_four.close_overlay_on_outside_click = false;
        let parsed = parse(&serialize(&version_four)).expect("parse serialized v4");
        assert_eq!(parsed.output_format, ReplayOutputFormat::Mp4);
        assert_eq!(parsed.overlay_shortcut, "Alt+F9");
        assert!(!parsed.close_overlay_on_outside_click);
    }

    #[test]
    fn overlay_and_direct_save_shortcuts_cannot_collide() {
        let state = fixture();
        let result = set_hotkeys(
            &state,
            "F8".to_owned(),
            vec![ReplayShortcutBinding {
                shortcut: "F8".to_owned(),
                duration: ReplayDuration::Seconds30,
            }],
        );
        assert!(result.is_err());
        assert!(!state.exists());
    }

    #[test]
    fn overlay_behavior_update_preserves_direct_save_bindings() {
        let state = fixture();
        let saved = set_overlay_behavior(&state, "Ctrl+Tab".to_owned(), false)
            .expect("save overlay behavior");
        assert_eq!(saved.overlay_shortcut, "Ctrl+Tab");
        assert!(!saved.close_overlay_on_outside_click);
        assert_eq!(
            saved.save_shortcuts,
            ReplayPreferences::default().save_shortcuts
        );

        assert!(set_overlay_behavior(&state, "F8".to_owned(), true).is_err());
        let _ = fs::remove_dir_all(state);
    }
}
