use redunar_core::{
    EffectiveGameProfile, GameCatalog, GameId, GameLaunchConfig, GameMatchRule, GameProcess,
    GameRecord, GameResolution, GlobalGameProfile, Inheritable, OverlayCorner, OverlayLayout,
    OverlayMetricSet, OverlayOpacity, OverlayPalette, OverlayPreset, OverlayScale, PerGameProfile,
    ReplayDuration, ReplayFrameRate, ReplayQuality, ReplaySettings, ReplayStorageLimit,
    resolve_game,
};
use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

const CATALOG_FILE: &str = "games-v1.txt";
const CATALOG_HEADER_V1: &str = "redunar-games-v1";
const CATALOG_HEADER_V2: &str = "redunar-games-v2";
const CATALOG_HEADER_V3: &str = "redunar-games-v3";
const CATALOG_HEADER_V4: &str = "redunar-games-v4";
const CATALOG_HEADER_V5: &str = "redunar-games-v5";
const CATALOG_HEADER_V6: &str = "redunar-games-v6";
const CATALOG_HEADER_V7: &str = "redunar-games-v7";
const CATALOG_HEADER_V8: &str = "redunar-games-v8";
const CATALOG_HEADER_V9: &str = "redunar-games-v9";
const CATALOG_HEADER_V10: &str = "redunar-games-v10";
const CATALOG_HEADER_V11: &str = "redunar-games-v11";
const CATALOG_HEADER_V12: &str = "redunar-games-v12";
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;
const MAX_GAMES: usize = 256;
const MAX_IMPORT_GAMES: usize = 64;
const MAX_DISPLAY_NAME_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 4096;
const MAX_TOTAL_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_MATCH_RULES: usize = 32;
const MAX_EXECUTABLE_NAME_BYTES: usize = 256;
const MAX_TOTAL_MATCH_RULE_BYTES: usize = 64 * 1024;
static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);
static CATALOG_OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddGameRequest {
    pub display_name: String,
    pub launch: GameLaunchConfig,
    /// Optional local identity evidence supplied by a detector or launcher
    /// adapter. The daemon also adds rules for the direct launch executable.
    pub match_rules: Vec<GameMatchRule>,
    identity: LaunchIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchIdentity {
    DirectExecutable,
    ExternalLauncher,
}

/// Catalog resolution plus a profile only when identity evidence is safe for
/// automatic activation. Suggested and ambiguous matches never carry one.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCatalogGame {
    pub resolution: GameResolution,
    pub effective_profile: Option<EffectiveGameProfile>,
}

struct StoredCatalog {
    catalog: GameCatalog,
    next_id: u64,
}

impl Default for StoredCatalog {
    fn default() -> Self {
        Self {
            catalog: GameCatalog::default(),
            next_id: 1,
        }
    }
}

impl AddGameRequest {
    #[must_use]
    pub fn new(display_name: impl Into<String>, executable: impl Into<PathBuf>) -> Self {
        let executable = executable.into();
        Self {
            display_name: display_name.into(),
            launch: GameLaunchConfig {
                working_directory: executable.parent().map(Path::to_path_buf),
                executable,
                arguments: Vec::new(),
            },
            match_rules: Vec::new(),
            identity: LaunchIdentity::DirectExecutable,
        }
    }

    pub(crate) fn external_launcher(
        display_name: String,
        launch: GameLaunchConfig,
        match_rules: Vec<GameMatchRule>,
    ) -> Self {
        Self {
            display_name,
            launch,
            match_rules,
            identity: LaunchIdentity::ExternalLauncher,
        }
    }
}

pub(crate) fn load(state_directory: &Path) -> Result<GameCatalog, GameCatalogError> {
    let _operation = catalog_operation();
    load_stored(state_directory).map(|stored| stored.catalog)
}

#[cfg(test)]
fn effective_profile(
    state_directory: &Path,
    id: GameId,
) -> Result<EffectiveGameProfile, GameCatalogError> {
    let _operation = catalog_operation();
    let stored = load_stored(state_directory)?;
    let game = stored
        .catalog
        .games
        .iter()
        .find(|game| game.id == id)
        .ok_or_else(|| GameCatalogError::new("the selected game no longer exists"))?;
    Ok(game.profile.resolve(stored.catalog.global_profile))
}

pub(crate) fn resolve_process(
    state_directory: &Path,
    process: &GameProcess,
) -> Result<ResolvedCatalogGame, GameCatalogError> {
    let _operation = catalog_operation();
    let stored = load_stored(state_directory)?;
    let resolution = resolve_game(&stored.catalog.games, process);
    let effective_profile = match resolution {
        GameResolution::Automatic(id) => stored
            .catalog
            .games
            .iter()
            .find(|game| game.id == id)
            .map(|game| game.profile.resolve(stored.catalog.global_profile)),
        GameResolution::Suggested(_) | GameResolution::Ambiguous(_) | GameResolution::Unknown => {
            None
        }
    };
    Ok(ResolvedCatalogGame {
        resolution,
        effective_profile,
    })
}

fn catalog_operation() -> MutexGuard<'static, ()> {
    CATALOG_OPERATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn load_stored(state_directory: &Path) -> Result<StoredCatalog, GameCatalogError> {
    let path = state_directory.join(CATALOG_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(StoredCatalog::default());
        }
        Err(error) => return Err(read_error(&path, &error)),
    };
    if !metadata.file_type().is_file() {
        return Err(GameCatalogError::owned(format!(
            "could not load local game catalog from {}: state is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > MAX_CATALOG_BYTES {
        return Err(GameCatalogError::owned(format!(
            "could not load local game catalog from {}: state exceeds {MAX_CATALOG_BYTES} bytes",
            path.display()
        )));
    }

    let mut contents = String::new();
    File::open(&path)
        .and_then(|file| {
            file.take(MAX_CATALOG_BYTES + 1)
                .read_to_string(&mut contents)
        })
        .map_err(|error| read_error(&path, &error))?;
    parse(&contents).map_err(|error| {
        GameCatalogError::owned(format!(
            "could not load local game catalog from {}: {error}",
            path.display()
        ))
    })
}

pub(crate) fn add(
    state_directory: &Path,
    request: AddGameRequest,
) -> Result<GameRecord, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    if stored.catalog.games.len() >= MAX_GAMES {
        return Err(GameCatalogError::new("the local game catalog is full"));
    }
    let request = normalize_request(request)?;
    if requests_conflict_with_catalog(&stored.catalog.games, std::slice::from_ref(&request)) {
        return Err(GameCatalogError::new(
            "this game identity already belongs to a local game",
        ));
    }
    let id = GameId::new(stored.next_id)
        .map_err(|_| GameCatalogError::new("could not allocate a local game identifier"))?;
    stored.next_id = stored
        .next_id
        .checked_add(1)
        .ok_or_else(|| GameCatalogError::new("the local game identifier space is exhausted"))?;
    let game = GameRecord {
        id,
        display_name: request.display_name,
        match_rules: request.match_rules,
        launch: request.launch,
        profile: PerGameProfile::default(),
    };
    stored.catalog.games.push(game.clone());
    save(state_directory, &stored)?;
    Ok(game)
}

pub(crate) fn import(
    state_directory: &Path,
    requests: Vec<AddGameRequest>,
) -> Result<Vec<GameRecord>, GameCatalogError> {
    let _operation = catalog_operation();
    if requests.is_empty() || requests.len() > MAX_IMPORT_GAMES {
        return Err(GameCatalogError::new(
            "a game import must contain between 1 and 64 entries",
        ));
    }
    let mut stored = load_stored(state_directory)?;
    if stored.catalog.games.len().saturating_add(requests.len()) > MAX_GAMES {
        return Err(GameCatalogError::new(
            "the game import would exceed the local catalog limit",
        ));
    }

    let requests = requests
        .into_iter()
        .map(normalize_request)
        .collect::<Result<Vec<_>, _>>()?;
    if requests_conflict_with_catalog(&stored.catalog.games, &requests)
        || requests_have_duplicate_identities(&requests)
    {
        return Err(GameCatalogError::new(
            "the game import contains an identity already in the catalog or batch",
        ));
    }

    let mut imported = Vec::with_capacity(requests.len());
    for request in requests {
        let id = GameId::new(stored.next_id)
            .map_err(|_| GameCatalogError::new("could not allocate a local game identifier"))?;
        stored.next_id = stored
            .next_id
            .checked_add(1)
            .ok_or_else(|| GameCatalogError::new("the local game identifier space is exhausted"))?;
        imported.push(GameRecord {
            id,
            display_name: request.display_name,
            match_rules: request.match_rules,
            launch: request.launch,
            profile: PerGameProfile::default(),
        });
    }
    stored.catalog.games.extend(imported.iter().cloned());
    save(state_directory, &stored)?;
    Ok(imported)
}

pub(crate) fn update_launch(
    state_directory: &Path,
    id: GameId,
    launch: GameLaunchConfig,
) -> Result<GameRecord, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    let (display_name, match_rules, identity) = stored
        .catalog
        .games
        .iter()
        .find(|game| game.id == id)
        .map(|game| {
            let identity = if game.match_rules.iter().any(
                |rule| matches!(rule, GameMatchRule::Executable(path) if path == &game.launch.executable),
            ) {
                LaunchIdentity::DirectExecutable
            } else {
                LaunchIdentity::ExternalLauncher
            };
            (game.display_name.clone(), game.match_rules.clone(), identity)
        })
        .ok_or_else(|| GameCatalogError::new("the selected game no longer exists"))?;
    let normalized = normalize_request(AddGameRequest {
        display_name,
        launch,
        match_rules,
        identity,
    })?;
    let other_games = stored
        .catalog
        .games
        .iter()
        .filter(|game| game.id != id)
        .cloned()
        .collect::<Vec<_>>();
    if requests_conflict_with_catalog(&other_games, std::slice::from_ref(&normalized)) {
        return Err(GameCatalogError::new(
            "this game identity already belongs to a different local game",
        ));
    }
    let game = stored
        .catalog
        .games
        .iter_mut()
        .find(|game| game.id == id)
        .expect("game was checked above");
    game.launch = normalized.launch;
    game.match_rules = normalized.match_rules;
    let updated = game.clone();
    save(state_directory, &stored)?;
    Ok(updated)
}

pub(crate) fn refresh_discovered_launches(
    state_directory: &Path,
    requests: Vec<AddGameRequest>,
) -> Result<Vec<GameRecord>, GameCatalogError> {
    let _operation = catalog_operation();
    if requests.len() > MAX_GAMES {
        return Err(GameCatalogError::new(
            "the discovery refresh exceeds the local catalog limit",
        ));
    }
    let requests = requests
        .into_iter()
        .map(normalize_request)
        .collect::<Result<Vec<_>, _>>()?;
    let mut stored = load_stored(state_directory)?;
    let mut updated_ids = HashSet::new();
    let mut updated = Vec::new();

    for request in requests {
        let matching = stored
            .catalog
            .games
            .iter()
            .enumerate()
            .filter(|(_, game)| {
                strong_identities(&request.match_rules).any(|candidate| {
                    strong_identities(&game.match_rules).any(|saved| saved == candidate)
                })
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [index] = matching.as_slice() else {
            continue;
        };
        let game = &stored.catalog.games[*index];
        if game.launch == request.launch || !updated_ids.insert(game.id) {
            continue;
        }
        let identity = if game.match_rules.iter().any(
            |rule| matches!(rule, GameMatchRule::Executable(path) if path == &game.launch.executable),
        ) {
            LaunchIdentity::DirectExecutable
        } else {
            LaunchIdentity::ExternalLauncher
        };
        let mut match_rules = game.match_rules.clone();
        match_rules.extend(request.match_rules);
        let normalized = normalize_request(AddGameRequest {
            display_name: game.display_name.clone(),
            launch: request.launch,
            match_rules,
            identity,
        })?;
        let game = &mut stored.catalog.games[*index];
        game.launch = normalized.launch;
        game.match_rules = normalized.match_rules;
        updated.push(game.clone());
    }

    if updated.is_empty() {
        return Ok(updated);
    }
    validate_stored(&stored)?;
    save(state_directory, &stored)?;
    Ok(updated)
}

pub(crate) fn update_profile(
    state_directory: &Path,
    id: GameId,
    profile: PerGameProfile,
) -> Result<GameRecord, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    let game = stored
        .catalog
        .games
        .iter_mut()
        .find(|game| game.id == id)
        .ok_or_else(|| GameCatalogError::new("the selected game no longer exists"))?;
    game.profile = profile;
    let updated = game.clone();
    save(state_directory, &stored)?;
    Ok(updated)
}

pub(crate) fn remove(state_directory: &Path, id: GameId) -> Result<GameRecord, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    let index = stored
        .catalog
        .games
        .iter()
        .position(|game| game.id == id)
        .ok_or_else(|| GameCatalogError::new("the selected game no longer exists"))?;
    let removed = stored.catalog.games.remove(index);
    save(state_directory, &stored)?;
    Ok(removed)
}

pub(crate) fn update_global_profile(
    state_directory: &Path,
    profile: GlobalGameProfile,
) -> Result<GameCatalog, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    stored.catalog.global_profile = profile;
    save(state_directory, &stored)?;
    Ok(stored.catalog)
}

pub(crate) fn update_global_replay_settings(
    state_directory: &Path,
    settings: ReplaySettings,
) -> Result<GameCatalog, GameCatalogError> {
    let _operation = catalog_operation();
    let mut stored = load_stored(state_directory)?;
    stored.catalog.global_profile.replay = settings;
    save(state_directory, &stored)?;
    Ok(stored.catalog)
}

fn normalize_request(mut request: AddGameRequest) -> Result<AddGameRequest, GameCatalogError> {
    validate_display_name(&request.display_name)?;
    validate_arguments(&request.launch.arguments)?;
    validate_path("game executable", &request.launch.executable)?;
    let metadata = fs::metadata(&request.launch.executable).map_err(|error| {
        GameCatalogError::owned(format!("could not inspect the game executable: {error}"))
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(GameCatalogError::new(
            "the selected game must be an executable regular file",
        ));
    }
    request.launch.executable = fs::canonicalize(&request.launch.executable).map_err(|error| {
        GameCatalogError::owned(format!("could not resolve the game executable: {error}"))
    })?;

    let mut match_rules = match request.identity {
        LaunchIdentity::DirectExecutable => match_rules_for(&request.launch.executable),
        LaunchIdentity::ExternalLauncher => Vec::new(),
    };
    match_rules.extend(request.match_rules);
    request.match_rules = normalize_match_rules(match_rules)?;

    if let Some(directory) = &request.launch.working_directory {
        validate_path("game working directory", directory)?;
        let metadata = fs::metadata(directory).map_err(|error| {
            GameCatalogError::owned(format!(
                "could not inspect the game working directory: {error}"
            ))
        })?;
        if !metadata.is_dir() {
            return Err(GameCatalogError::new(
                "the game working directory is not a directory",
            ));
        }
        request.launch.working_directory = Some(fs::canonicalize(directory).map_err(|error| {
            GameCatalogError::owned(format!(
                "could not resolve the game working directory: {error}"
            ))
        })?);
    }
    Ok(request)
}

fn validate_display_name(name: &str) -> Result<(), GameCatalogError> {
    if name.trim().is_empty()
        || name.len() > MAX_DISPLAY_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err(GameCatalogError::new(
            "game name must be plain text between 1 and 256 bytes",
        ));
    }
    Ok(())
}

fn validate_path(label: &str, path: &Path) -> Result<(), GameCatalogError> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute() || bytes.is_empty() || bytes.len() > MAX_PATH_BYTES || bytes.contains(&0)
    {
        return Err(GameCatalogError::owned(format!(
            "{label} must be an absolute path no longer than {MAX_PATH_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_arguments(arguments: &[OsString]) -> Result<(), GameCatalogError> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(GameCatalogError::new(
            "the game has too many launch arguments",
        ));
    }
    let mut total = 0_usize;
    for argument in arguments {
        let bytes = argument.as_bytes();
        if bytes.len() > MAX_ARGUMENT_BYTES || bytes.contains(&0) {
            return Err(GameCatalogError::new(
                "a game launch argument is invalid or too large",
            ));
        }
        total = total.saturating_add(bytes.len());
    }
    if total > MAX_TOTAL_ARGUMENT_BYTES {
        return Err(GameCatalogError::new(
            "the combined game launch arguments are too large",
        ));
    }
    Ok(())
}

fn normalize_match_rules(
    match_rules: Vec<GameMatchRule>,
) -> Result<Vec<GameMatchRule>, GameCatalogError> {
    // Normalization prepends at most the launch path and basename. Allow those
    // two entries to duplicate supplied evidence before enforcing the stored
    // bound, while still rejecting unbounded caller input before scanning it.
    if match_rules.len() > MAX_MATCH_RULES + 2 {
        return Err(GameCatalogError::new(
            "the game has too many local match rules",
        ));
    }
    let mut normalized = Vec::with_capacity(match_rules.len());
    for rule in match_rules {
        validate_match_rule(&rule)?;
        if !normalized.contains(&rule) {
            normalized.push(rule);
        }
    }
    validate_match_rules(&normalized)?;
    Ok(normalized)
}

fn validate_match_rules(match_rules: &[GameMatchRule]) -> Result<(), GameCatalogError> {
    if match_rules.is_empty() || match_rules.len() > MAX_MATCH_RULES {
        return Err(GameCatalogError::new(
            "a game must have between 1 and 32 local match rules",
        ));
    }
    let mut total_bytes = 0_usize;
    for (index, rule) in match_rules.iter().enumerate() {
        validate_match_rule(rule)?;
        if match_rules[..index].contains(rule) {
            return Err(GameCatalogError::new(
                "the game contains duplicate local match rules",
            ));
        }
        total_bytes = total_bytes.saturating_add(match_rule_bytes(rule));
    }
    if total_bytes > MAX_TOTAL_MATCH_RULE_BYTES {
        return Err(GameCatalogError::new(
            "the combined local match rules are too large",
        ));
    }
    Ok(())
}

fn validate_match_rule(rule: &GameMatchRule) -> Result<(), GameCatalogError> {
    match rule {
        GameMatchRule::SteamAppId(0) => Err(GameCatalogError::new(
            "a Steam app identifier must be greater than zero",
        )),
        GameMatchRule::Executable(path) => validate_path("game match executable", path),
        GameMatchRule::ExecutableName(name)
            if name.is_empty()
                || name.len() > MAX_EXECUTABLE_NAME_BYTES
                || name.contains('/')
                || name.chars().any(char::is_control) =>
        {
            Err(GameCatalogError::new(
                "an executable-name alias must be plain text between 1 and 256 bytes",
            ))
        }
        GameMatchRule::SteamAppId(_) | GameMatchRule::ExecutableName(_) => Ok(()),
    }
}

fn match_rule_bytes(rule: &GameMatchRule) -> usize {
    match rule {
        GameMatchRule::SteamAppId(_) => std::mem::size_of::<u32>(),
        GameMatchRule::Executable(path) => path.as_os_str().as_bytes().len(),
        GameMatchRule::ExecutableName(name) => name.len(),
    }
}

fn match_rules_for(executable: &Path) -> Vec<GameMatchRule> {
    let mut rules = vec![GameMatchRule::Executable(executable.to_path_buf())];
    if let Some(name) = executable.file_name().and_then(OsStr::to_str) {
        rules.push(GameMatchRule::ExecutableName(name.to_owned()));
    }
    rules
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum StrongIdentity {
    SteamAppId(u32),
    Executable(PathBuf),
}

fn strong_identities(match_rules: &[GameMatchRule]) -> impl Iterator<Item = StrongIdentity> + '_ {
    match_rules.iter().filter_map(|rule| match rule {
        GameMatchRule::SteamAppId(app_id) => Some(StrongIdentity::SteamAppId(*app_id)),
        GameMatchRule::Executable(path) => Some(StrongIdentity::Executable(path.clone())),
        GameMatchRule::ExecutableName(_) => None,
    })
}

fn requests_conflict_with_catalog(games: &[GameRecord], requests: &[AddGameRequest]) -> bool {
    let existing = games
        .iter()
        .flat_map(|game| strong_identities(&game.match_rules))
        .collect::<BTreeSet<_>>();
    requests.iter().any(|request| {
        strong_identities(&request.match_rules).any(|identity| existing.contains(&identity))
    })
}

fn requests_have_duplicate_identities(requests: &[AddGameRequest]) -> bool {
    let mut identities = BTreeSet::new();
    requests.iter().any(|request| {
        strong_identities(&request.match_rules).any(|identity| !identities.insert(identity))
    })
}

fn save(state_directory: &Path, stored: &StoredCatalog) -> Result<(), GameCatalogError> {
    validate_stored(stored)?;
    let contents = serialize(stored);
    if u64::try_from(contents.len()).unwrap_or(u64::MAX) > MAX_CATALOG_BYTES {
        return Err(GameCatalogError::new("the local game catalog is too large"));
    }
    fs::create_dir_all(state_directory).map_err(|error| write_error(state_directory, &error))?;
    fs::set_permissions(state_directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| write_error(state_directory, &error))?;
    let destination = state_directory.join(CATALOG_FILE);
    atomic_write(&destination, contents.as_bytes())
        .map_err(|error| write_error(&destination, &error))
}

fn validate_stored(stored: &StoredCatalog) -> Result<(), GameCatalogError> {
    let catalog = &stored.catalog;
    if catalog.games.len() > MAX_GAMES {
        return Err(GameCatalogError::new("the local game catalog is too large"));
    }
    let mut ids = BTreeSet::new();
    let mut executable_identities = BTreeSet::new();
    for game in &catalog.games {
        if !ids.insert(game.id.get())
            || strong_identities(&game.match_rules).any(|identity| match identity {
                StrongIdentity::Executable(path) => !executable_identities.insert(path),
                // Older catalogs may contain the same launcher-supplied ID on
                // multiple records. Resolution already treats that as
                // ambiguous and activates no profile; preserve readable state
                // while preventing new duplicates at import time.
                StrongIdentity::SteamAppId(_) => false,
            })
        {
            return Err(GameCatalogError::new(
                "the local game catalog contains duplicate identities",
            ));
        }
        validate_display_name(&game.display_name)?;
        validate_path("game executable", &game.launch.executable)?;
        validate_arguments(&game.launch.arguments)?;
        validate_match_rules(&game.match_rules)?;
        if let Some(directory) = &game.launch.working_directory {
            validate_path("game working directory", directory)?;
        }
    }
    if catalog
        .games
        .iter()
        .any(|game| game.id.get() >= stored.next_id)
    {
        return Err(GameCatalogError::new(
            "the local game catalog next identifier is invalid",
        ));
    }
    Ok(())
}

fn serialize(stored: &StoredCatalog) -> String {
    let catalog = &stored.catalog;
    let mut output = String::from(CATALOG_HEADER_V12);
    output.push('\n');
    writeln!(output, "next\t{}", stored.next_id).expect("write to string");
    writeln!(
        output,
        "global\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        bool_token(catalog.global_profile.capture_metrics),
        bool_token(catalog.global_profile.overlay_visible),
        overlay_preset_token(catalog.global_profile.overlay_preset),
        catalog.global_profile.overlay_metrics.bits(),
        overlay_corner_token(catalog.global_profile.overlay_corner),
        catalog.global_profile.overlay_opacity.percent(),
        catalog.global_profile.overlay_scale.percent(),
        overlay_layout_token(catalog.global_profile.overlay_layout),
        overlay_palette_token(catalog.global_profile.overlay_palette),
        bool_token(catalog.global_profile.gamescope_enabled),
        bool_token(catalog.global_profile.gamemode_enabled),
        replay_duration_token(catalog.global_profile.replay.duration),
        replay_frame_rate_token(catalog.global_profile.replay.frame_rate),
        replay_quality_token(catalog.global_profile.replay.quality),
        replay_storage_token(catalog.global_profile.replay.storage_limit),
        bool_token(catalog.global_profile.instant_replay),
        bool_token(catalog.global_profile.overlay_branding)
    )
    .expect("write to string");
    for game in &catalog.games {
        let working_directory = game
            .launch
            .working_directory
            .as_deref()
            .map_or_else(|| "-".to_owned(), |value| hex(value.as_os_str().as_bytes()));
        let arguments = if game.launch.arguments.is_empty() {
            "-".to_owned()
        } else {
            game.launch
                .arguments
                .iter()
                .map(|value| hex(value.as_bytes()))
                .collect::<Vec<_>>()
                .join(",")
        };
        let match_rules = serialize_match_rules(&game.match_rules);
        writeln!(
            output,
            "game\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            game.id.get(),
            hex(game.display_name.as_bytes()),
            hex(game.launch.executable.as_os_str().as_bytes()),
            working_directory,
            arguments,
            inheritable_token(game.profile.capture_metrics),
            inheritable_token(game.profile.overlay_visible),
            inheritable_token(game.profile.instant_replay),
            match_rules
        )
        .expect("write to string");
    }
    output
}

fn parse(contents: &str) -> Result<StoredCatalog, GameCatalogError> {
    let mut lines = contents.lines();
    let version = match lines.next() {
        Some(CATALOG_HEADER_V1) => 1,
        Some(CATALOG_HEADER_V2) => 2,
        Some(CATALOG_HEADER_V3) => 3,
        Some(CATALOG_HEADER_V4) => 4,
        Some(CATALOG_HEADER_V5) => 5,
        Some(CATALOG_HEADER_V6) => 6,
        Some(CATALOG_HEADER_V7) => 7,
        Some(CATALOG_HEADER_V8) => 8,
        Some(CATALOG_HEADER_V9) => 9,
        Some(CATALOG_HEADER_V10) => 10,
        Some(CATALOG_HEADER_V11) => 11,
        Some(CATALOG_HEADER_V12) => 12,
        _ => {
            return Err(GameCatalogError::new(
                "catalog header or version is invalid",
            ));
        }
    };
    let next = lines
        .next()
        .ok_or_else(|| GameCatalogError::new("catalog next identifier is missing"))?;
    let next_fields = next.split('\t').collect::<Vec<_>>();
    if next_fields.len() != 2 || next_fields[0] != "next" {
        return Err(GameCatalogError::new(
            "catalog next identifier is malformed",
        ));
    }
    let next_id = next_fields[1]
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| GameCatalogError::new("catalog next identifier is invalid"))?;
    let global = lines
        .next()
        .ok_or_else(|| GameCatalogError::new("catalog global profile is missing"))?;
    let global_profile = parse_global_profile(global, version)?;
    let mut games = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        games.push(parse_game(line, version)?);
    }
    let stored = StoredCatalog {
        catalog: GameCatalog {
            global_profile,
            games,
        },
        next_id,
    };
    validate_stored(&stored)?;
    Ok(stored)
}

#[expect(
    clippy::too_many_lines,
    reason = "versioned profile parsing remains ordered for migration auditing"
)]
fn parse_global_profile(line: &str, version: u8) -> Result<GlobalGameProfile, GameCatalogError> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let expected_fields = if version >= 12 {
        18
    } else if version >= 11 {
        17
    } else if version >= 10 {
        15
    } else if version >= 9 {
        16
    } else if version >= 8 {
        15
    } else if version >= 7 {
        14
    } else if version >= 6 {
        13
    } else if version >= 5 {
        9
    } else if version >= 4 {
        8
    } else {
        4 + usize::from(version >= 3)
    };
    if fields.len() != expected_fields || fields[0] != "global" {
        return Err(GameCatalogError::new("catalog global profile is malformed"));
    }
    Ok(GlobalGameProfile {
        capture_metrics: parse_bool(fields[1])?,
        overlay_visible: if version < 3 {
            false
        } else {
            parse_bool(fields[2])?
        },
        overlay_preset: if version < 4 {
            OverlayPreset::default()
        } else {
            parse_overlay_preset(fields[3])?
        },
        overlay_metrics: if version < 5 {
            OverlayMetricSet::default()
        } else {
            fields[4]
                .parse::<u16>()
                .ok()
                .and_then(|bits| OverlayMetricSet::from_bits(bits).ok())
                .ok_or_else(|| GameCatalogError::new("catalog overlay metrics are invalid"))?
        },
        overlay_corner: if version < 4 {
            OverlayCorner::default()
        } else {
            parse_overlay_corner(fields[usize::from(version >= 5) + 4])?
        },
        overlay_opacity: if version < 4 {
            OverlayOpacity::default()
        } else {
            fields[usize::from(version >= 5) + 5]
                .parse::<u8>()
                .ok()
                .and_then(|value| OverlayOpacity::new(value).ok())
                .ok_or_else(|| GameCatalogError::new("catalog overlay opacity is invalid"))?
        },
        overlay_scale: if version < 7 {
            OverlayScale::default()
        } else {
            fields[7]
                .parse::<u8>()
                .ok()
                .and_then(OverlayScale::new)
                .ok_or_else(|| GameCatalogError::new("catalog overlay scale is invalid"))?
        },
        overlay_layout: if version < 11 {
            OverlayLayout::default()
        } else {
            parse_overlay_layout(fields[8])?
        },
        overlay_palette: if version < 11 {
            OverlayPalette::default()
        } else {
            parse_overlay_palette(fields[9])?
        },
        overlay_branding: if version < 12 {
            true
        } else {
            parse_bool(fields[17])?
        },
        gamescope_enabled: if version >= 8 {
            parse_bool(fields[8 + 2 * usize::from(version >= 11)])?
        } else {
            false
        },
        gamemode_enabled: if version >= 9 {
            parse_bool(fields[9 + 2 * usize::from(version >= 11)])?
        } else {
            false
        },
        replay: if version < 6 {
            ReplaySettings::default()
        } else {
            ReplaySettings {
                duration: parse_replay_duration(
                    fields[7
                        + usize::from(version >= 7)
                        + usize::from(version >= 8)
                        + usize::from(version >= 9)
                        + 2 * usize::from(version >= 11)],
                )?,
                frame_rate: parse_replay_frame_rate(
                    fields[8
                        + usize::from(version >= 7)
                        + usize::from(version >= 8)
                        + usize::from(version >= 9)
                        + 2 * usize::from(version >= 11)],
                )?,
                quality: parse_replay_quality(
                    fields[9
                        + usize::from(version >= 7)
                        + usize::from(version >= 8)
                        + usize::from(version >= 9)
                        + 2 * usize::from(version >= 11)],
                )?,
                storage_limit: parse_replay_storage(
                    fields[10
                        + usize::from(version >= 7)
                        + usize::from(version >= 8)
                        + usize::from(version >= 9)
                        + 2 * usize::from(version >= 11)],
                )?,
            }
        },
        instant_replay: parse_bool(if version >= 6 {
            fields[11
                + usize::from(version >= 7)
                + usize::from(version >= 8)
                + usize::from(version >= 9)
                + 2 * usize::from(version >= 11)]
        } else if version >= 4 {
            fields[usize::from(version >= 5) + 6]
        } else {
            fields[2 + usize::from(version >= 3)]
        })?,
    })
}

fn parse_game(line: &str, version: u8) -> Result<GameRecord, GameCatalogError> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let overlay_offset = usize::from(version >= 3);
    let expected_fields = match version {
        1 => 9,
        3..=9 => 11,
        _ => 10,
    };
    if fields.len() != expected_fields || fields[0] != "game" {
        return Err(GameCatalogError::new("catalog game entry is malformed"));
    }
    let id = fields[1]
        .parse::<u64>()
        .ok()
        .and_then(|value| GameId::new(value).ok())
        .ok_or_else(|| GameCatalogError::new("catalog game id is invalid"))?;
    let display_name = String::from_utf8(unhex(fields[2])?)
        .map_err(|_| GameCatalogError::new("catalog game name is not UTF-8"))?;
    let executable = PathBuf::from(OsString::from_vec(unhex(fields[3])?));
    let working_directory = if fields[4] == "-" {
        None
    } else {
        Some(PathBuf::from(OsString::from_vec(unhex(fields[4])?)))
    };
    let arguments = if fields[5] == "-" {
        Vec::new()
    } else {
        fields[5]
            .split(',')
            .map(unhex)
            .map(|value| value.map(OsString::from_vec))
            .collect::<Result<Vec<_>, _>>()?
    };
    let profile = PerGameProfile {
        capture_metrics: parse_inheritable(fields[6])?,
        overlay_visible: if version < 3 {
            Inheritable::InheritGlobal
        } else {
            parse_inheritable(fields[7])?
        },
        instant_replay: parse_inheritable(fields[7 + overlay_offset])?,
    };
    let match_rules = if version == 1 {
        match_rules_for(&executable)
    } else {
        parse_match_rules(
            fields[if version >= 10 {
                8 + overlay_offset
            } else {
                9 + overlay_offset
            }],
        )?
    };
    Ok(GameRecord {
        id,
        display_name,
        match_rules,
        launch: GameLaunchConfig {
            executable,
            arguments,
            working_directory,
        },
        profile,
    })
}

const fn overlay_preset_token(value: OverlayPreset) -> &'static str {
    match value {
        OverlayPreset::Compact => "compact",
        OverlayPreset::FpsOnly => "fps-only",
        OverlayPreset::Detailed => "detailed",
        OverlayPreset::Custom => "custom",
    }
}

fn parse_overlay_preset(value: &str) -> Result<OverlayPreset, GameCatalogError> {
    match value {
        "compact" => Ok(OverlayPreset::Compact),
        "fps-only" => Ok(OverlayPreset::FpsOnly),
        "detailed" => Ok(OverlayPreset::Detailed),
        "custom" => Ok(OverlayPreset::Custom),
        _ => Err(GameCatalogError::new("catalog overlay preset is invalid")),
    }
}

const fn overlay_layout_token(value: OverlayLayout) -> &'static str {
    match value {
        OverlayLayout::Grid => "grid",
        OverlayLayout::Ribbon => "ribbon",
        OverlayLayout::Telemetry => "telemetry",
    }
}

fn parse_overlay_layout(value: &str) -> Result<OverlayLayout, GameCatalogError> {
    match value {
        "grid" => Ok(OverlayLayout::Grid),
        "ribbon" => Ok(OverlayLayout::Ribbon),
        "telemetry" => Ok(OverlayLayout::Telemetry),
        _ => Err(GameCatalogError::new("catalog overlay layout is invalid")),
    }
}

const fn overlay_palette_token(value: OverlayPalette) -> &'static str {
    match value {
        OverlayPalette::Redunar => "redunar",
        OverlayPalette::Glacier => "glacier",
        OverlayPalette::Ember => "ember",
        OverlayPalette::Mint => "mint",
        OverlayPalette::Mono => "mono",
        OverlayPalette::Amethyst => "amethyst",
        OverlayPalette::Solar => "solar",
        OverlayPalette::Rose => "rose",
    }
}

fn parse_overlay_palette(value: &str) -> Result<OverlayPalette, GameCatalogError> {
    match value {
        "redunar" => Ok(OverlayPalette::Redunar),
        "glacier" => Ok(OverlayPalette::Glacier),
        "ember" => Ok(OverlayPalette::Ember),
        "mint" => Ok(OverlayPalette::Mint),
        "mono" => Ok(OverlayPalette::Mono),
        "amethyst" => Ok(OverlayPalette::Amethyst),
        "solar" => Ok(OverlayPalette::Solar),
        "rose" => Ok(OverlayPalette::Rose),
        _ => Err(GameCatalogError::new("catalog overlay palette is invalid")),
    }
}

const fn overlay_corner_token(value: OverlayCorner) -> &'static str {
    match value {
        OverlayCorner::TopLeft => "top-left",
        OverlayCorner::TopRight => "top-right",
        OverlayCorner::BottomLeft => "bottom-left",
        OverlayCorner::BottomRight => "bottom-right",
    }
}

fn parse_overlay_corner(value: &str) -> Result<OverlayCorner, GameCatalogError> {
    match value {
        "top-left" => Ok(OverlayCorner::TopLeft),
        "top-right" => Ok(OverlayCorner::TopRight),
        "bottom-left" => Ok(OverlayCorner::BottomLeft),
        "bottom-right" => Ok(OverlayCorner::BottomRight),
        _ => Err(GameCatalogError::new("catalog overlay corner is invalid")),
    }
}

const fn replay_duration_token(value: ReplayDuration) -> &'static str {
    match value {
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

fn parse_replay_duration(value: &str) -> Result<ReplayDuration, GameCatalogError> {
    match value {
        "15" => Ok(ReplayDuration::Seconds15),
        "30" => Ok(ReplayDuration::Seconds30),
        "60" => Ok(ReplayDuration::Seconds60),
        "120" => Ok(ReplayDuration::Seconds120),
        "180" => Ok(ReplayDuration::Seconds180),
        "300" => Ok(ReplayDuration::Seconds300),
        "600" => Ok(ReplayDuration::Seconds600),
        "900" => Ok(ReplayDuration::Seconds900),
        _ => Err(GameCatalogError::new("catalog replay duration is invalid")),
    }
}

const fn replay_frame_rate_token(value: ReplayFrameRate) -> &'static str {
    match value {
        ReplayFrameRate::Fps30 => "30",
        ReplayFrameRate::Fps60 => "60",
        ReplayFrameRate::Fps120 => "120",
        ReplayFrameRate::Variable => "variable",
    }
}

fn parse_replay_frame_rate(value: &str) -> Result<ReplayFrameRate, GameCatalogError> {
    match value {
        "30" => Ok(ReplayFrameRate::Fps30),
        "60" | "adaptive" => Ok(ReplayFrameRate::Fps60),
        "120" => Ok(ReplayFrameRate::Fps120),
        "variable" => Ok(ReplayFrameRate::Variable),
        _ => Err(GameCatalogError::new(
            "catalog replay frame rate is invalid",
        )),
    }
}

const fn replay_quality_token(value: ReplayQuality) -> &'static str {
    match value {
        ReplayQuality::Efficient => "efficient",
        ReplayQuality::Balanced => "balanced",
        ReplayQuality::High => "high",
    }
}

fn parse_replay_quality(value: &str) -> Result<ReplayQuality, GameCatalogError> {
    match value {
        "efficient" => Ok(ReplayQuality::Efficient),
        "balanced" => Ok(ReplayQuality::Balanced),
        "high" => Ok(ReplayQuality::High),
        _ => Err(GameCatalogError::new("catalog replay quality is invalid")),
    }
}

const fn replay_storage_token(value: ReplayStorageLimit) -> &'static str {
    match value {
        ReplayStorageLimit::GiB5 => "5",
        ReplayStorageLimit::GiB10 => "10",
        ReplayStorageLimit::GiB25 => "25",
        ReplayStorageLimit::GiB50 => "50",
        ReplayStorageLimit::Unlimited => "unlimited",
    }
}

fn parse_replay_storage(value: &str) -> Result<ReplayStorageLimit, GameCatalogError> {
    match value {
        "5" => Ok(ReplayStorageLimit::GiB5),
        "10" => Ok(ReplayStorageLimit::GiB10),
        "25" => Ok(ReplayStorageLimit::GiB25),
        "50" => Ok(ReplayStorageLimit::GiB50),
        "unlimited" => Ok(ReplayStorageLimit::Unlimited),
        _ => Err(GameCatalogError::new(
            "catalog replay storage limit is invalid",
        )),
    }
}

const fn bool_token(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn parse_bool(value: &str) -> Result<bool, GameCatalogError> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(GameCatalogError::new("catalog boolean is invalid")),
    }
}

const fn inheritable_token(value: Inheritable<bool>) -> &'static str {
    match value {
        Inheritable::InheritGlobal => "i",
        Inheritable::Custom(false) => "0",
        Inheritable::Custom(true) => "1",
    }
}

fn parse_inheritable(value: &str) -> Result<Inheritable<bool>, GameCatalogError> {
    match value {
        "i" => Ok(Inheritable::InheritGlobal),
        "0" => Ok(Inheritable::Custom(false)),
        "1" => Ok(Inheritable::Custom(true)),
        _ => Err(GameCatalogError::new(
            "catalog inherited profile value is invalid",
        )),
    }
}

fn serialize_match_rules(match_rules: &[GameMatchRule]) -> String {
    match_rules
        .iter()
        .map(|rule| match rule {
            GameMatchRule::SteamAppId(app_id) => format!("s:{app_id}"),
            GameMatchRule::Executable(path) => {
                format!("p:{}", hex(path.as_os_str().as_bytes()))
            }
            GameMatchRule::ExecutableName(name) => format!("n:{}", hex(name.as_bytes())),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_match_rules(value: &str) -> Result<Vec<GameMatchRule>, GameCatalogError> {
    let encoded = value.split(',').collect::<Vec<_>>();
    if encoded.len() > MAX_MATCH_RULES {
        return Err(GameCatalogError::new(
            "catalog game has too many local match rules",
        ));
    }
    let mut match_rules = Vec::with_capacity(encoded.len());
    for value in encoded {
        let (kind, payload) = value
            .split_once(':')
            .ok_or_else(|| GameCatalogError::new("catalog game match rule is malformed"))?;
        let rule = match kind {
            "s" => GameMatchRule::SteamAppId(
                payload
                    .parse::<u32>()
                    .map_err(|_| GameCatalogError::new("catalog Steam app id is invalid"))?,
            ),
            "p" => GameMatchRule::Executable(PathBuf::from(OsString::from_vec(unhex(payload)?))),
            "n" => {
                GameMatchRule::ExecutableName(String::from_utf8(unhex(payload)?).map_err(|_| {
                    GameCatalogError::new("catalog executable-name alias is not UTF-8")
                })?)
            }
            _ => return Err(GameCatalogError::new("catalog game match rule is unknown")),
        };
        match_rules.push(rule);
    }
    validate_match_rules(&match_rules)?;
    Ok(match_rules)
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

fn unhex(value: &str) -> Result<Vec<u8>, GameCatalogError> {
    if !value.len().is_multiple_of(2) {
        return Err(GameCatalogError::new(
            "catalog hexadecimal value is invalid",
        ));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Ok((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect()
}

fn hex_digit(value: u8) -> Result<u8, GameCatalogError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(GameCatalogError::new(
            "catalog hexadecimal value is invalid",
        )),
    }
}

fn atomic_write(destination: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "catalog has no parent directory",
        )
    })?;
    for _ in 0..32 {
        let id = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{CATALOG_FILE}.{}.{id}.tmp", std::process::id()));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
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
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary game catalog file",
    ))
}

fn read_error(path: &Path, error: &io::Error) -> GameCatalogError {
    GameCatalogError::owned(format!("could not read {}: {error}", path.display()))
}

fn write_error(path: &Path, error: &io::Error) -> GameCatalogError {
    GameCatalogError::owned(format!("could not save {}: {error}", path.display()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameCatalogError {
    message: String,
}

impl GameCatalogError {
    pub(crate) fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }

    pub(crate) fn owned(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for GameCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GameCatalogError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstalled_adaptive_development_state_migrates_to_60_fps() {
        assert_eq!(parse_replay_frame_rate("120"), Ok(ReplayFrameRate::Fps120));
        assert_eq!(
            parse_replay_frame_rate("adaptive"),
            Ok(ReplayFrameRate::Fps60)
        );
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("redunar-game-catalog-{}-{id}", std::process::id()));
            fs::create_dir_all(&root).expect("create fixture");
            let executable = root.join("game-bin");
            fs::write(&executable, b"fixture").expect("write executable");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
                .expect("make executable");
            Self { root, executable }
        }

        fn request(&self, name: &str) -> AddGameRequest {
            AddGameRequest::new(name, &self.executable)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("remove fixture");
        }
    }

    #[test]
    fn missing_catalog_is_empty_and_added_game_round_trips() {
        let fixture = Fixture::new();
        assert_eq!(
            load(&fixture.root).expect("empty catalog"),
            GameCatalog::default()
        );

        let game = add(&fixture.root, fixture.request("Example Game")).expect("add game");
        assert_eq!(game.id.get(), 1);
        assert_eq!(game.launch.executable, fixture.executable);
        assert!(game.profile.inherits_everything());

        let loaded = load(&fixture.root).expect("load catalog");
        assert_eq!(loaded.games, vec![game]);
        let metadata = fs::metadata(fixture.root.join(CATALOG_FILE)).expect("catalog metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(
            fs::read_dir(&fixture.root)
                .expect("read fixture")
                .all(|entry| !entry
                    .expect("fixture entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn bounded_batch_import_is_atomic_and_preserves_direct_identity() {
        let fixture = Fixture::new();
        let second = fixture.root.join("second-game");
        fs::write(&second, b"fixture").expect("write second executable");
        fs::set_permissions(&second, fs::Permissions::from_mode(0o700))
            .expect("make second executable");

        let imported = import(
            &fixture.root,
            vec![
                fixture.request("First Game"),
                AddGameRequest::new("Second Game", &second),
            ],
        )
        .expect("batch import");
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].id.get(), 1);
        assert_eq!(imported[1].id.get(), 2);
        assert!(imported.iter().all(|game| game.match_rules.iter().any(
            |rule| matches!(rule, GameMatchRule::Executable(path) if path == &game.launch.executable)
        )));

        let invalid = fixture.root.join("not-executable");
        fs::write(&invalid, b"fixture").expect("write invalid executable");
        assert!(
            import(
                &fixture.root,
                vec![
                    AddGameRequest::new("Duplicate", &second),
                    AddGameRequest::new("Invalid", invalid),
                ],
            )
            .is_err()
        );
        assert_eq!(load(&fixture.root).expect("catalog").games.len(), 2);
        assert!(import(&fixture.root, Vec::new()).is_err());
    }

    #[test]
    fn discovered_game_import_starts_with_global_feature_inheritance() {
        let fixture = Fixture::new();
        let imported = import(
            &fixture.root,
            vec![AddGameRequest::external_launcher(
                "Discovered Game".into(),
                GameLaunchConfig {
                    executable: fixture.executable.clone(),
                    arguments: vec!["--steam".into()],
                    working_directory: None,
                },
                vec![GameMatchRule::SteamAppId(123_456)],
            )],
        )
        .expect("import discovered game");

        let profile = imported[0].profile;
        assert_eq!(profile.capture_metrics, Inheritable::InheritGlobal);
        assert_eq!(profile.overlay_visible, Inheritable::InheritGlobal);
        assert_eq!(profile.instant_replay, Inheritable::InheritGlobal);
    }

    #[test]
    fn discovery_refresh_updates_only_exact_saved_identity_and_preserves_profile() {
        let fixture = Fixture::new();
        let replacement = fixture.root.join("native-steam");
        fs::write(&replacement, b"fixture").expect("write replacement launcher");
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700))
            .expect("make replacement executable");
        let steam_id = GameMatchRule::SteamAppId(1_808_500);
        let saved = import(
            &fixture.root,
            vec![AddGameRequest::external_launcher(
                "ARC Raiders".to_owned(),
                GameLaunchConfig {
                    executable: fixture.executable.clone(),
                    arguments: vec!["run".into()],
                    working_directory: None,
                },
                vec![steam_id.clone()],
            )],
        )
        .expect("save discovered game")
        .remove(0);
        let profile = PerGameProfile {
            capture_metrics: Inheritable::Custom(false),
            ..PerGameProfile::default()
        };
        update_profile(&fixture.root, saved.id, profile).expect("save profile override");

        let refreshed = refresh_discovered_launches(
            &fixture.root,
            vec![AddGameRequest::external_launcher(
                "Discovery name is not authoritative".to_owned(),
                GameLaunchConfig {
                    executable: replacement.clone(),
                    arguments: vec!["-applaunch".into(), "1808500".into()],
                    working_directory: None,
                },
                vec![steam_id],
            )],
        )
        .expect("refresh exact discovery identity");

        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].id, saved.id);
        assert_eq!(refreshed[0].display_name, "ARC Raiders");
        assert_eq!(refreshed[0].profile, profile);
        assert_eq!(refreshed[0].launch.executable, replacement);
        assert!(
            refresh_discovered_launches(
                &fixture.root,
                vec![AddGameRequest::external_launcher(
                    "Other game".to_owned(),
                    GameLaunchConfig {
                        executable: fixture.executable.clone(),
                        arguments: Vec::new(),
                        working_directory: None,
                    },
                    vec![GameMatchRule::SteamAppId(42)],
                )],
            )
            .expect("ignore unknown identity")
            .is_empty()
        );
    }

    #[test]
    fn shared_typed_launcher_uses_distinct_steam_identity() {
        let fixture = Fixture::new();
        let launcher = fixture.root.join("flatpak");
        fs::write(&launcher, b"fixture").expect("write launcher");
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700))
            .expect("make launcher executable");
        let request = |name: &str, app_id: u32| {
            AddGameRequest::external_launcher(
                name.to_owned(),
                GameLaunchConfig {
                    executable: launcher.clone(),
                    arguments: vec![
                        OsString::from("run"),
                        OsString::from("com.valvesoftware.Steam"),
                        OsString::from("-applaunch"),
                        OsString::from(app_id.to_string()),
                    ],
                    working_directory: None,
                },
                vec![GameMatchRule::SteamAppId(app_id)],
            )
        };

        let games = import(
            &fixture.root,
            vec![request("ARC Raiders", 1_808_500), request("Other", 42)],
        )
        .expect("import shared launcher games");
        assert_eq!(games.len(), 2);
        assert!(games.iter().all(|game| {
            !game
                .match_rules
                .contains(&GameMatchRule::Executable(launcher.clone()))
        }));
        assert_eq!(load(&fixture.root).expect("reload").games, games);
    }

    #[test]
    fn arguments_working_directory_and_profiles_round_trip_without_shell_parsing() {
        let fixture = Fixture::new();
        let mut request = fixture.request("Argument Game");
        let byte_directory = fixture
            .root
            .join(OsString::from_vec(b"working-\xff-directory".to_vec()));
        fs::create_dir(&byte_directory).expect("create byte-preserving directory");
        request.launch.working_directory = Some(byte_directory.clone());
        request.launch.arguments = vec![
            OsString::new(),
            OsString::from("$(false)"),
            OsString::from_vec(b"line one\nline two".to_vec()),
            OsString::from_vec(b"non-utf8-\xff".to_vec()),
        ];
        let game = add(&fixture.root, request).expect("add game");
        let profile = PerGameProfile {
            capture_metrics: Inheritable::Custom(false),
            overlay_visible: Inheritable::Custom(true),
            instant_replay: Inheritable::InheritGlobal,
        };
        update_profile(&fixture.root, game.id, profile).expect("profile update");

        let loaded = load(&fixture.root).expect("load catalog");
        assert_eq!(loaded.games[0].launch.arguments[0], OsStr::new(""));
        assert_eq!(loaded.games[0].launch.arguments[1], OsStr::new("$(false)"));
        assert_eq!(
            loaded.games[0].launch.arguments[2].as_bytes(),
            b"line one\nline two"
        );
        assert_eq!(
            loaded.games[0].launch.arguments[3].as_bytes(),
            b"non-utf8-\xff"
        );
        assert_eq!(
            loaded.games[0].launch.working_directory.as_deref(),
            Some(byte_directory.as_path())
        );
        assert_eq!(loaded.games[0].profile, profile);
    }

    #[test]
    fn match_evidence_round_trips_and_survives_launch_updates() {
        let fixture = Fixture::new();
        let old_alias = PathBuf::from("/games/previous/example-bin");
        let mut request = fixture.request("Identity Game");
        request.match_rules = vec![
            GameMatchRule::SteamAppId(123_456),
            GameMatchRule::Executable(old_alias.clone()),
            GameMatchRule::ExecutableName("stable-alias".into()),
        ];
        let added = add(&fixture.root, request).expect("add game with identity evidence");
        assert!(
            added
                .match_rules
                .contains(&GameMatchRule::SteamAppId(123_456))
        );

        let replacement = fixture.root.join("replacement-bin");
        fs::write(&replacement, b"fixture").expect("write replacement executable");
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700))
            .expect("make replacement executable");
        let updated = update_launch(
            &fixture.root,
            added.id,
            GameLaunchConfig {
                executable: replacement.clone(),
                arguments: Vec::new(),
                working_directory: Some(fixture.root.clone()),
            },
        )
        .expect("update launch");

        for preserved in [
            GameMatchRule::SteamAppId(123_456),
            GameMatchRule::Executable(old_alias),
            GameMatchRule::ExecutableName("stable-alias".into()),
            GameMatchRule::Executable(fixture.executable.clone()),
        ] {
            assert!(updated.match_rules.contains(&preserved));
        }
        assert!(
            updated
                .match_rules
                .contains(&GameMatchRule::Executable(replacement))
        );
        assert_eq!(
            load(&fixture.root).expect("reload match evidence").games[0],
            updated
        );
    }

    #[test]
    fn legacy_nine_field_catalog_is_loaded_with_direct_match_rules() {
        let fixture = Fixture::new();
        let contents = format!(
            "{CATALOG_HEADER_V1}\nnext\t2\nglobal\t1\t0\t0\ngame\t1\t{}\t{}\t-\t-\ti\ti\ti\n",
            hex(b"Legacy Game"),
            hex(fixture.executable.as_os_str().as_bytes())
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write legacy catalog");

        let loaded = load(&fixture.root).expect("load legacy catalog");
        assert_eq!(loaded.games.len(), 1);
        assert!(!loaded.global_profile.overlay_visible);
        assert_eq!(
            loaded.games[0].profile.overlay_visible,
            Inheritable::InheritGlobal
        );
        assert_eq!(
            loaded.games[0].match_rules,
            match_rules_for(&fixture.executable)
        );

        update_profile(&fixture.root, loaded.games[0].id, PerGameProfile::default())
            .expect("migrate legacy catalog on mutation");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
        assert_eq!(
            load(&fixture.root).expect("reload migrated catalog").games[0],
            loaded.games[0]
        );
    }

    #[test]
    fn version_two_catalog_migrates_with_overlay_hidden_and_inherited() {
        let fixture = Fixture::new();
        let direct_rules = match_rules_for(&fixture.executable);
        let contents = format!(
            "{CATALOG_HEADER_V2}\nnext\t2\nglobal\t1\t0\t0\ngame\t1\t{}\t{}\t-\t-\t1\ti\t0\t{}\n",
            hex(b"Version Two Game"),
            hex(fixture.executable.as_os_str().as_bytes()),
            serialize_match_rules(&direct_rules)
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write v2 catalog");

        let loaded = load(&fixture.root).expect("load v2 catalog");
        assert!(!loaded.global_profile.overlay_visible);
        assert_eq!(
            loaded.games[0].profile.overlay_visible,
            Inheritable::InheritGlobal
        );
        assert!(
            loaded.games[0]
                .profile
                .resolve(loaded.global_profile)
                .capture_metrics
        );
        assert!(
            !loaded.games[0]
                .profile
                .resolve(loaded.global_profile)
                .overlay_visible
        );

        update_profile(&fixture.root, loaded.games[0].id, loaded.games[0].profile)
            .expect("migrate v2 catalog on mutation");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
        assert_eq!(load(&fixture.root).expect("reload v3 catalog"), loaded);
    }

    #[test]
    fn version_three_catalog_migrates_with_safe_overlay_customization_defaults() {
        let fixture = Fixture::new();
        let direct_rules = match_rules_for(&fixture.executable);
        let contents = format!(
            "{CATALOG_HEADER_V3}\nnext\t2\nglobal\t1\t1\t0\t0\ngame\t1\t{}\t{}\t-\t-\t1\ti\ti\ti\t{}\n",
            hex(b"Version Three Game"),
            hex(fixture.executable.as_os_str().as_bytes()),
            serialize_match_rules(&direct_rules)
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write v3 catalog");

        let loaded = load(&fixture.root).expect("load v3 catalog");
        assert!(loaded.global_profile.overlay_visible);
        assert_eq!(loaded.global_profile.overlay_preset, OverlayPreset::Compact);
        assert_eq!(loaded.global_profile.overlay_corner, OverlayCorner::TopLeft);
        assert_eq!(loaded.global_profile.overlay_opacity.percent(), 50);

        update_global_profile(&fixture.root, loaded.global_profile).expect("migrate v3 catalog");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
    }

    #[test]
    fn version_four_rejects_unbounded_overlay_opacity() {
        let fixture = Fixture::new();
        let contents =
            format!("{CATALOG_HEADER_V4}\nnext\t1\nglobal\t1\t0\tcompact\ttop-left\t101\t0\t0\n");
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write invalid v4 catalog");
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn version_four_catalog_migrates_with_bounded_default_metrics() {
        let fixture = Fixture::new();
        let contents = format!(
            "{CATALOG_HEADER_V4}\nnext\t1\nglobal\t1\t1\tdetailed\tbottom-right\t65\t0\t0\n"
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write v4 catalog");
        let loaded = load(&fixture.root).expect("load v4 catalog");
        assert_eq!(
            loaded.global_profile.overlay_preset,
            OverlayPreset::Detailed
        );
        assert_eq!(
            loaded.global_profile.overlay_metrics,
            OverlayMetricSet::COMPACT
        );
        assert_eq!(
            loaded.global_profile.overlay_corner,
            OverlayCorner::BottomRight
        );
        assert_eq!(loaded.global_profile.overlay_opacity.percent(), 65);

        update_global_profile(&fixture.root, loaded.global_profile).expect("migrate v4 catalog");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
    }

    #[test]
    fn version_five_rejects_empty_or_unknown_custom_metric_sets() {
        let fixture = Fixture::new();
        for bits in ["0", "65535"] {
            let contents = format!(
                "{CATALOG_HEADER_V5}\nnext\t1\nglobal\t1\t1\tcustom\t{bits}\ttop-left\t50\t0\t0\n"
            );
            fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write invalid v5 catalog");
            assert!(load(&fixture.root).is_err());
        }
    }

    #[test]
    fn version_five_catalog_migrates_with_bounded_replay_defaults() {
        let fixture = Fixture::new();
        let contents = format!(
            "{CATALOG_HEADER_V5}\nnext\t1\nglobal\t1\t1\tcompact\t51\ttop-right\t50\t0\t0\n"
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write v5 catalog");
        let loaded = load(&fixture.root).expect("load v5 catalog");
        assert_eq!(loaded.global_profile.replay, ReplaySettings::default());

        update_global_profile(&fixture.root, loaded.global_profile).expect("migrate v5 catalog");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
    }

    #[test]
    fn version_six_rejects_unknown_replay_settings() {
        let fixture = Fixture::new();
        for replay_fields in [
            "31\t60\tbalanced\t10",
            "30\t59\tbalanced\t10",
            "30\t60\textreme\t10",
            "30\t60\tbalanced\t11",
        ] {
            let contents = format!(
                "{CATALOG_HEADER_V6}\nnext\t1\nglobal\t1\t1\tcompact\t51\ttop-left\t50\t{replay_fields}\t0\t0\n"
            );
            fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write invalid v6 catalog");
            assert!(load(&fixture.root).is_err());
        }
    }

    #[test]
    fn version_nine_discards_removed_profile_fields_and_writes_current_version() {
        let fixture = Fixture::new();
        let direct_rules = match_rules_for(&fixture.executable);
        let contents = format!(
            "{CATALOG_HEADER_V9}\nnext\t2\nglobal\t1\t1\tdetailed\t255\tbottom-right\t65\t125\t1\t1\t120\t60\thigh\t25\t1\t1\ngame\t1\t{}\t{}\t-\t-\t1\t0\t1\t1\t{}\n",
            hex(b"Version Nine Game"),
            hex(fixture.executable.as_os_str().as_bytes()),
            serialize_match_rules(&direct_rules)
        );
        fs::write(fixture.root.join(CATALOG_FILE), contents).expect("write v9 catalog");

        let loaded = load(&fixture.root).expect("load v9 catalog");
        assert!(loaded.global_profile.capture_metrics);
        assert!(loaded.global_profile.instant_replay);
        assert_eq!(
            loaded.games[0].profile.capture_metrics,
            Inheritable::Custom(true)
        );
        assert_eq!(
            loaded.games[0].profile.overlay_visible,
            Inheritable::Custom(false)
        );
        assert_eq!(
            loaded.games[0].profile.instant_replay,
            Inheritable::Custom(true)
        );

        update_profile(&fixture.root, loaded.games[0].id, loaded.games[0].profile)
            .expect("migrate v9 catalog");
        let migrated =
            fs::read_to_string(fixture.root.join(CATALOG_FILE)).expect("read migrated catalog");
        assert_eq!(migrated.lines().next(), Some(CATALOG_HEADER_V12));
        assert_eq!(
            load(&fixture.root).expect("reload migrated catalog"),
            loaded
        );
    }

    #[test]
    fn concurrent_additions_are_serialized_without_losing_stable_ids() {
        const WRITERS: usize = 16;
        let fixture = Fixture::new();
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut writers = Vec::new();
        for index in 0..WRITERS {
            let root = fixture.root.clone();
            let executable = root.join(format!("concurrent-game-{index}"));
            fs::write(&executable, b"fixture").expect("write concurrent executable");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
                .expect("make concurrent executable");
            let barrier = Arc::clone(&barrier);
            writers.push(thread::spawn(move || {
                barrier.wait();
                add(
                    &root,
                    AddGameRequest::new(format!("Game {index}"), executable),
                )
                .expect("concurrent add")
                .id
                .get()
            }));
        }

        let mut returned_ids = writers
            .into_iter()
            .map(|writer| writer.join().expect("join catalog writer"))
            .collect::<Vec<_>>();
        returned_ids.sort_unstable();
        assert_eq!(returned_ids, (1..=WRITERS as u64).collect::<Vec<_>>());

        let catalog = load(&fixture.root).expect("load concurrent catalog");
        assert_eq!(catalog.games.len(), WRITERS);
        let mut stored_ids = catalog
            .games
            .iter()
            .map(|game| game.id.get())
            .collect::<Vec<_>>();
        stored_ids.sort_unstable();
        assert_eq!(stored_ids, returned_ids);
    }

    #[test]
    fn invalid_or_unbounded_match_evidence_is_rejected() {
        let fixture = Fixture::new();
        let mut zero_steam = fixture.request("Zero Steam");
        zero_steam.match_rules = vec![GameMatchRule::SteamAppId(0)];
        assert!(add(&fixture.root, zero_steam).is_err());

        let mut too_many = fixture.request("Too Many Rules");
        too_many.match_rules = (0..MAX_MATCH_RULES)
            .map(|index| {
                GameMatchRule::SteamAppId(u32::try_from(index).expect("small rule limit") + 1)
            })
            .collect();
        assert!(add(&fixture.root, too_many).is_err());
    }

    #[test]
    fn duplicate_executable_and_malformed_state_are_rejected() {
        let fixture = Fixture::new();
        add(&fixture.root, fixture.request("First")).expect("first game");
        assert!(add(&fixture.root, fixture.request("Second")).is_err());

        fs::write(fixture.root.join(CATALOG_FILE), b"redunar-games-v9\n")
            .expect("write malformed catalog");
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn removal_preserves_monotonic_local_ids() {
        let fixture = Fixture::new();
        let first = add(&fixture.root, fixture.request("First")).expect("first game");
        remove(&fixture.root, first.id).expect("remove game");
        let second_path = fixture.root.join("second-bin");
        fs::write(&second_path, b"fixture").expect("write second executable");
        fs::set_permissions(&second_path, fs::Permissions::from_mode(0o700))
            .expect("make second executable");
        let second =
            add(&fixture.root, AddGameRequest::new("Second", second_path)).expect("second game");
        assert_eq!(second.id.get(), 2);
    }

    #[test]
    fn missing_installed_file_does_not_erase_the_local_record() {
        let fixture = Fixture::new();
        let game = add(&fixture.root, fixture.request("Portable Game")).expect("add game");
        fs::remove_file(&fixture.executable).expect("remove executable");

        let loaded = load(&fixture.root).expect("load catalog after uninstall");
        assert_eq!(loaded.games, vec![game]);
    }

    #[test]
    fn launch_and_global_profile_updates_are_persisted() {
        let fixture = Fixture::new();
        let game = add(&fixture.root, fixture.request("Updated Game")).expect("add game");
        let launch = GameLaunchConfig {
            executable: fixture.executable.clone(),
            arguments: vec![OsString::from("--safe"), OsString::from("two words")],
            working_directory: Some(fixture.root.clone()),
        };
        update_launch(&fixture.root, game.id, launch.clone()).expect("update launch");
        let global = GlobalGameProfile {
            capture_metrics: false,
            overlay_visible: true,
            overlay_preset: OverlayPreset::FpsOnly,
            overlay_layout: OverlayLayout::Ribbon,
            overlay_palette: OverlayPalette::Glacier,
            overlay_branding: false,
            overlay_metrics: OverlayMetricSet::DETAILED,
            overlay_corner: OverlayCorner::BottomRight,
            overlay_opacity: OverlayOpacity::new(65).expect("opacity"),
            overlay_scale: OverlayScale::default(),
            gamescope_enabled: false,
            gamemode_enabled: false,
            replay: ReplaySettings {
                duration: ReplayDuration::Seconds120,
                frame_rate: ReplayFrameRate::Fps120,
                quality: ReplayQuality::High,
                storage_limit: ReplayStorageLimit::GiB25,
            },
            instant_replay: false,
        };
        update_global_profile(&fixture.root, global).expect("update global profile");

        let loaded = load(&fixture.root).expect("load updated catalog");
        assert_eq!(loaded.global_profile, global);
        assert_eq!(loaded.games[0].launch, launch);
        assert_eq!(
            effective_profile(&fixture.root, game.id).expect("effective profile"),
            loaded.games[0].profile.resolve(global)
        );
    }

    #[test]
    fn only_exact_process_identity_resolves_an_effective_profile() {
        let fixture = Fixture::new();
        let game = add(&fixture.root, fixture.request("Resolved Game")).expect("add game");
        let exact = GameProcess {
            pid: 42,
            comm: "game-bin".into(),
            executable: fixture.executable.clone(),
            steam_app_id: None,
            game_mode_active: false,
        };
        let resolved = resolve_process(&fixture.root, &exact).expect("resolve exact process");
        assert_eq!(resolved.resolution, GameResolution::Automatic(game.id));
        assert_eq!(
            resolved.effective_profile,
            Some(game.profile.resolve(GlobalGameProfile::default()))
        );

        let suggested = GameProcess {
            executable: fixture.root.join("another/game-bin"),
            ..exact
        };
        let resolved = resolve_process(&fixture.root, &suggested).expect("resolve suggestion");
        assert_eq!(resolved.resolution, GameResolution::Suggested(game.id));
        assert_eq!(resolved.effective_profile, None);
    }

    #[test]
    fn non_executable_and_oversized_launch_inputs_are_rejected() {
        let fixture = Fixture::new();
        fs::set_permissions(&fixture.executable, fs::Permissions::from_mode(0o600))
            .expect("remove executable bit");
        assert!(add(&fixture.root, fixture.request("Not Executable")).is_err());

        fs::set_permissions(&fixture.executable, fs::Permissions::from_mode(0o700))
            .expect("restore executable bit");
        let mut request = fixture.request("Too Many Arguments");
        request.launch.arguments = vec![OsString::from("argument"); MAX_ARGUMENTS + 1];
        assert!(add(&fixture.root, request).is_err());
    }
}
