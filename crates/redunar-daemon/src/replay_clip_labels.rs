//! Store-local, inode-bound labels authorize only explicitly renamed recordings.
//! Persist destination authorization before renaming: either filename remains
//! discoverable after a crash, without broadening inventory to arbitrary videos.
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

pub(super) const FILE: &str = ".redunar-clip-labels-v1";
const HEADER: &str = "redunar-clip-labels-v1";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
#[derive(Clone)]
pub(super) struct Label {
    pub original: String,
    pub favorite: bool,
    dev: u64,
    ino: u64,
}
impl Label {
    pub fn new(original: String, metadata: &Metadata) -> Self {
        Self {
            original,
            favorite: false,
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
    pub fn matches(&self, metadata: &Metadata) -> bool {
        metadata.is_file()
            && metadata.nlink() == 1
            && self.dev == metadata.dev()
            && self.ino == metadata.ino()
    }
}
pub(super) type Labels = BTreeMap<String, Label>;
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.trim() == name
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        && matches!(
            Path::new(name)
                .extension()
                .and_then(std::ffi::OsStr::to_str),
            Some("mkv" | "mp4")
        )
}
fn hex(value: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    encoded
}
fn unhex(value: &str) -> io::Result<String> {
    if !value.len().is_multiple_of(2) {
        return Err(invalid());
    }
    let bytes = value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| invalid())?;
            u8::from_str_radix(pair, 16).map_err(|_| invalid())
        })
        .collect::<io::Result<Vec<_>>>()?;
    String::from_utf8(bytes).map_err(|_| invalid())
}
fn invalid() -> io::Error {
    io::Error::other("Replay filename metadata is invalid. Recordings have not been changed.")
}
pub(super) fn load(directory: &Path) -> io::Result<Labels> {
    load_from(directory, FILE)
}
pub(super) fn load_from(directory: &Path, file_name: &str) -> io::Result<Labels> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(file_name))
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Labels::new()),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.len() > MAX_BYTES
        || meta.uid() != fs::metadata(directory)?.uid()
    {
        return Err(invalid());
    }
    let mut text = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(invalid());
    }
    let mut labels = Labels::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 5 || labels.len() >= MAX_ENTRIES {
            return Err(invalid());
        }
        let name = unhex(fields[0])?;
        let original = unhex(fields[1])?;
        if !valid_name(&name)
            || Path::new(&name).extension() != Path::new(&original).extension()
            || !super::replay_store::is_owned_clip_name(std::ffi::OsStr::new(&original))
        {
            return Err(invalid());
        }
        let favorite = match fields[4] {
            "0" => false,
            "1" => true,
            _ => return Err(invalid()),
        };
        let label = Label {
            original,
            dev: fields[2].parse().map_err(|_| invalid())?,
            ino: fields[3].parse().map_err(|_| invalid())?,
            favorite,
        };
        if labels.insert(name, label).is_some() {
            return Err(invalid());
        }
    }
    Ok(labels)
}
pub(super) fn save(directory: &Path, labels: &mut Labels) -> io::Result<()> {
    save_to(directory, FILE, labels)
}
pub(super) fn save_to(directory: &Path, file_name: &str, labels: &Labels) -> io::Result<()> {
    // Callers retain pending destination entries until rename commits. Pruning
    // happens before a mutation, never here, so crash recovery remains valid.
    if labels.len() > MAX_ENTRIES {
        return Err(invalid());
    }
    let mut text = format!("{HEADER}\n");
    for (name, label) in labels {
        let _ = writeln!(
            text,
            "{}\t{}\t{}\t{}\t{}",
            hex(name),
            hex(&label.original),
            label.dev,
            label.ino,
            u8::from(label.favorite)
        );
    }
    if text.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let (temporary, mut file) =
        super::replay_store::create_temporary(directory).map_err(io::Error::other)?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, directory.join(file_name))?;
        super::replay_store::sync_directory(directory)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub(super) fn prune(directory: &Path, labels: &mut Labels) {
    labels.retain(|name, label| {
        fs::symlink_metadata(directory.join(name)).is_ok_and(|m| label.matches(&m))
    });
}
