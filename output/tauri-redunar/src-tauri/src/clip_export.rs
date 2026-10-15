use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MINIMUM_TRIM_SECONDS: f64 = 0.1;
const MAXIMUM_TRIM_SECONDS: f64 = 24.0 * 60.0 * 60.0;
const EXPORT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EXPORT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OUTPUT_SIZE_ALLOWANCE: u64 = 64 * 1024 * 1024;
const OUTPUT_SIZE_MULTIPLIER: u64 = 2;
static NEXT_EXPORT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
pub struct ClipExports(Arc<Mutex<Option<ActiveExport>>>, Arc<AtomicBool>);

struct ActiveExport {
    id: u64,
    job: Arc<ExportJob>,
}

struct ExportJob {
    cancelled: AtomicBool,
    progress_thousandths: AtomicU32,
}

impl ClipExports {
    fn begin(&self) -> Result<(u64, Arc<ExportJob>), String> {
        let mut active = self.0.lock().map_err(|_| "Clip export is unavailable")?;
        if self.1.load(Ordering::Acquire) {
            return Err("Redunar is restarting".into());
        }
        if active.is_some() {
            return Err("Another clip export is already running".into());
        }
        let id = NEXT_EXPORT.fetch_add(1, Ordering::Relaxed);
        let job = Arc::new(ExportJob {
            cancelled: AtomicBool::new(false),
            progress_thousandths: AtomicU32::new(0),
        });
        *active = Some(ActiveExport {
            id,
            job: job.clone(),
        });
        Ok((id, job))
    }

    fn finish(&self, id: u64) {
        if let Ok(mut active) = self.0.lock() {
            if active.as_ref().is_some_and(|export| export.id == id) {
                active.take();
            }
        }
    }

    // Serialize mutations with export admission. An export owns its source
    // filename until archive/commit finishes; playback owns an open descriptor.
    pub fn with_idle<T>(&self, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let active = self.0.lock().map_err(|_| "Clip export is unavailable")?;
        if self.1.load(Ordering::Acquire) {
            return Err("Redunar is restarting".into());
        }
        if active.is_some() {
            return Err(
                "Finish or cancel the export before changing the clip or replay folder".into(),
            );
        }
        action()
    }

    pub(crate) fn claim_restart(
        &self,
        claim: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.with_idle(|| {
            claim()?;
            self.1.store(true, Ordering::Release);
            Ok(())
        })
    }

    pub(crate) fn cancel_restart(&self) {
        self.1.store(false, Ordering::Release);
    }

    pub fn shutdown(&self) {
        if let Ok(active) = self.0.lock() {
            if let Some(export) = active.as_ref() {
                export.job.cancelled.store(true, Ordering::Release);
            }
        }
    }
}

#[derive(Serialize)]
pub struct ClipExportStatus {
    active: bool,
    progress: f64,
}

#[derive(Serialize)]
pub struct ExportedClip {
    file_name: String,
    bytes: u64,
    duration_seconds: f64,
    destination: String,
}

#[tauri::command]
pub fn clip_export_status(exports: tauri::State<'_, ClipExports>) -> ClipExportStatus {
    let Ok(active) = exports.0.lock() else {
        return ClipExportStatus {
            active: false,
            progress: 0.0,
        };
    };
    active.as_ref().map_or(
        ClipExportStatus {
            active: false,
            progress: 0.0,
        },
        |export| ClipExportStatus {
            active: true,
            progress: f64::from(export.job.progress_thousandths.load(Ordering::Acquire)) / 10.0,
        },
    )
}

#[tauri::command]
pub fn cancel_clip_export(exports: tauri::State<'_, ClipExports>) -> Result<(), String> {
    let active = exports.0.lock().map_err(|_| "Clip export is unavailable")?;
    let export = active.as_ref().ok_or("No clip export is running")?;
    export.job.cancelled.store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub async fn export_replay_clip(
    file_name: String,
    start_seconds: f64,
    end_seconds: f64,
    output_name: Option<String>,
    exports: tauri::State<'_, ClipExports>,
) -> Result<ExportedClip, String> {
    crate::backend::ensure_write_access()?;
    let duration = validate_trim_range(start_seconds, end_seconds)?;
    let exports = exports.inner().clone();
    let (id, job) = exports.begin()?;
    let opened = match crate::media::open_inventory_clip(&file_name) {
        Ok(opened) => opened,
        Err(error) => {
            exports.finish(id);
            return Err(error);
        }
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        export_clip(
            opened.file,
            opened.bytes,
            opened.game_name,
            start_seconds,
            duration,
            &job,
            output_name.as_deref(),
        )
    })
    .await
    .map_err(|error| format!("Clip export worker failed: {error}"));
    exports.finish(id);
    result?
}

fn validate_trim_range(start_seconds: f64, end_seconds: f64) -> Result<f64, String> {
    if !start_seconds.is_finite() || !end_seconds.is_finite() {
        return Err("The selected trim range is invalid".into());
    }
    if start_seconds < 0.0 || end_seconds > MAXIMUM_TRIM_SECONDS {
        return Err("The selected trim range is outside the supported clip duration".into());
    }
    let duration = end_seconds - start_seconds;
    if duration < MINIMUM_TRIM_SECONDS {
        return Err("Select at least 0.1 seconds to export".into());
    }
    Ok(duration)
}

fn export_clip(
    source: File,
    source_bytes: u64,
    source_game_name: Option<String>,
    start_seconds: f64,
    duration_seconds: f64,
    job: &ExportJob,
    output_name: Option<&str>,
) -> Result<ExportedClip, String> {
    // Validate the inspection tool before spending time encoding an export.
    let _probe = crate::media_tools::ffprobe()?;
    let temporary = TemporaryExport::new().map_err(|error| {
        format!("A private temporary export file could not be created: {error}")
    })?;
    render_trim(
        source,
        source_bytes,
        start_seconds,
        duration_seconds,
        job,
        &temporary.path,
    )?;
    if job.cancelled.load(Ordering::Acquire) {
        return Err("Clip export was cancelled".into());
    }
    let actual_duration = probe_duration(&temporary.path)?;
    if job.cancelled.load(Ordering::Acquire) {
        return Err("Clip export was cancelled".into());
    }
    let minimum_expected = (duration_seconds - 0.35).max(MINIMUM_TRIM_SECONDS * 0.5);
    if actual_duration < minimum_expected || actual_duration > duration_seconds + 0.75 {
        return Err(format!(
            "The exported clip duration was unexpected ({actual_duration:.2}s instead of {duration_seconds:.2}s)"
        ));
    }
    let bytes = temporary
        .path
        .metadata()
        .map_err(|error| format!("Exported clip metadata is unavailable: {error}"))?
        .len();
    let store = crate::backend::service()
        .prepare_replay_store()
        .map_err(|error| error.to_string())?;
    store
        .ensure_free_space(bytes)
        .map_err(|error| error.to_string())?;
    let mut rendered = File::open(&temporary.path)
        .map_err(|error| format!("The trimmed clip could not be reopened: {error}"))?;
    let saved = match output_name {
        Some(stem) => store.save_named_mp4_preserving_existing(&mut rendered, stem),
        None => store.save_mp4_preserving_existing(&mut rendered),
    }
    .map_err(|error| error.to_string())?;
    let file_name = saved
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("The exported clip name is invalid")?
        .to_owned();
    // Carry the source clip's game attribution onto the trimmed export.
    // Labeling only: a ledger failure never fails a completed export.
    if let Some(game_name) = source_game_name {
        let identity = store.inventory().ok().and_then(|clips| {
            clips
                .into_iter()
                .find(|clip| clip.file_name == file_name)
                .map(|clip| clip.original_file_name)
        });
        if let Some(identity) = identity {
            let _ = crate::backend::service().record_clip_game(&identity, &game_name);
        }
    }
    job.progress_thousandths.store(1000, Ordering::Release);
    Ok(ExportedClip {
        file_name,
        bytes: saved.bytes,
        duration_seconds: actual_duration,
        destination: store.directory().to_string_lossy().into_owned(),
    })
}

fn render_trim(
    source: File,
    source_bytes: u64,
    start_seconds: f64,
    duration_seconds: f64,
    job: &ExportJob,
    output: &Path,
) -> Result<(), String> {
    let max_output_bytes = temporary_output_limit(source_bytes)?;
    let mut child = crate::media_tools::ffmpeg(Some("libx264"))?
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            "pipe:0",
            "-ss",
            &format!("{start_seconds:.6}"),
            "-t",
            &format!("{duration_seconds:.6}"),
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-map_metadata",
            "-1",
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
            "-progress",
            "pipe:2",
            "-nostats",
            "-f",
            "mp4",
            "-y",
        ])
        .arg(output)
        .stdin(Stdio::from(source))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "FFmpeg is required to export a trimmed clip".into()
            } else {
                format!("Clip export could not start: {error}")
            }
        })?;
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate(&mut child);
            return Err("Clip export diagnostics are unavailable".into());
        }
    };
    let progress = Arc::new(AtomicU32::new(0));
    let progress_reader = progress.clone();
    let reader = match std::thread::Builder::new()
        .name("redunar-clip-export-progress".into())
        .spawn(move || read_ffmpeg_progress(stderr, duration_seconds, progress_reader))
    {
        Ok(reader) => reader,
        Err(error) => {
            terminate(&mut child);
            return Err(format!("Clip export progress could not start: {error}"));
        }
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate(&mut child);
                let _ = reader.join();
                return Err(format!("Clip export process failed: {error}"));
            }
        }
        job.progress_thousandths
            .store(progress.load(Ordering::Acquire), Ordering::Release);
        if job.cancelled.load(Ordering::Acquire) {
            terminate(&mut child);
            let _ = reader.join();
            return Err("Clip export was cancelled".into());
        }
        if output
            .metadata()
            .is_ok_and(|metadata| metadata.len() > max_output_bytes)
        {
            terminate(&mut child);
            let _ = reader.join();
            return Err("The trimmed clip exceeded its safe temporary size limit".into());
        }
        if started.elapsed() >= EXPORT_TIMEOUT {
            terminate(&mut child);
            let _ = reader.join();
            return Err("Clip export timed out".into());
        }
        std::thread::sleep(EXPORT_POLL_INTERVAL);
    };
    let diagnostics = reader
        .join()
        .unwrap_or_else(|_| "Clip export diagnostics stopped unexpectedly".into());
    if !status.success() {
        return Err(
            if let Some(message) = crate::media_tools::capability_error(&diagnostics) {
                message
            } else if diagnostics.is_empty() {
                "FFmpeg could not export the selected range".into()
            } else {
                format!("FFmpeg could not export the selected range: {diagnostics}")
            },
        );
    }
    job.progress_thousandths.store(950, Ordering::Release);
    Ok(())
}

fn temporary_output_limit(source_bytes: u64) -> Result<u64, String> {
    source_bytes
        .checked_mul(OUTPUT_SIZE_MULTIPLIER)
        .and_then(|bytes| bytes.checked_add(OUTPUT_SIZE_ALLOWANCE))
        .ok_or_else(|| "The source clip is too large to export safely".into())
}

fn read_ffmpeg_progress(
    stderr: impl Read,
    duration_seconds: f64,
    progress: Arc<AtomicU32>,
) -> String {
    let mut diagnostics = String::new();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if let Some(value) = line.strip_prefix("out_time_us=") {
            if let Ok(microseconds) = value.parse::<f64>() {
                let fraction = microseconds / 1_000_000.0 / duration_seconds;
                progress.store(
                    (fraction.clamp(0.0, 0.95) * 1000.0) as u32,
                    Ordering::Release,
                );
            }
        } else if !line.contains('=') {
            if !diagnostics.is_empty() {
                diagnostics.push(' ');
            }
            diagnostics.push_str(line.trim());
            if diagnostics.len() > 1200 {
                diagnostics.truncate(1200);
            }
        }
    }
    diagnostics
}

fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn probe_duration(path: &Path) -> Result<f64, String> {
    let output = crate::media_tools::ffprobe()?
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "FFprobe is required to validate the trimmed clip".into()
            } else {
                format!("The trimmed clip could not be validated: {error}")
            }
        })?;
    if !output.status.success() {
        return Err("The trimmed clip could not be validated".into());
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "The trimmed clip reported an invalid duration")?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("The trimmed clip reported an invalid duration".into());
    }
    Ok(duration)
}

struct TemporaryExport {
    directory: PathBuf,
    path: PathBuf,
}

impl TemporaryExport {
    fn new() -> io::Result<Self> {
        for _ in 0..8 {
            let mut random = [0_u8; 16];
            File::open("/dev/urandom")?.read_exact(&mut random)?;
            let token = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let directory = std::env::temp_dir().join(format!("redunar-export-{token}"));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&directory) {
                Ok(()) => {
                    let path = directory.join("trimmed.mp4");
                    let _ = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&path)?;
                    return Ok(Self { directory, path });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a private clip export directory",
        ))
    }
}

impl Drop for TemporaryExport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_range_rejects_invalid_or_empty_intervals() {
        for (start, end) in [
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (-1.0, 1.0),
            (2.0, 1.0),
            (1.0, 1.05),
            (0.0, MAXIMUM_TRIM_SECONDS + 1.0),
        ] {
            assert!(validate_trim_range(start, end).is_err());
        }
        assert_eq!(validate_trim_range(1.0, 2.5).unwrap(), 1.5);
    }

    #[test]
    fn export_state_rejects_overlap_and_shutdown_requests_cancellation() {
        let exports = ClipExports::default();
        let (id, job) = exports.begin().expect("begin export");
        assert!(exports.begin().is_err());
        exports.shutdown();
        assert!(job.cancelled.load(Ordering::Acquire));
        exports.finish(id);
        assert!(exports.begin().is_ok());
    }

    #[test]
    fn mutations_wait_for_export_completion_and_failures_release_admission() {
        let exports = ClipExports::default();
        let (id, _) = exports.begin().expect("begin export");
        let called = AtomicBool::new(false);
        assert!(exports
            .with_idle(|| {
                called.store(true, Ordering::Release);
                Ok(())
            })
            .is_err());
        assert!(!called.load(Ordering::Acquire));
        exports.finish(id);
        assert!(exports
            .with_idle(|| Err::<(), _>("Fixture mutation failed".into()))
            .is_err());
        let (next, _) = exports.begin().expect("failed mutation released admission");
        exports.finish(id);
        assert!(
            exports.with_idle(|| Ok(())).is_err(),
            "stale completion must not release a new export"
        );
        exports.finish(next);
        assert!(exports.with_idle(|| Ok(())).is_ok());
    }

    #[test]
    fn restart_claim_rejects_exports_and_failed_claims_keep_admission() {
        let exports = ClipExports::default();
        let (id, _) = exports.begin().unwrap();
        assert!(exports.claim_restart(|| Ok(())).is_err());
        exports.finish(id);
        assert!(exports
            .claim_restart(|| Err("Session started".into()))
            .is_err());
        let (id, _) = exports.begin().unwrap();
        exports.finish(id);
        exports.claim_restart(|| Ok(())).unwrap();
        assert!(exports.begin().is_err());
        assert!(exports.claim_restart(|| Ok(())).is_err());
        assert!(exports.with_idle(|| Ok(())).is_err());
    }

    #[test]
    fn ffmpeg_progress_is_bounded_to_the_encoding_stage() {
        let progress = Arc::new(AtomicU32::new(0));
        let diagnostics = read_ffmpeg_progress(
            b"out_time_us=2500000\nprogress=continue\nout_time_us=12000000\nencoder failed\n"
                .as_slice(),
            10.0,
            progress.clone(),
        );
        assert_eq!(progress.load(Ordering::Acquire), 950);
        assert_eq!(diagnostics, "encoder failed");
    }

    #[test]
    fn temporary_export_allows_reencoding_overhead_without_becoming_unbounded() {
        assert_eq!(
            temporary_output_limit(8 * 1024 * 1024).unwrap(),
            80 * 1024 * 1024
        );
        assert!(temporary_output_limit(u64::MAX).is_err());
    }
}
