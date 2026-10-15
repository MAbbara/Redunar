//! Native release metadata verification for the in-app updater.
//!
//! Development builds stay source-unconfigured unless a test endpoint is
//! supplied. Release builds embed the public GitHub release channel, while the
//! native process owns downloads and signature verification in every case.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE_ENV: &str = "REDUNAR_UPDATE_SOURCE_URL";
const ALLOW_INSECURE_ENV: &str = "REDUNAR_ALLOW_INSECURE_URL";
const RELEASE_SOURCE: Option<&str> = option_env!("REDUNAR_UPDATE_SOURCE_URL");
const PUBLIC_KEY_PEM: &str = include_str!("../../../../packaging/release-signing-public.pem");
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const UPDATE_CACHE_FOLDER: &str = "redunar/updates";
const PENDING_UPDATE_FILE: &str = "pending-update.txt";
// Startup and manual checks may overlap. Serialize cache replacement and
// installation so an old check cannot overwrite a newer verified package.
static UPDATE_OPERATION: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckStatus {
    pub current_version: String,
    pub state: String,
    pub message: String,
    pub latest_version: Option<String>,
    pub package_kind: Option<String>,
    pub asset_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInstallStatus {
    pub state: String,
    pub message: String,
}

pub fn check_for_updates(refresh_pending: bool) -> UpdateCheckStatus {
    let _operation = UPDATE_OPERATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current_version = env!("CARGO_PKG_VERSION").to_owned();
    let retained = reconcile_pending_update(&current_version);
    if let Some(status) = retained
        .as_ref()
        .filter(|status| hold_pending(status, refresh_pending))
    {
        return status.clone();
    }
    let Some(source) = env::var(SOURCE_ENV)
        .ok()
        .or_else(|| RELEASE_SOURCE.map(str::to_owned))
    else {
        return retained.unwrap_or_else(|| {
            unavailable(
                current_version,
                "Signed update checking is not configured in this build yet.",
            )
        });
    };
    let source = source.trim_end_matches('/').to_owned();
    if source.is_empty() {
        return retained.unwrap_or_else(|| {
            unavailable(
                current_version,
                "Signed update checking is not configured in this build yet.",
            )
        });
    }
    if !is_allowed_source(&source) {
        return retained_or_error(
            retained,
            error_status(current_version, "Update sources must use HTTPS."),
        );
    }

    let temporary_directory = match temporary_directory() {
        Ok(path) => path,
        Err(error) => {
            return retained_or_error(
                retained,
                error_status(
                    current_version,
                    &format!("Could not prepare update verification: {error}"),
                ),
            )
        }
    };
    let result = check_source(
        &source,
        &current_version,
        &temporary_directory,
        retained.as_ref(),
    );
    let _ = fs::remove_dir_all(&temporary_directory);
    if result.state == "error" {
        retained_or_error(retained, result)
    } else {
        result
    }
}

fn retained_or_error(
    retained: Option<UpdateCheckStatus>,
    failure: UpdateCheckStatus,
) -> UpdateCheckStatus {
    retained.map_or(failure, |mut status| {
        status.message.push_str(
            " A newer release could not be checked; the retained verified package remains available.",
        );
        status
    })
}

fn hold_pending(status: &UpdateCheckStatus, refresh_pending: bool) -> bool {
    status.state == "restart-needed" || (status.state == "handoff" && !refresh_pending)
}

fn retain_if_latest_not_newer(
    retained: Option<&UpdateCheckStatus>,
    release_version: &str,
) -> Option<UpdateCheckStatus> {
    retained
        .filter(|status| {
            status
                .latest_version
                .as_deref()
                .is_some_and(|cached| !is_newer_version(release_version, cached))
        })
        .cloned()
}

fn check_source(
    source: &str,
    current_version: &str,
    directory: &Path,
    retained: Option<&UpdateCheckStatus>,
) -> UpdateCheckStatus {
    let allow_insecure = env::var(ALLOW_INSECURE_ENV).ok().as_deref() == Some("1");
    check_source_with(
        source,
        current_version,
        directory,
        retained,
        PUBLIC_KEY_PEM,
        allow_insecure,
        |package_path, package, version, digest| {
            persist_verified_release(
                package_path,
                &directory.join("VERSION"),
                &directory.join("SHA256SUMS"),
                &directory.join("SHA256SUMS.sig"),
                package,
                version,
                digest,
            )
        },
    )
}

fn check_source_with<Persist>(
    source: &str,
    current_version: &str,
    directory: &Path,
    retained: Option<&UpdateCheckStatus>,
    public_key_pem: &str,
    allow_insecure: bool,
    persist: Persist,
) -> UpdateCheckStatus
where
    Persist: FnOnce(&Path, PackageTarget, &str, &str) -> Result<(), String>,
{
    let version_path = directory.join("VERSION");
    let manifest_path = directory.join("SHA256SUMS");
    let signature_path = directory.join("SHA256SUMS.sig");
    for (name, path) in [
        ("VERSION", &version_path),
        ("SHA256SUMS", &manifest_path),
        ("SHA256SUMS.sig", &signature_path),
    ] {
        if let Err(error) = download(&format!("{source}/{name}"), path, allow_insecure) {
            return error_status(
                current_version.to_owned(),
                &format!("Could not download signed update metadata: {error}"),
            );
        }
    }
    let manifest = match fs::read_to_string(&manifest_path) {
        Ok(value) => value,
        Err(error) => {
            return error_status(
                current_version.to_owned(),
                &format!("Could not read update manifest: {error}"),
            )
        }
    };
    if let Err(error) =
        verify_signature_with_key(&manifest_path, &signature_path, directory, public_key_pem)
    {
        return error_status(
            current_version.to_owned(),
            &format!("The update manifest signature is invalid: {error}"),
        );
    }
    let manifest_entries = match parse_manifest(&manifest) {
        Ok(entries) => entries,
        Err(error) => {
            return error_status(
                current_version.to_owned(),
                &format!("The update manifest is invalid: {error}"),
            )
        }
    };
    let latest_version = match verified_release_version(&version_path, &manifest_entries) {
        Ok(version) => version,
        Err(error) => {
            return error_status(
                current_version.to_owned(),
                &format!("The signed release version is invalid: {error}"),
            )
        }
    };
    if let Some(status) = retain_if_latest_not_newer(retained, &latest_version) {
        return status;
    }
    if !is_newer_version(&latest_version, current_version) {
        return UpdateCheckStatus {
            current_version: current_version.to_owned(),
            state: "up-to-date".into(),
            message: format!("Redunar {current_version} is up to date."),
            latest_version: Some(latest_version),
            package_kind: None,
            asset_name: None,
        };
    }
    let package = match package_target() {
        Ok(package) => package,
        Err(error) => {
            return error_status(
                current_version.to_owned(),
                &format!("This system cannot select a Redunar update package: {error}"),
            )
        }
    };
    let Some(expected_digest) = manifest_entries.get(package.asset_name) else {
        return error_status(
            current_version.to_owned(),
            &format!(
                "The signed release does not include {}.",
                package.asset_name
            ),
        );
    };
    let package_path = directory.join(package.asset_name);
    if let Err(error) = download_with_limit(
        &format!("{source}/{}", package.asset_name),
        &package_path,
        MAX_PACKAGE_BYTES,
        allow_insecure,
    ) {
        return error_status(
            current_version.to_owned(),
            &format!("Could not download the Redunar update package: {error}"),
        );
    }
    if let Err(error) = verify_checksum(&package_path, expected_digest) {
        return error_status(
            current_version.to_owned(),
            &format!("The downloaded update package failed checksum verification: {error}"),
        );
    }
    if let Err(error) = persist(&package_path, package, &latest_version, expected_digest) {
        return error_status(
            current_version.to_owned(),
            &format!("The verified update package could not be retained: {error}"),
        );
    }
    UpdateCheckStatus {
        current_version: current_version.to_owned(),
        state: "available".into(),
        message: format!("Redunar {latest_version} has a verified update package."),
        latest_version: Some(latest_version),
        package_kind: Some(package.kind.into()),
        asset_name: Some(package.asset_name.into()),
    }
}

pub fn install_update() -> UpdateInstallStatus {
    let _operation = UPDATE_OPERATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let pending = match read_pending_update() {
        Ok(update) => update,
        Err(error) => return install_error(error),
    };
    let target = match package_target() {
        Ok(target) => target,
        Err(error) => {
            return install_error(format!("This system cannot install the update: {error}"))
        }
    };
    let directory = match update_cache_directory() {
        Ok(directory) => directory,
        Err(error) => return install_error(error),
    };
    let installed_version = installed_package_version(&pending.kind);
    let pending_kind = pending.kind.clone();
    let helper = Path::new("/usr/libexec/redunar-update-helper");
    install_verified_update(
        &directory,
        pending,
        target,
        env!("CARGO_PKG_VERSION"),
        installed_version.as_deref(),
        helper.is_file().then_some(helper),
        |helper, directory| {
            let pkexec = ["/usr/bin/pkexec", "/bin/pkexec"]
                .iter()
                .map(Path::new)
                .find(|path| path.is_file())
                .ok_or("polkit (pkexec) is not installed".to_owned())?;
            let status = Command::new(pkexec)
                .arg("--disable-internal-agent")
                .arg(helper)
                .arg(directory)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("could not request system authorization: {error}"))?;
            match status.code() {
                Some(0) => Ok(installed_package_version(&pending_kind)),
                Some(126) => Err("Authorization was cancelled. The update is still ready.".into()),
                Some(127) => Err("System authorization is unavailable. Check that a graphical polkit agent is running.".into()),
                _ => Err("The update could not be installed. The verified package is still ready; check your system package manager and retry.".into()),
            }
        },
    )
}

fn install_verified_update(
    directory: &Path,
    pending: PendingUpdate,
    target: PackageTarget,
    current_version: &str,
    installed_version: Option<&str>,
    helper: Option<&Path>,
    launch: impl FnOnce(&Path, &Path) -> Result<Option<String>, String>,
) -> UpdateInstallStatus {
    if !is_newer_version(&pending.version, current_version) {
        return install_error("The retained update is not newer than this build.".into());
    }
    if pending.asset_name != target.asset_name || pending.kind != target.kind {
        return install_error("The downloaded package does not match this system.".into());
    }
    let path = pending_package_path(directory, &pending);
    if let Err(error) = verify_checksum(&path, &pending.digest) {
        return install_error(format!(
            "The retained update package failed verification: {error}"
        ));
    }
    if package_version_at_least(&pending, installed_version) {
        return UpdateInstallStatus {
            state: "restart-needed".into(),
            message: "The new Redunar package is installed. Fully quit Redunar, including its tray process, then reopen it to load the update.".into(),
        };
    }
    if !versioned_package_path(directory, &pending.asset_name, &pending.digest).is_file()
        || ["VERSION", "SHA256SUMS", "SHA256SUMS.sig"]
            .iter()
            .any(|name| !signed_metadata_path(directory, name, &pending.digest).is_file())
    {
        return install_retry("This cached update predates direct installation. Check for updates again to refresh the signed package.".into());
    }
    let Some(helper) = helper else {
        return install_retry("The update helper is not installed. Update Redunar through your package manager once to enable direct updates.".into());
    };
    if let Err(error) = write_pending_update_in_directory(
        directory,
        &PendingUpdate {
            state: "handoff".into(),
            ..pending.clone()
        },
    ) {
        return install_error(format!("Could not record the update attempt: {error}"));
    }
    let installed_after = match launch(helper, directory) {
        Ok(version) => version,
        Err(error) => {
            if let Err(restore_error) = write_pending_update_in_directory(
                directory,
                &PendingUpdate {
                    state: "downloaded".into(),
                    ..pending
                },
            ) {
                return install_error(format!(
                    "{error} Could not restore the cached update state: {restore_error}"
                ));
            }
            return install_retry(error);
        }
    };
    if package_version_at_least(&pending, installed_after.as_deref()) {
        return UpdateInstallStatus {
            state: "restart-needed".into(),
            message: "The new Redunar package is installed. Fully quit Redunar, including its tray process, then reopen it to load the update.".into(),
        };
    }
    UpdateInstallStatus {
        state: "handoff".into(),
        message: "The installer finished, but Redunar could not confirm the new package version. Check again or retry installation.".into(),
    }
}

fn install_retry(message: String) -> UpdateInstallStatus {
    UpdateInstallStatus {
        state: "available".into(),
        message,
    }
}

fn install_error(message: String) -> UpdateInstallStatus {
    UpdateInstallStatus {
        state: "error".into(),
        message,
    }
}

fn unavailable(current_version: String, message: &str) -> UpdateCheckStatus {
    UpdateCheckStatus {
        current_version,
        state: "not-configured".into(),
        message: message.into(),
        latest_version: None,
        package_kind: None,
        asset_name: None,
    }
}

fn error_status(current_version: impl Into<String>, message: &str) -> UpdateCheckStatus {
    UpdateCheckStatus {
        current_version: current_version.into(),
        state: "error".into(),
        message: message.into(),
        latest_version: None,
        package_kind: None,
        asset_name: None,
    }
}

fn is_allowed_source(source: &str) -> bool {
    source.starts_with("https://") || env::var(ALLOW_INSECURE_ENV).ok().as_deref() == Some("1")
}

fn temporary_directory() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = env::temp_dir().join(format!("redunar-update-{}-{nanos}", std::process::id()));
    fs::create_dir(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingUpdate {
    state: String,
    version: String,
    kind: String,
    asset_name: String,
    digest: String,
}

fn update_cache_directory() -> Result<PathBuf, String> {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or("could not determine a user cache directory")?;
    let directory = base.join(UPDATE_CACHE_FOLDER);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    fs::set_permissions(
        &directory,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|error| error.to_string())?;
    Ok(directory)
}

fn persist_verified_release(
    source: &Path,
    version_source: &Path,
    manifest_source: &Path,
    signature_source: &Path,
    package: PackageTarget,
    version: &str,
    digest: &str,
) -> Result<(), String> {
    let directory = update_cache_directory()?;
    for (name, source) in [
        ("VERSION", version_source),
        ("SHA256SUMS", manifest_source),
        ("SHA256SUMS.sig", signature_source),
    ] {
        let destination = signed_metadata_path(&directory, name, digest);
        let temporary = directory.join(format!(".{name}.{digest}.download"));
        fs::copy(source, &temporary).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &destination).map_err(|error| error.to_string())?;
    }
    persist_verified_package_in_directory(&directory, source, package, version, digest)
}

fn signed_metadata_path(directory: &Path, name: &str, digest: &str) -> PathBuf {
    directory.join(format!("{name}.{digest}"))
}

fn persist_verified_package_in_directory(
    directory: &Path,
    source: &Path,
    package: PackageTarget,
    version: &str,
    digest: &str,
) -> Result<(), String> {
    let previous_update = read_pending_update_in_directory(directory).ok();
    let previous = previous_update
        .as_ref()
        .map(|pending| pending_package_path(directory, pending));
    let destination = versioned_package_path(directory, package.asset_name, digest);
    let temporary = directory.join(format!(".{}.{}.download", package.asset_name, digest));
    let destination_preexisted = destination.exists();
    fs::copy(source, &temporary).map_err(|error| error.to_string())?;
    if let Err(error) = verify_checksum(&temporary, digest) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    let update = PendingUpdate {
        state: "downloaded".into(),
        version: version.to_owned(),
        kind: package.kind.to_owned(),
        asset_name: package.asset_name.to_owned(),
        digest: digest.to_owned(),
    };
    if let Err(error) = write_pending_update_in_directory(directory, &update) {
        if !destination_preexisted {
            let _ = fs::remove_file(&destination);
        }
        return Err(error);
    }
    if let Some(previous) = previous.filter(|path| path != &destination) {
        let _ = fs::remove_file(previous);
        if let Some(old_digest) = previous_update
            .map(|pending| pending.digest)
            .filter(|old| old != digest)
        {
            remove_signed_metadata(directory, &old_digest);
        }
    }
    Ok(())
}

fn versioned_package_path(directory: &Path, asset_name: &str, digest: &str) -> PathBuf {
    directory.join(format!("{asset_name}.{digest}"))
}

fn remove_signed_metadata(directory: &Path, digest: &str) {
    for name in ["VERSION", "SHA256SUMS", "SHA256SUMS.sig"] {
        let _ = fs::remove_file(signed_metadata_path(directory, name, digest));
    }
}

// Old pending metadata pointed at the unsuffixed asset. Preserve that reader
// until the pending update is replaced or consumed.
fn pending_package_path(directory: &Path, pending: &PendingUpdate) -> PathBuf {
    let versioned = versioned_package_path(directory, &pending.asset_name, &pending.digest);
    if versioned.is_file() {
        versioned
    } else {
        directory.join(&pending.asset_name)
    }
}

fn write_pending_update_in_directory(
    directory: &Path,
    update: &PendingUpdate,
) -> Result<(), String> {
    let metadata = format!(
        "state={}\nversion={}\nkind={}\nasset_name={}\ndigest={}\n",
        update.state, update.version, update.kind, update.asset_name, update.digest
    );
    let metadata_temporary = directory.join(".pending-update.txt.tmp");
    fs::write(&metadata_temporary, metadata).map_err(|error| error.to_string())?;
    fs::rename(metadata_temporary, directory.join(PENDING_UPDATE_FILE))
        .map_err(|error| error.to_string())
}

fn read_pending_update() -> Result<PendingUpdate, String> {
    let directory = update_cache_directory()?;
    read_pending_update_in_directory(&directory)
}

fn read_pending_update_in_directory(directory: &Path) -> Result<PendingUpdate, String> {
    let metadata = fs::read_to_string(directory.join(PENDING_UPDATE_FILE))
        .map_err(|error| format!("no verified update is ready: {error}"))?;
    parse_pending_update(&metadata)
}

fn parse_pending_update(metadata: &str) -> Result<PendingUpdate, String> {
    let mut values = BTreeMap::new();
    for line in metadata.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or("retained update metadata is malformed")?;
        if values.insert(key, value).is_some() {
            return Err("retained update metadata contains a duplicate field".into());
        }
    }
    let state = values
        .get("state")
        .copied()
        .ok_or("retained update metadata has no state")?;
    let version = values
        .get("version")
        .copied()
        .ok_or("retained update metadata has no version")?;
    let kind = values
        .get("kind")
        .copied()
        .ok_or("retained update metadata has no package kind")?;
    let asset_name = values
        .get("asset_name")
        .copied()
        .ok_or("retained update metadata has no package name")?;
    let digest = values
        .get("digest")
        .copied()
        .ok_or("retained update metadata has no checksum")?;
    if !matches!(state, "downloaded" | "handoff")
        || !valid_version(version)
        || !matches!(kind, "deb" | "rpm" | "arch")
        || asset_name.contains('/')
        || asset_name.contains('\\')
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("retained update metadata is invalid".into());
    }
    Ok(PendingUpdate {
        state: state.to_owned(),
        version: version.to_owned(),
        kind: kind.to_owned(),
        asset_name: asset_name.to_owned(),
        digest: digest.to_ascii_lowercase(),
    })
}

fn reconcile_pending_update(current_version: &str) -> Option<UpdateCheckStatus> {
    let pending = read_pending_update().ok()?;
    let directory = update_cache_directory().ok()?;
    let package_path = pending_package_path(&directory, &pending);
    if !is_newer_version(&pending.version, current_version) {
        let _ = fs::remove_file(package_path);
        remove_signed_metadata(&directory, &pending.digest);
        let _ = fs::remove_file(directory.join(PENDING_UPDATE_FILE));
        return Some(UpdateCheckStatus {
            current_version: current_version.to_owned(),
            state: "up-to-date".into(),
            message: format!("Redunar {current_version} is up to date."),
            latest_version: Some(current_version.to_owned()),
            package_kind: None,
            asset_name: None,
        });
    }
    if verify_checksum(&package_path, &pending.digest).is_err() {
        let _ = fs::remove_file(package_path);
        remove_signed_metadata(&directory, &pending.digest);
        let _ = fs::remove_file(directory.join(PENDING_UPDATE_FILE));
        return None;
    }
    let installed = pending.state == "handoff" && installed_version_at_least(&pending);
    let message = if installed {
        format!(
            "Redunar {} is installed. Fully quit Redunar, including its tray process, then reopen it to load the update.",
            pending.version
        )
    } else if pending.state == "handoff" {
        format!(
            "Redunar {} is not confirmed installed. Retry installation or check again.",
            pending.version
        )
    } else {
        format!("Redunar {} has a verified update package.", pending.version)
    };
    Some(UpdateCheckStatus {
        current_version: current_version.to_owned(),
        state: if installed {
            "restart-needed"
        } else if pending.state == "handoff" {
            "handoff"
        } else {
            "available"
        }
        .into(),
        message,
        latest_version: Some(pending.version),
        package_kind: Some(pending.kind),
        asset_name: Some(pending.asset_name),
    })
}

/// Local confirmation only; restarting never triggers a download or installer.
pub(crate) fn ensure_restart_ready() -> Result<(), String> {
    let _operation = UPDATE_OPERATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let status = reconcile_pending_update(env!("CARGO_PKG_VERSION"));
    if status.is_some_and(|value| value.state == "restart-needed") {
        Ok(())
    } else {
        Err(
            "A newer installed Redunar package has not been confirmed. Check for updates again."
                .into(),
        )
    }
}

fn installed_version_at_least(pending: &PendingUpdate) -> bool {
    package_version_at_least(pending, installed_package_version(&pending.kind).as_deref())
}

fn package_version_at_least(pending: &PendingUpdate, installed: Option<&str>) -> bool {
    installed.is_some_and(|version| !is_newer_version(&pending.version, version))
}

/// Read only the system package database. An installer exit is not proof of
/// success, and an already-installed update must not be installed again.
fn installed_package_version(kind: &str) -> Option<String> {
    let (program, arguments): (&str, &[&str]) = match kind {
        "rpm" => ("rpm", &["-q", "--qf", "%{VERSION}", "redunar-app"]),
        "deb" => (
            "dpkg-query",
            &["-W", "-f=${Status}\t${Version}", "redunar-app"],
        ),
        "arch" => ("pacman", &["-Q", "redunar-app"]),
        _ => return None,
    };
    let executable = [format!("/usr/bin/{program}"), format!("/bin/{program}")]
        .into_iter()
        .find(|path| Path::new(path).is_file())?;
    let output = Command::new(executable).args(arguments).output().ok()?;
    if !output.status.success() || output.stdout.len() > 128 {
        return None;
    }
    let result = std::str::from_utf8(&output.stdout).ok()?;
    let version = if kind == "deb" {
        let (status, version) = result.split_once('\t')?;
        (status == "install ok installed").then_some(version)?
    } else {
        result
    };
    parse_installed_version(kind, version)
}

fn parse_installed_version(kind: &str, output: &str) -> Option<String> {
    let value = output.trim();
    let value = match kind {
        "rpm" => value,
        "deb" => value.rsplit(':').next()?.split('-').next()?,
        "arch" => value.strip_prefix("redunar-app ")?.split('-').next()?,
        _ => return None,
    };
    valid_version(value).then(|| value.to_owned())
}

fn download(url: &str, destination: &Path, allow_insecure: bool) -> Result<(), String> {
    download_with_limit(url, destination, MAX_METADATA_BYTES, allow_insecure)
}

fn download_with_limit(
    url: &str,
    destination: &Path,
    maximum_bytes: u64,
    allow_insecure: bool,
) -> Result<(), String> {
    let curl = ["/usr/bin/curl", "/bin/curl"]
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .ok_or("curl is not installed")?;
    let output = destination
        .to_str()
        .ok_or("temporary update path is not valid UTF-8")?;
    let mut command = Command::new(curl);
    command.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "20",
        "--max-filesize",
        &maximum_bytes.to_string(),
        "--output",
        output,
    ]);
    if !allow_insecure {
        command.args(["--proto", "=https", "--tlsv1.2"]);
    }
    let status = command
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("curl exited with {status}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackageTarget {
    kind: &'static str,
    asset_name: &'static str,
}

fn package_target() -> Result<PackageTarget, String> {
    let architecture =
        env::var("REDUNAR_ARCHITECTURE").unwrap_or_else(|_| std::env::consts::ARCH.into());
    if !matches!(architecture.as_str(), "x86_64" | "amd64") {
        return Err(format!("unsupported architecture: {architecture}"));
    }
    let path = env::var_os("REDUNAR_OS_RELEASE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/os-release"));
    let content = fs::read_to_string(path)
        .map_err(|error| format!("could not read distribution information: {error}"))?;
    let values = content
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key, value.trim_matches('"').trim_matches('\'').to_owned()))
        .collect::<BTreeMap<_, _>>();
    let id = values.get("ID").map(String::as_str).unwrap_or_default();
    let like = values
        .get("ID_LIKE")
        .map(String::as_str)
        .unwrap_or_default();
    select_package(
        &architecture,
        id,
        like,
        Path::new("/run/ostree-booted").exists(),
    )
}

fn select_package(
    architecture: &str,
    id: &str,
    like: &str,
    immutable_or_musl: bool,
) -> Result<PackageTarget, String> {
    if !matches!(architecture, "x86_64" | "amd64") {
        return Err(format!("unsupported architecture: {architecture}"));
    }
    if matches!(id, "alpine" | "nixos" | "steamos") || immutable_or_musl {
        return Err(format!(
            "immutable or musl distribution is not supported ({id})"
        ));
    }
    let words = format!(" {id} {like} ");
    if words
        .split_whitespace()
        .any(|word| matches!(word, "ubuntu" | "debian" | "linuxmint" | "pop"))
    {
        return Ok(PackageTarget {
            kind: "deb",
            asset_name: "redunar-app-linux-amd64.deb",
        });
    }
    if words.split_whitespace().any(|word| {
        matches!(
            word,
            "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "nobara"
        )
    }) {
        return Ok(PackageTarget {
            kind: "rpm",
            asset_name: "redunar-app-linux-x86_64.rpm",
        });
    }
    if words
        .split_whitespace()
        .any(|word| matches!(word, "arch" | "manjaro" | "endeavouros" | "cachyos"))
    {
        return Ok(PackageTarget {
            kind: "arch",
            asset_name: "redunar-app-linux-x86_64.pkg.tar.zst",
        });
    }
    if words.split_whitespace().any(|word| {
        matches!(
            word,
            "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "sles" | "suse"
        )
    }) {
        return Ok(PackageTarget {
            kind: "rpm",
            asset_name: "redunar-app-linux-opensuse-x86_64.rpm",
        });
    }
    Err(format!("unsupported distribution: {id}"))
}

fn verify_checksum(path: &Path, expected: &str) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_PACKAGE_BYTES {
        return Err("package exceeds the size limit".into());
    }
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read =
            std::io::Read::read(&mut file, &mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = format!("{:x}", digest.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {expected}, got {actual}"))
    }
}

fn verified_release_version(
    path: &Path,
    manifest_entries: &BTreeMap<String, String>,
) -> Result<String, String> {
    let digest = manifest_entries
        .get("VERSION")
        .ok_or("the signed manifest has no VERSION checksum")?;
    verify_checksum(path, digest)?;
    let version = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let version = version.trim();
    if !valid_version(version) {
        return Err("VERSION is not a supported version number".into());
    }
    Ok(version.to_owned())
}

fn verify_signature_with_key(
    manifest: &Path,
    signature: &Path,
    directory: &Path,
    public_key_pem: &str,
) -> Result<(), String> {
    let public_key = directory.join("release-public-key.pem");
    let mut file = File::create(&public_key).map_err(|error| error.to_string())?;
    file.write_all(public_key_pem.as_bytes())
        .map_err(|error| error.to_string())?;
    let openssl = ["/usr/bin/openssl", "/bin/openssl"]
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .ok_or("OpenSSL is not installed")?;
    let status = Command::new(openssl)
        .args(["dgst", "-sha256", "-verify"])
        .arg(&public_key)
        .args(["-signature"])
        .arg(signature)
        .arg(manifest)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("openssl exited with {status}"))
    }
}

fn parse_manifest(input: &str) -> Result<BTreeMap<String, String>, String> {
    if input.len() as u64 > MAX_METADATA_BYTES {
        return Err("manifest exceeds the size limit".into());
    }
    let mut entries = BTreeMap::new();
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let digest = fields.next().ok_or("manifest entry has no digest")?;
        let name = fields.next().ok_or("manifest entry has no filename")?;
        if fields.next().is_some()
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == "."
            || name == ".."
        {
            return Err(format!("invalid manifest entry: {line}"));
        }
        if entries
            .insert(name.to_owned(), digest.to_ascii_lowercase())
            .is_some()
        {
            return Err(format!("duplicate manifest entry: {name}"));
        }
    }
    if entries.is_empty() {
        return Err("manifest is empty".into());
    }
    Ok(entries)
}

fn valid_version(version: &str) -> bool {
    let trimmed = version.strip_prefix('v').unwrap_or(version);
    let parts: Vec<_> = trimmed.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| {
        value
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let candidate = parse(candidate);
    let current = parse(current);
    candidate > current
}

#[cfg(test)]
mod tests {
    use super::{
        check_source_with, hold_pending, install_verified_update, is_newer_version, package_target,
        parse_installed_version, parse_manifest, parse_pending_update, pending_package_path,
        persist_verified_package_in_directory, read_pending_update_in_directory,
        retain_if_latest_not_newer, retained_or_error, select_package, signed_metadata_path,
        valid_version, verified_release_version, verify_checksum, verify_signature_with_key,
        write_pending_update_in_directory, PackageTarget, PendingUpdate, UpdateCheckStatus,
    };
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use std::process::{Command, Stdio};

    #[test]
    fn manifest_parser_accepts_sha256sum_output_and_rejects_paths() {
        let manifest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  redunar-app-linux-x86_64.rpm\n";
        let parsed = parse_manifest(manifest).expect("valid manifest");
        assert_eq!(parsed.len(), 1);
        assert!(parse_manifest(&format!("{}  ../escape", "0".repeat(64))).is_err());
    }

    #[test]
    fn manifest_parser_rejects_duplicates_and_bad_digests() {
        let digest = "a".repeat(64);
        let manifest = format!("{digest}  one\n{digest}  one\n");
        assert!(parse_manifest(&manifest).is_err());
        assert!(parse_manifest("not-a-digest  one").is_err());
    }

    #[test]
    fn versions_are_strict_and_numeric() {
        assert!(valid_version("0.1.0"));
        assert!(valid_version("v1.2.3"));
        assert!(!valid_version("1.2"));
        assert!(is_newer_version("0.2.0", "0.1.9"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
    }

    #[test]
    fn retained_update_refreshes_only_for_a_newer_signed_release() {
        let cached = UpdateCheckStatus {
            current_version: "0.1.3".into(),
            state: "available".into(),
            message: "A verified package is cached.".into(),
            latest_version: Some("0.1.4".into()),
            package_kind: Some("rpm".into()),
            asset_name: Some("redunar-app-linux-x86_64.rpm".into()),
        };
        assert_eq!(
            retain_if_latest_not_newer(Some(&cached), "0.1.4"),
            Some(cached.clone())
        );
        assert_eq!(
            retain_if_latest_not_newer(Some(&cached), "0.1.3"),
            Some(cached.clone())
        );
        assert_eq!(retain_if_latest_not_newer(Some(&cached), "0.1.5"), None);
        let offline = retained_or_error(
            Some(cached.clone()),
            super::error_status("0.1.3", "The source is offline."),
        );
        assert_eq!(offline.state, "available");
        assert!(offline.message.contains("could not be checked"));
        let handoff = UpdateCheckStatus {
            state: "handoff".into(),
            ..cached.clone()
        };
        assert!(hold_pending(&handoff, false));
        assert!(!hold_pending(&handoff, true));
        let installed = UpdateCheckStatus {
            state: "restart-needed".into(),
            ..cached
        };
        assert!(hold_pending(&installed, false));
        assert!(hold_pending(&installed, true));
    }

    #[test]
    fn package_targets_match_supported_distribution_families() {
        assert_eq!(
            select_package("x86_64", "ubuntu", "debian", false),
            Ok(PackageTarget {
                kind: "deb",
                asset_name: "redunar-app-linux-amd64.deb"
            })
        );
        assert_eq!(
            select_package("x86_64", "fedora", "", false),
            Ok(PackageTarget {
                kind: "rpm",
                asset_name: "redunar-app-linux-x86_64.rpm"
            })
        );
        assert_eq!(
            select_package("amd64", "arch", "", false),
            Ok(PackageTarget {
                kind: "arch",
                asset_name: "redunar-app-linux-x86_64.pkg.tar.zst"
            })
        );
        assert_eq!(
            select_package("x86_64", "opensuse-tumbleweed", "", false),
            Ok(PackageTarget {
                kind: "rpm",
                asset_name: "redunar-app-linux-opensuse-x86_64.rpm"
            })
        );
        assert!(select_package("aarch64", "ubuntu", "", false).is_err());
        assert!(select_package("x86_64", "fedora", "", true).is_err());
    }

    #[test]
    fn checksum_verification_accepts_matching_bytes_and_rejects_changes() {
        let path = std::env::temp_dir().join(format!(
            "redunar-update-checksum-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let bytes = b"verified package fixture";
        std::fs::write(&path, bytes).expect("fixture write");
        let mut digest = Sha256::new();
        digest.update(bytes);
        let expected = format!("{:x}", digest.finalize());
        assert!(verify_checksum(&path, &expected).is_ok());
        assert!(verify_checksum(&path, &"0".repeat(64)).is_err());
        std::fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn release_version_must_match_the_signed_manifest() {
        let path = std::env::temp_dir().join(format!(
            "redunar-update-version-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let contents = b"0.2.0\n";
        std::fs::write(&path, contents).expect("version fixture");
        let mut digest = Sha256::new();
        digest.update(contents);
        let entries = parse_manifest(&format!("{:x}  VERSION\n", digest.finalize()))
            .expect("signed version entry");
        assert_eq!(
            verified_release_version(&path, &entries),
            Ok("0.2.0".into())
        );
        std::fs::write(&path, b"0.3.0\n").expect("tampered version");
        assert!(verified_release_version(&path, &entries).is_err());
        assert!(verified_release_version(&path, &Default::default()).is_err());
        std::fs::remove_file(path).expect("fixture cleanup");
    }

    #[test]
    fn installed_package_versions_require_the_expected_package_and_format() {
        assert_eq!(
            parse_installed_version("rpm", "0.2.0"),
            Some("0.2.0".into())
        );
        assert_eq!(
            parse_installed_version("deb", "1:0.2.0-1"),
            Some("0.2.0".into())
        );
        assert_eq!(
            parse_installed_version("arch", "redunar-app 0.2.0-1"),
            Some("0.2.0".into())
        );
        assert!(parse_installed_version("arch", "another-app 0.2.0-1").is_none());
        assert!(parse_installed_version("rpm", "not-a-version").is_none());
    }

    #[test]
    fn retained_update_metadata_is_strict_and_safe() {
        let metadata = format!(
            "state=downloaded\nversion=0.2.0\nkind=deb\nasset_name=redunar-app-linux-amd64.deb\ndigest={}\n",
            "a".repeat(64)
        );
        let parsed = parse_pending_update(&metadata).expect("valid retained metadata");
        assert_eq!(parsed.version, "0.2.0");
        assert!(parse_pending_update(
            "state=downloaded\nversion=0.2.0\nkind=deb\nasset_name=../escape\ndigest=bad\n"
        )
        .is_err());
        assert!(parse_pending_update(&metadata.replace("kind=deb", "kind=portable")).is_err());
    }

    #[test]
    fn failed_cache_commit_keeps_the_previous_verified_package() {
        let directory = std::env::temp_dir().join(format!(
            "redunar-update-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("cache fixture");
        let target = PackageTarget {
            kind: "rpm",
            asset_name: "redunar-app-linux-x86_64.rpm",
        };
        let old_bytes = b"old verified package";
        let old_digest = format!("{:x}", Sha256::digest(old_bytes));
        let old_path = directory.join(target.asset_name);
        std::fs::write(&old_path, old_bytes).expect("legacy cached package");
        let old_pending = PendingUpdate {
            state: "handoff".into(),
            version: "0.1.4".into(),
            kind: "rpm".into(),
            asset_name: target.asset_name.into(),
            digest: old_digest.clone(),
        };
        write_pending_update_in_directory(&directory, &old_pending).expect("old metadata");

        let new_bytes = b"new verified package";
        let new_digest = format!("{:x}", Sha256::digest(new_bytes));
        let source = directory.join("source.rpm");
        std::fs::write(&source, new_bytes).expect("new package fixture");
        std::fs::create_dir(directory.join(".pending-update.txt.tmp"))
            .expect("block metadata commit");
        assert!(persist_verified_package_in_directory(
            &directory,
            &source,
            target,
            "0.1.5",
            &new_digest
        )
        .is_err());
        assert_eq!(
            read_pending_update_in_directory(&directory).expect("retained metadata"),
            old_pending
        );
        assert_eq!(
            std::fs::read(pending_package_path(&directory, &old_pending))
                .expect("retained package"),
            old_bytes
        );
        std::fs::remove_dir(directory.join(".pending-update.txt.tmp"))
            .expect("unblock metadata commit");

        persist_verified_package_in_directory(&directory, &source, target, "0.1.5", &new_digest)
            .expect("commit new package");
        let current = read_pending_update_in_directory(&directory).expect("new metadata");
        assert_eq!(current.version, "0.1.5");
        assert_eq!(
            std::fs::read(pending_package_path(&directory, &current)).expect("new package"),
            new_bytes
        );
        assert!(!old_path.exists());
        std::fs::remove_dir_all(directory).expect("fixture cleanup");
    }

    #[test]
    fn native_manifest_verifier_rejects_tampered_signed_metadata() {
        let directory = std::env::temp_dir().join(format!(
            "redunar-signed-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("signed fixture directory");
        let private_key = directory.join("private.pem");
        let public_key = directory.join("public.pem");
        let manifest = directory.join("SHA256SUMS");
        let signature = directory.join("SHA256SUMS.sig");
        std::fs::write(&manifest, format!("{}  VERSION\n", "a".repeat(64)))
            .expect("signed manifest fixture");
        let run = |arguments: &[&str]| {
            assert!(Command::new("openssl")
                .args(arguments)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("OpenSSL fixture command")
                .success());
        };
        run(&[
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
            private_key.to_str().expect("private key path"),
        ]);
        run(&[
            "pkey",
            "-in",
            private_key.to_str().expect("private key path"),
            "-pubout",
            "-out",
            public_key.to_str().expect("public key path"),
        ]);
        run(&[
            "dgst",
            "-sha256",
            "-sign",
            private_key.to_str().expect("private key path"),
            "-out",
            signature.to_str().expect("signature path"),
            manifest.to_str().expect("manifest path"),
        ]);
        let public_pem = std::fs::read_to_string(&public_key).expect("fixture public key");
        assert!(verify_signature_with_key(&manifest, &signature, &directory, &public_pem).is_ok());
        std::fs::write(&manifest, b"tampered manifest\n").expect("tamper manifest");
        assert!(verify_signature_with_key(&manifest, &signature, &directory, &public_pem).is_err());
        std::fs::remove_dir_all(directory).expect("fixture cleanup");
    }

    #[test]
    fn native_signed_feed_downloads_and_refreshes_only_verified_packages() {
        let root = std::env::temp_dir().join(format!(
            "redunar-signed-feed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture time")
                .as_nanos()
        ));
        let feed = root.join("feed");
        let work = root.join("work");
        let cache = root.join("cache");
        for directory in [&feed, &work, &cache] {
            std::fs::create_dir_all(directory).expect("fixture directory");
        }
        let private_key = root.join("private.pem");
        let public_key = root.join("public.pem");
        let run = |arguments: &[&str]| {
            assert!(Command::new("openssl")
                .args(arguments)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("OpenSSL fixture command")
                .success());
        };
        run(&[
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
            private_key.to_str().expect("private key path"),
        ]);
        run(&[
            "pkey",
            "-in",
            private_key.to_str().expect("private key path"),
            "-pubout",
            "-out",
            public_key.to_str().expect("public key path"),
        ]);
        let public_pem = std::fs::read_to_string(public_key).expect("fixture public key");
        let target = package_target().expect("supported updater test platform");
        let source = format!("file://{}", feed.display());
        let write_release = |version: &str, bytes: &[u8]| {
            let version_bytes = format!("{version}\n");
            std::fs::write(feed.join("VERSION"), &version_bytes).expect("release version");
            std::fs::write(feed.join(target.asset_name), bytes).expect("release asset");
            let manifest = format!(
                "{:x}  {}\n{:x}  VERSION\n",
                Sha256::digest(bytes),
                target.asset_name,
                Sha256::digest(version_bytes.as_bytes())
            );
            std::fs::write(feed.join("SHA256SUMS"), manifest).expect("release manifest");
            run(&[
                "dgst",
                "-sha256",
                "-sign",
                private_key.to_str().expect("private key path"),
                "-out",
                feed.join("SHA256SUMS.sig")
                    .to_str()
                    .expect("signature path"),
                feed.join("SHA256SUMS").to_str().expect("manifest path"),
            ]);
        };
        let check = |retained: Option<&UpdateCheckStatus>| {
            check_source_with(
                &source,
                "0.1.3",
                &work,
                retained,
                &public_pem,
                true,
                |path, package, version, digest| {
                    persist_verified_package_in_directory(&cache, path, package, version, digest)
                },
            )
        };

        write_release("0.1.4", b"first signed package");
        let first = check(None);
        assert_eq!(first.state, "available");
        assert_eq!(first.latest_version.as_deref(), Some("0.1.4"));
        let first_pending = read_pending_update_in_directory(&cache).expect("first package");
        let first_path = pending_package_path(&cache, &first_pending);
        assert_eq!(
            std::fs::read(&first_path).expect("cached first package"),
            b"first signed package"
        );

        std::fs::write(feed.join("VERSION"), b"0.1.9\n").expect("tamper version");
        let bad_version = check(Some(&first));
        assert_eq!(bad_version.state, "error");
        assert_eq!(
            retained_or_error(Some(first.clone()), bad_version).state,
            "available"
        );
        assert!(first_path.exists());

        write_release("0.1.5", b"newer signed package");
        let second = check(Some(&first));
        assert_eq!(second.state, "available");
        assert_eq!(second.latest_version.as_deref(), Some("0.1.5"));
        let second_pending = read_pending_update_in_directory(&cache).expect("newer package");
        assert_eq!(
            std::fs::read(pending_package_path(&cache, &second_pending))
                .expect("cached newer package"),
            b"newer signed package"
        );
        assert!(!first_path.exists());

        write_release("0.1.6", b"expected next package");
        std::fs::write(feed.join(target.asset_name), b"tampered next package")
            .expect("tamper package");
        assert_eq!(check(Some(&second)).state, "error");
        assert_eq!(
            read_pending_update_in_directory(&cache).expect("retained newer package"),
            second_pending
        );
        std::fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn installer_handoff_can_retry_and_reports_restart_only_after_package_install() {
        let directory = std::env::temp_dir().join(format!(
            "redunar-installer-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("installer fixture directory");
        let target = PackageTarget {
            kind: "rpm",
            asset_name: "redunar-app-linux-x86_64.rpm",
        };
        let bytes = b"verified update package";
        let digest = format!("{:x}", Sha256::digest(bytes));
        let pending = PendingUpdate {
            state: "downloaded".into(),
            version: "0.1.4".into(),
            kind: target.kind.into(),
            asset_name: target.asset_name.into(),
            digest,
        };
        let versioned = directory.join(format!("{}.{}", target.asset_name, pending.digest));
        std::fs::write(&versioned, bytes).expect("cached package");
        for name in ["VERSION", "SHA256SUMS", "SHA256SUMS.sig"] {
            std::fs::write(
                signed_metadata_path(&directory, name, &pending.digest),
                b"fixture",
            )
            .expect("cached signed metadata");
        }
        write_pending_update_in_directory(&directory, &pending).expect("pending metadata");
        let helper = Path::new("/usr/libexec/redunar-update-helper");
        let mut launches = 0;
        for _ in 0..2 {
            let current = read_pending_update_in_directory(&directory).expect("pending update");
            let status = install_verified_update(
                &directory,
                current,
                target,
                "0.1.3",
                Some("0.1.3"),
                Some(helper),
                |_, path| {
                    assert_eq!(path, directory);
                    launches += 1;
                    Ok(None)
                },
            );
            assert_eq!(status.state, "handoff");
            assert_eq!(
                read_pending_update_in_directory(&directory)
                    .expect("handoff metadata")
                    .state,
                "handoff"
            );
        }
        assert_eq!(launches, 2);
        let versioned_handoff = install_verified_update(
            &directory,
            pending.clone(),
            target,
            "0.1.3",
            Some("0.1.3"),
            Some(helper),
            |_, path| {
                assert_eq!(path, directory);
                launches += 1;
                Ok(None)
            },
        );
        assert_eq!(versioned_handoff.state, "handoff");
        assert_eq!(launches, 3);
        let installed = install_verified_update(
            &directory,
            pending.clone(),
            target,
            "0.1.3",
            Some("0.1.4"),
            Some(helper),
            |_, _| panic!("installed update must not reopen the package installer"),
        );
        assert_eq!(installed.state, "restart-needed");

        write_pending_update_in_directory(&directory, &pending).expect("reset metadata");
        let failed = install_verified_update(
            &directory,
            pending.clone(),
            target,
            "0.1.3",
            Some("0.1.3"),
            Some(helper),
            |_, _| Err("authorization cancelled".into()),
        );
        assert_eq!(failed.state, "available");
        assert_eq!(
            read_pending_update_in_directory(&directory).expect("restored metadata"),
            pending
        );
        std::fs::write(&versioned, b"tampered package").expect("tamper cached package");
        let tampered = install_verified_update(
            &directory,
            pending,
            target,
            "0.1.3",
            Some("0.1.4"),
            Some(helper),
            |_, _| panic!("tampered package must not open the installer"),
        );
        assert_eq!(tampered.state, "error");
        std::fs::remove_dir_all(directory).expect("fixture cleanup");
    }
}
