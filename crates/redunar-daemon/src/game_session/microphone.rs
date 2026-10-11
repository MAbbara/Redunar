//! Session-owned microphone capture and bounded route/transport recovery.
use crate::{MicrophoneMode, ProductionReplayRuntime};
use redunar_capture_audio::{EncodedOpusPacket, MicrophoneAudioCapture, MicrophoneAudioSource};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) fn start(
    stop: Arc<AtomicBool>,
    replay: ProductionReplayRuntime,
    first_frame: u64,
    mode: MicrophoneMode,
) -> Option<JoinHandle<()>> {
    let started = Instant::now();
    let timestamp = redunar_capture_audio::monotonic_now_ns().unwrap_or(first_frame);
    thread::Builder::new()
        .name("redunar-replay-microphone".to_owned())
        .spawn(move || {
            run(&stop, &replay, timestamp, started, &mode);
        })
        .map_err(|error| {
            crate::log_op!("Redunar Replay: could not start microphone worker: {error}");
        })
        .ok()
}

fn discover(mode: &MicrophoneMode) -> Vec<MicrophoneAudioSource> {
    let selected = match mode {
        MicrophoneMode::Off => return Vec::new(),
        MicrophoneMode::DefaultInput => None,
        MicrophoneMode::SelectedInput(name) => Some(name.as_str()),
    };
    redunar_capture_audio::discover_microphone_audio_sources_for(selected)
}

fn run(
    stop: &Arc<AtomicBool>,
    replay: &ProductionReplayRuntime,
    timestamp: u64,
    started: Instant,
    mode: &MicrophoneMode,
) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let route_stop = Arc::clone(stop);
    let route_mode = mode.clone();
    let watcher = thread::Builder::new()
        .name("redunar-microphone-route".to_owned())
        .spawn(move || {
            watch_route(&route_stop, &sender, &route_mode);
        });
    let mut attempt = 0_usize;
    let mut retry_logged = None;
    while !stop.load(Ordering::Acquire) {
        let sources = discover(mode);
        let Some(source) = select_backend(&sources, attempt) else {
            log_retry(&mut retry_logged);
            wait_retry(stop);
            continue;
        };
        let Ok(mut capture) = MicrophoneAudioCapture::start(&source, timestamp, started) else {
            attempt = attempt.wrapping_add(1);
            log_retry(&mut retry_logged);
            wait_retry(stop);
            continue;
        };
        crate::log_op!(
            "Redunar Replay: microphone capture active via {}",
            source.backend_name()
        );
        let result = consume(
            stop,
            &mut capture,
            &receiver,
            &source,
            |packet| {
                // Recorder resets/failures pause ingestion but must not leave a
                // blocked helper pipe or turn audio availability into video authority.
                let _ = replay.submit_microphone_audio(packet);
            },
            Instant::now(),
        );
        // Kill and reap the old helper before starting any replacement or waiting.
        drop(capture);
        if result == CaptureEnd::TransportFailed {
            attempt = attempt.wrapping_add(1);
            log_retry(&mut retry_logged);
        }
        if result != CaptureEnd::Stopped {
            wait_retry(stop);
        }
    }
    if let Ok(watcher) = watcher {
        let _ = watcher.join();
    }
}

fn select_backend(
    sources: &[MicrophoneAudioSource],
    attempt: usize,
) -> Option<MicrophoneAudioSource> {
    if sources.is_empty() {
        None
    } else {
        Some(sources[attempt % sources.len()].clone())
    }
}

#[derive(Debug, Eq, PartialEq)]
enum CaptureEnd {
    Stopped,
    RouteChanged,
    TransportFailed,
}

trait PacketCapture {
    fn next(&mut self) -> Result<Option<EncodedOpusPacket>, ()>;
}

impl PacketCapture for MicrophoneAudioCapture {
    fn next(&mut self) -> Result<Option<EncodedOpusPacket>, ()> {
        self.next_packet(Duration::from_millis(100)).map_err(|_| ())
    }
}

fn consume(
    stop: &AtomicBool,
    capture: &mut impl PacketCapture,
    routes: &Receiver<Vec<MicrophoneAudioSource>>,
    source: &MicrophoneAudioSource,
    mut submit: impl FnMut(EncodedOpusPacket),
    mut last_packet: Instant,
) -> CaptureEnd {
    while !stop.load(Ordering::Acquire) {
        match capture.next() {
            Ok(Some(packet)) => {
                last_packet = Instant::now();
                submit(packet);
            }
            Ok(None) => {
                if last_packet.elapsed() >= super::AUDIO_CAPTURE_STALL_TIMEOUT {
                    return CaptureEnd::TransportFailed;
                }
            }
            Err(()) => return CaptureEnd::TransportFailed,
        }
        // A transient discovery failure is not evidence of a changed input.
        // A removed transport is still recovered by timeout/exit above.
        if routes.try_recv().ok().is_some_and(|updated| {
            updated
                .iter()
                .find(|candidate| candidate.backend_name() == source.backend_name())
                .is_some_and(|candidate| candidate != source)
        }) {
            return CaptureEnd::RouteChanged;
        }
    }
    CaptureEnd::Stopped
}

fn watch_route(
    stop: &AtomicBool,
    sender: &SyncSender<Vec<MicrophoneAudioSource>>,
    mode: &MicrophoneMode,
) {
    while !stop.load(Ordering::Acquire) {
        let deadline = Instant::now() + super::AUDIO_ROUTE_CHECK_INTERVAL;
        while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(100));
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let _ = sender.try_send(discover(mode));
    }
}

fn wait_retry(stop: &AtomicBool) {
    let deadline = Instant::now() + super::AUDIO_RECONNECT_DELAY;
    while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(25));
    }
}

fn log_retry(last: &mut Option<Instant>) {
    if last.is_none_or(|time| time.elapsed() >= Duration::from_secs(5)) {
        crate::log_op!("Redunar Replay: microphone capture unavailable; retrying");
        *last = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeCapture {
        packets: std::collections::VecDeque<Result<Option<EncodedOpusPacket>, ()>>,
    }
    impl PacketCapture for FakeCapture {
        fn next(&mut self) -> Result<Option<EncodedOpusPacket>, ()> {
            self.packets.pop_front().expect("bounded fixture")
        }
    }
    fn source(name: &str) -> MicrophoneAudioSource {
        MicrophoneAudioSource::PulseAudio {
            source_name: name.to_owned(),
        }
    }
    #[test]
    fn failed_native_backend_cycles_to_pulse_and_back() {
        let sources = vec![
            MicrophoneAudioSource::PipeWire(redunar_capture_audio::GameAudioNode {
                serial: 42,
                process_id: 0,
                node_name: "input".to_owned(),
            }),
            source("input"),
        ];
        assert_eq!(select_backend(&sources, 0), Some(sources[0].clone()));
        assert_eq!(select_backend(&sources, 1), Some(sources[1].clone()));
        assert_eq!(select_backend(&sources, 2), Some(sources[0].clone()));
        assert_eq!(select_backend(&[], 0), None);
    }
    #[test]
    fn timeout_route_change_exit_and_stop_end_the_capture() {
        let stop = AtomicBool::new(false);
        let (sender, routes) = mpsc::sync_channel(1);
        let current = source("one");
        let mut capture = FakeCapture {
            packets: [Ok(None)].into(),
        };
        assert_eq!(
            consume(
                &stop,
                &mut capture,
                &routes,
                &current,
                |_| {},
                Instant::now().checked_sub(Duration::from_secs(4)).unwrap()
            ),
            CaptureEnd::TransportFailed
        );
        sender.send(vec![source("two")]).unwrap();
        let mut capture = FakeCapture {
            packets: [Ok(None)].into(),
        };
        assert_eq!(
            consume(
                &stop,
                &mut capture,
                &routes,
                &current,
                |_| {},
                Instant::now()
            ),
            CaptureEnd::RouteChanged
        );
        let mut capture = FakeCapture {
            packets: [Err(())].into(),
        };
        assert_eq!(
            consume(
                &stop,
                &mut capture,
                &routes,
                &current,
                |_| {},
                Instant::now()
            ),
            CaptureEnd::TransportFailed
        );
        stop.store(true, Ordering::Release);
        let mut capture = FakeCapture { packets: [].into() };
        assert_eq!(
            consume(
                &stop,
                &mut capture,
                &routes,
                &current,
                |_| {},
                Instant::now()
            ),
            CaptureEnd::Stopped
        );
    }
    #[test]
    fn packets_continue_after_a_transient_empty_route_check() {
        let stop = AtomicBool::new(false);
        let (sender, routes) = mpsc::sync_channel(1);
        sender.send(Vec::new()).unwrap();
        let packet = EncodedOpusPacket {
            timestamp_ns: 0,
            duration_ns: 20_000_000,
            bytes: vec![1],
        };
        let mut capture = FakeCapture {
            packets: [Ok(Some(packet)), Err(())].into(),
        };
        let mut count = 0;
        assert_eq!(
            consume(
                &stop,
                &mut capture,
                &routes,
                &source("one"),
                |_| {
                    count += 1;
                },
                Instant::now()
            ),
            CaptureEnd::TransportFailed
        );
        assert_eq!(count, 1);
    }
}
