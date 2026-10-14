//! Bounded, same-user keyboard ownership. No blocking reader survives a reconnect.
use super::{EV_KEY, INPUT_EVENT_BYTES, KeyEvent, MAX_KEYBOARDS};
use evdev::Device;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const EVENTS_PER_DEVICE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Identity(u64, u64, u64);

impl Identity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self(metadata.dev(), metadata.ino(), metadata.rdev())
    }
}

pub(crate) fn identity(path: &Path) -> io::Result<Identity> {
    fs::metadata(path).map(|metadata| Identity::from_metadata(&metadata))
}

pub(crate) fn open(path: &Path) -> io::Result<(Identity, File)> {
    let device = Device::open(path)?;
    device.set_nonblocking(true)?;
    let file = File::from(device.as_fd().try_clone_to_owned()?);
    // Identify the opened handle, not a path that could have been replaced.
    Ok((Identity::from_metadata(&file.metadata()?), file))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyboardEvent {
    Key(KeyEvent),
    Removed(usize),
    Availability(usize),
}

struct Keyboard<I> {
    path: PathBuf,
    identity: Identity,
    generation: usize,
    input: I,
}

pub(crate) struct Keyboards<I> {
    devices: Vec<Keyboard<I>>,
    next_generation: usize,
    reported_count: Option<usize>,
}

impl<I: Read> Keyboards<I> {
    pub(crate) fn new() -> Self {
        Self {
            devices: Vec::new(),
            next_generation: 0,
            reported_count: None,
        }
    }

    pub(crate) fn refresh(
        &mut self,
        candidates: &[(PathBuf, Identity)],
        mut open: impl FnMut(&Path) -> io::Result<(Identity, I)>,
    ) -> Vec<KeyboardEvent> {
        let mut events = Vec::new();
        self.devices.retain(|device| {
            let keep = candidates
                .iter()
                .any(|(path, identity)| *path == device.path && *identity == device.identity);
            if !keep {
                events.push(KeyboardEvent::Removed(device.generation));
            }
            keep
        });
        for (path, identity) in candidates {
            if self.devices.len() == MAX_KEYBOARDS {
                break;
            }
            if self.devices.iter().any(|device| device.path == *path) {
                continue;
            }
            let Ok((opened_identity, input)) = open(path) else {
                continue;
            };
            if opened_identity != *identity {
                continue;
            }
            let Some(next) = self.next_generation.checked_add(1) else {
                continue;
            };
            let generation = self.next_generation;
            self.next_generation = next;
            self.devices.push(Keyboard {
                path: path.clone(),
                identity: *identity,
                generation,
                input,
            });
        }
        self.report_availability(&mut events);
        events
    }

    pub(crate) fn poll(&mut self) -> Vec<KeyboardEvent> {
        let mut events = Vec::new();
        self.devices.retain_mut(|device| {
            let mut bytes = [0_u8; INPUT_EVENT_BYTES * EVENTS_PER_DEVICE];
            match device.input.read(&mut bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    true
                }
                Ok(size) if size != 0 && size % INPUT_EVENT_BYTES == 0 => {
                    // SYN_DROPPED means held-key state is no longer trustworthy.
                    // Retire this generation rather than inventing press edges.
                    if bytes[..size].chunks_exact(INPUT_EVENT_BYTES).any(|event| {
                        u16::from_ne_bytes([event[16], event[17]]) == 0
                            && u16::from_ne_bytes([event[18], event[19]]) == 3
                    }) {
                        events.push(KeyboardEvent::Removed(device.generation));
                        return false;
                    }
                    for event in bytes[..size].chunks_exact(INPUT_EVENT_BYTES) {
                        if u16::from_ne_bytes([event[16], event[17]]) == EV_KEY {
                            events.push(KeyboardEvent::Key(KeyEvent {
                                device: device.generation,
                                code: u16::from_ne_bytes([event[18], event[19]]),
                                value: i32::from_ne_bytes([
                                    event[20], event[21], event[22], event[23],
                                ]),
                            }));
                        }
                    }
                    true
                }
                _ => {
                    events.push(KeyboardEvent::Removed(device.generation));
                    false
                }
            }
        });
        self.report_availability(&mut events);
        events
    }

    fn report_availability(&mut self, events: &mut Vec<KeyboardEvent>) {
        let count = self.devices.len();
        if self.reported_count != Some(count) {
            self.reported_count = Some(count);
            events.push(KeyboardEvent::Availability(count));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Input {
        reads: VecDeque<Result<Vec<u8>, io::ErrorKind>>,
        drops: Arc<AtomicUsize>,
    }
    impl Read for Input {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let data = self
                .reads
                .pop_front()
                .unwrap_or(Err(io::ErrorKind::WouldBlock))
                .map_err(io::Error::from)?;
            assert!(data.len() <= bytes.len());
            bytes[..data.len()].copy_from_slice(&data);
            Ok(data.len())
        }
    }
    impl Drop for Input {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn input(reads: Vec<Result<Vec<u8>, io::ErrorKind>>, drops: &Arc<AtomicUsize>) -> Input {
        Input {
            reads: reads.into(),
            drops: drops.clone(),
        }
    }
    fn event(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut bytes = vec![0; INPUT_EVENT_BYTES];
        bytes[16..18].copy_from_slice(&kind.to_ne_bytes());
        bytes[18..20].copy_from_slice(&code.to_ne_bytes());
        bytes[20..24].copy_from_slice(&value.to_ne_bytes());
        bytes
    }
    fn candidate(inode: u64) -> (PathBuf, Identity) {
        (PathBuf::from("fixture-keyboard"), Identity(1, inode, 13))
    }

    #[test]
    fn reconnect_same_path_retires_handle_and_assigns_new_generation() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut keyboards = Keyboards::new();
        assert_eq!(
            keyboards.refresh(&[candidate(1)], |_| Ok((
                candidate(1).1,
                input(vec![Ok(event(EV_KEY, 56, 1))], &drops)
            ))),
            vec![KeyboardEvent::Availability(1)]
        );
        assert_eq!(
            keyboards.poll(),
            vec![KeyboardEvent::Key(KeyEvent {
                device: 0,
                code: 56,
                value: 1
            })]
        );
        assert_eq!(
            keyboards.refresh(&[candidate(2)], |_| Ok((
                candidate(2).1,
                input(vec![Ok(event(EV_KEY, 44, 1))], &drops)
            ))),
            vec![KeyboardEvent::Removed(0)]
        );
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(
            keyboards.poll(),
            vec![KeyboardEvent::Key(KeyEvent {
                device: 1,
                code: 44,
                value: 1
            })]
        );
        assert_eq!(
            keyboards.refresh(&[candidate(2)], |_| panic!("unchanged keyboard reopened")),
            vec![]
        );
        drop(keyboards);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn newly_connected_keyboard_is_added_while_dongle_stays_open() {
        let drops = Arc::new(AtomicUsize::new(0));
        let dongle = (PathBuf::from("fixture-dongle"), Identity(1, 1, 13));
        let keyboard = candidate(2);
        let mut keyboards = Keyboards::new();
        keyboards.refresh(std::slice::from_ref(&dongle), |_| {
            Ok((dongle.1, input(vec![], &drops)))
        });
        let events = keyboards.refresh(&[dongle.clone(), keyboard.clone()], |path| {
            assert_eq!(path, keyboard.0, "existing dongle must retain its handle");
            Ok((keyboard.1, input(vec![Ok(event(EV_KEY, 44, 1))], &drops)))
        });
        assert_eq!(events, vec![KeyboardEvent::Availability(2)]);
        assert_eq!(
            keyboards.poll(),
            vec![KeyboardEvent::Key(KeyEvent {
                device: 1,
                code: 44,
                value: 1
            })]
        );
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(
            keyboards.refresh(std::slice::from_ref(&dongle), |_| panic!()),
            vec![KeyboardEvent::Removed(1), KeyboardEvent::Availability(1)]
        );
        assert_eq!(keyboards.devices[0].generation, 0);
    }

    #[test]
    fn denied_startup_and_lost_access_recover_without_restarting() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut keyboards = Keyboards::new();
        assert_eq!(
            keyboards.refresh(&[candidate(1)], |_| Err(
                io::ErrorKind::PermissionDenied.into()
            )),
            vec![KeyboardEvent::Availability(0)]
        );
        assert!(
            keyboards.refresh(&[], |_| panic!()).is_empty(),
            "unchanged unavailable status is quiet"
        );
        assert_eq!(
            keyboards.refresh(&[candidate(1)], |_| Ok((
                candidate(1).1,
                input(vec![], &drops)
            ))),
            vec![KeyboardEvent::Availability(1)]
        );
        assert_eq!(
            keyboards.refresh(&[], |_| panic!()),
            vec![KeyboardEvent::Removed(0), KeyboardEvent::Availability(0)]
        );
        assert_eq!(
            keyboards.refresh(&[candidate(1)], |_| Ok((
                candidate(1).1,
                input(vec![], &drops)
            ))),
            vec![KeyboardEvent::Availability(1)]
        );
        assert_eq!(keyboards.devices[0].generation, 1);
    }

    #[test]
    fn read_loss_eof_and_dropped_sync_retire_state_then_retry() {
        for failure in [
            Err(io::ErrorKind::NotConnected),
            Ok(vec![]),
            Ok(event(0, 3, 0)),
        ] {
            let drops = Arc::new(AtomicUsize::new(0));
            let mut keyboards = Keyboards::new();
            keyboards.refresh(&[candidate(1)], |_| {
                Ok((candidate(1).1, input(vec![failure.clone()], &drops)))
            });
            assert_eq!(
                keyboards.poll(),
                vec![KeyboardEvent::Removed(0), KeyboardEvent::Availability(0)]
            );
            assert_eq!(drops.load(Ordering::Relaxed), 1);
            keyboards.refresh(&[candidate(1)], |_| {
                Ok((candidate(1).1, input(vec![], &drops)))
            });
            assert_eq!(keyboards.devices[0].generation, 1);
        }
    }

    #[test]
    fn replacement_during_open_is_rejected_and_device_count_is_bounded() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut keyboards = Keyboards::new();
        assert_eq!(
            keyboards.refresh(&[candidate(1)], |_| Ok((
                candidate(2).1,
                input(vec![], &drops)
            ))),
            vec![KeyboardEvent::Availability(0)]
        );
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        let candidates = (0..MAX_KEYBOARDS + 4)
            .map(|n| {
                (
                    PathBuf::from(format!("fixture-{n}")),
                    Identity(1, n as u64, 13),
                )
            })
            .collect::<Vec<_>>();
        let mut opens = 0;
        keyboards.refresh(&candidates, |path| {
            opens += 1;
            Ok((
                candidates.iter().find(|(p, _)| p == path).unwrap().1,
                input(vec![], &drops),
            ))
        });
        assert_eq!(opens, MAX_KEYBOARDS);
        assert_eq!(keyboards.devices.len(), MAX_KEYBOARDS);
        drop(keyboards);
        assert_eq!(drops.load(Ordering::Relaxed), MAX_KEYBOARDS + 1);
    }
}
