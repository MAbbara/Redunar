//! Bounded same-user handoff before GTK startup. The backend lease remains
//! authoritative; this endpoint can only request showing the existing window.
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const REQUEST: &[u8; 15] = b"RedunarOpenV1\r\n";
const IO_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

type Action = Box<dyn Fn() + Send>;
#[derive(Default)]
struct Dispatch {
    action: Option<Action>,
    pending: bool,
}
pub struct Activation {
    dispatch: Arc<Mutex<Dispatch>>,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    path: PathBuf,
    identity: (u64, u64),
}
impl Activation {
    pub fn bind(path: PathBuf) -> io::Result<Self> {
        let parent = fs::symlink_metadata(
            path.parent()
                .ok_or_else(|| io::Error::other("missing activation directory"))?,
        )?;
        // Do not publish a control endpoint in a writable/shared directory.
        if !parent.is_dir()
            || parent.uid() != unsafe { libc::geteuid() }
            || parent.mode() & 0o022 != 0
        {
            return Err(io::Error::other("unsafe activation directory"));
        }
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket()
                || meta.uid() != parent.uid()
                || connect_bounded(&path).is_ok()
            {
                return Err(io::Error::other(
                    "activation endpoint is already occupied or unsafe",
                ));
            }
            fs::remove_file(&path)?; // Only the primary lease holder reaches bind.
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let meta = fs::symlink_metadata(&path)?;
        let dispatch = Arc::new(Mutex::new(Dispatch::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_dispatch = dispatch.clone();
        let thread_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if !same_user(&stream) {
                            continue;
                        }
                        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                        let mut request = [0; REQUEST.len()];
                        if stream.read_exact(&mut request).is_ok() && request == *REQUEST {
                            let mut dispatch = thread_dispatch
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if let Some(action) = &dispatch.action {
                                action();
                            } else {
                                dispatch.pending = true;
                            }
                            let _ = stream.write_all(b"OK");
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            dispatch,
            stop,
            worker: Mutex::new(Some(worker)),
            path,
            identity: (meta.dev(), meta.ino()),
        })
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
    pub fn ready(&self, action: impl Fn() + Send + 'static) {
        let mut dispatch = self
            .dispatch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if dispatch.pending {
            action();
            dispatch.pending = false;
        }
        dispatch.action = Some(Box::new(action));
    }
}
impl Drop for Activation {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn same_user(stream: &UnixStream) -> bool {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SO_PEERCRED reads kernel credentials of the connected peer, never a UI token.
    unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        ) == 0
            && credentials.uid == libc::geteuid()
    }
}
// A full UNIX listener backlog can block a normal connect indefinitely.
// Nonblocking connect plus poll keeps even that path inside the I/O deadline.
fn connect_bounded(path: &Path) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::other("invalid activation endpoint"));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // Own the descriptor immediately so every failure closes it.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let length = (std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1) as libc::socklen_t;
    if unsafe { libc::connect(fd.as_raw_fd(), (&raw const address).cast(), length) } != 0 {
        let error = io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
        ) {
            return Err(error);
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        if unsafe { libc::poll(&raw mut poll, 1, 250) } <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "activation connection timed out",
            ));
        }
        let mut error = 0i32;
        let mut size = std::mem::size_of::<i32>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut error).cast(),
                &raw mut size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

pub fn request(path: &Path) -> io::Result<()> {
    request_until(path, STARTUP_TIMEOUT)
}
fn request_until(path: &Path, timeout: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        if let Ok(mut stream) = connect_bounded(path) {
            if same_user(&stream) {
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                stream.write_all(REQUEST)?;
                let mut reply = [0; 2];
                stream.read_exact(&mut reply)?;
                return if &reply == b"OK" {
                    Ok(())
                } else {
                    Err(io::Error::other("activation rejected"))
                };
            }
            return Err(io::Error::other("activation owner is not the same user"));
        }
        if started.elapsed() >= timeout {
            return Err(io::Error::other(
                "existing Redunar cannot be activated; restart the existing app",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ru-act-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    #[test]
    fn startup_pending_repeated_requests_and_cleanup() {
        let root = root("pending");
        let path = root.join("a.sock");
        let activation = Activation::bind(path.clone()).unwrap();
        request(&path).unwrap();
        request(&path).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        activation.ready(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 1);
        request(&path).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(Activation::bind(path.clone()).is_err());
        drop(activation);
        assert!(!path.exists());
        let restarted = Activation::bind(path.clone()).unwrap();
        drop(restarted);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_socket_symlink_malformed_and_slow_client() {
        let root = root("stale");
        let path = root.join("a.sock");
        drop(UnixListener::bind(&path).unwrap());
        let activation = Activation::bind(path.clone()).unwrap();
        let mut invalid = UnixStream::connect(&path).unwrap();
        invalid.write_all(b"InvalidOpenV1\r\n").unwrap();
        drop(invalid);
        let slow = UnixStream::connect(&path).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(slow);
        request(&path).unwrap();
        drop(activation);
        let target = root.join("keep");
        fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(Activation::bind(path.clone()).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "keep");
        assert!(request_until(&root.join("missing"), Duration::from_millis(50)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
