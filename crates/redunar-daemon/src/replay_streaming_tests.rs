//! Real container decoding from synthetic encoded input; no GPU or user clips.
use crate::*;
use redunar_core::{ReplayDuration, ReplaySettings};
use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "redunar-streaming-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sparse_focus_transitions_preserve_video_timing_and_continuous_audio() {
    use crate::replay_packet_source::ReplayPacketSource;
    use std::collections::VecDeque;
    struct Source {
        packets: VecDeque<EncodedReplayPacket>,
    }
    impl ReplayPacketSource for Source {
        fn window(&self) -> (u64, u64) {
            (0, 6_000_000_000)
        }
        fn next_packet(&mut self) -> std::io::Result<Option<EncodedReplayPacket>> {
            Ok(self.packets.pop_front())
        }
    }
    let fixture = Fixture::new();
    let (encoded, stream) = encoded_fixture();
    let settings = ReplaySettings::default();
    let store = ReplayClipStore::open(&fixture.0, ReplayBudget::from_settings(settings)).unwrap();
    let timestamps = (0..180_u64)
        .map(|i| i * 1_000_000_000 / 90)
        .chain([2_000_000_000, 3_000_000_000])
        .chain((0..180_u64).map(|i| 4_000_000_000 + i * 1_000_000_000 / 90))
        .collect::<Vec<_>>();
    let packets = timestamps
        .iter()
        .enumerate()
        .map(|(i, &timestamp)| {
            let duration = if i + 1 == timestamps.len() {
                6_000_000_000 - timestamp
            } else {
                6_000_000
            };
            h264_annex_b_access_unit(timestamp, duration, &encoded).unwrap()
        })
        .collect::<VecDeque<_>>();
    let mut audio_encoder = redunar_capture_audio::OpusEncoder::open().unwrap();
    let mut audio = ReplayAudioSnapshot {
        stream: audio_encoder.stream(),
        packets: Vec::new(),
    };
    for index in 0..300_u64 {
        let mut samples = vec![0_i16; 1920];
        if !(100..200).contains(&index) {
            for sample in 0..960 {
                samples[sample * 2..sample * 2 + 2].fill(if sample % 120 < 60 {
                    1000
                } else {
                    -1000
                });
            }
        }
        audio.packets.push(
            audio_encoder
                .encode_frame(index * 20_000_000, &samples)
                .unwrap(),
        );
    }
    for format in [ReplayOutputFormat::Matroska, ReplayOutputFormat::Mp4] {
        let clip = store
            .save_streaming(
                &stream,
                &mut Source {
                    packets: packets.clone(),
                },
                &audio,
                format,
            )
            .unwrap();
        assert_sparse_video_timing(&clip.path, &timestamps, format);
        assert_sparse_audio(&clip.path);
    }
}

fn assert_sparse_video_timing(
    path: &std::path::Path,
    timestamps: &[u64],
    format: ReplayOutputFormat,
) {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(probe.status.success());
    let rows = String::from_utf8(probe.stdout).unwrap();
    let records = rows
        .lines()
        .map(|line| line.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), timestamps.len());
    for (index, &pts) in records.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let expected = timestamps[index] as f64 / 1_000_000_000.0;
        assert!((pts - expected).abs() < 0.0011);
    }
    if format == ReplayOutputFormat::Mp4 {
        // Older FFprobe versions report codec-derived durations for fragmented
        // H.264, even when the container stores different sample durations.
        // Check those stored durations directly; keep the independent demux
        // and audio decode checks above/below on their normal production flags.
        assert_mp4_sample_timing(path, timestamps);
    }
}

fn mp4_boxes(mut bytes: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    std::iter::from_fn(move || {
        if bytes.is_empty() {
            return None;
        }
        assert!(bytes.len() >= 8, "complete fixture box header");
        let size = usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
        assert!((8..=bytes.len()).contains(&size), "bounded fixture box");
        let result = (&bytes[4..8], &bytes[8..size]);
        bytes = &bytes[size..];
        Some(result)
    })
}

fn assert_mp4_sample_timing(path: &std::path::Path, timestamps: &[u64]) {
    let bytes = fs::read(path).unwrap();
    let mut samples = Vec::new();
    for (_, moof) in mp4_boxes(&bytes).filter(|(kind, _)| *kind == b"moof") {
        for (_, traf) in mp4_boxes(moof).filter(|(kind, _)| *kind == b"traf") {
            let boxes = mp4_boxes(traf).collect::<Vec<_>>();
            let find = |kind| boxes.iter().find(|(name, _)| *name == kind).unwrap().1;
            let tfhd = find(b"tfhd".as_slice());
            if u32::from_be_bytes(tfhd[4..8].try_into().unwrap()) != 1 {
                continue; // Audio has its own timescale and decode assertion.
            }
            let tfdt = find(b"tfdt".as_slice());
            assert_eq!(tfdt[0], 1, "64-bit fragment decode time");
            let mut start = u64::from_be_bytes(tfdt[4..12].try_into().unwrap());
            let trun = find(b"trun".as_slice());
            assert_eq!(&trun[..4], &[0, 0, 7, 1], "explicit sample durations");
            let count =
                usize::try_from(u32::from_be_bytes(trun[4..8].try_into().unwrap())).unwrap();
            assert_eq!(trun.len(), 12 + count * 12, "complete sample table");
            for sample in trun[12..].as_chunks::<12>().0 {
                let duration = u64::from(u32::from_be_bytes(sample[..4].try_into().unwrap()));
                samples.push((start, duration));
                start = start.checked_add(duration).unwrap();
            }
        }
    }
    let expected = timestamps
        .iter()
        .zip(timestamps.iter().skip(1).copied().chain([6_000_000_000]))
        .map(|(&start, end)| (start / 1_000, end / 1_000 - start / 1_000))
        .collect::<Vec<_>>();
    assert_eq!(samples, expected, "every stored frame ends at the next PTS");
}

fn assert_sparse_audio(path: &std::path::Path) {
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let samples = decoded
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    assert!(samples.len() > 5 * 48_000);
    let peak = |start, end| {
        samples[start..end]
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max)
    };
    assert!(peak(48_000, 72_000) > 0.001);
    assert!(
        peak(120_000, 168_000) < 0.0001,
        "background audio is silence, not missing timestamps"
    );
    assert!(peak(216_000, 240_000) > 0.001);
}

fn encoded_fixture() -> (Vec<u8>, ReplayVideoStream) {
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x64:r=30",
            "-frames:v",
            "1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-f",
            "h264",
            "pipe:1",
        ])
        .output()
        .expect("FFmpeg fixture dependency");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let headers = H264ParameterSets::from_annex_b(&output.stdout).unwrap();
    let stream = ReplayVideoStream::new(
        ReplayVideoCodec::H264,
        ReplayPacketFormat::H264LengthPrefixed4,
        64,
        64,
        60,
        headers.avc_decoder_configuration(),
    )
    .unwrap();
    (output.stdout, stream)
}

#[test]
fn unrepresentable_mp4_timestamps_leave_no_partial_clip() {
    use crate::replay_packet_source::ReplayPacketSource;
    struct Source(std::collections::VecDeque<EncodedReplayPacket>);
    impl ReplayPacketSource for Source {
        fn window(&self) -> (u64, u64) {
            (0, 6_000_001)
        }
        fn next_packet(&mut self) -> std::io::Result<Option<EncodedReplayPacket>> {
            Ok(self.0.pop_front())
        }
    }
    let fixture = Fixture::new();
    let (encoded, stream) = encoded_fixture();
    let store = ReplayClipStore::open(
        &fixture.0,
        ReplayBudget::from_settings(ReplaySettings::default()),
    )
    .unwrap();
    let packets = [0, 1]
        .into_iter()
        .map(|timestamp| h264_annex_b_access_unit(timestamp, 6_000_000, &encoded).unwrap())
        .collect();
    let audio = ReplayAudioSnapshot {
        stream: redunar_capture_audio::OpusStreamDescription::default(),
        packets: vec![],
    };
    assert!(
        store
            .save_streaming(
                &stream,
                &mut Source(packets),
                &audio,
                ReplayOutputFormat::Mp4
            )
            .is_err()
    );
    assert!(store.inventory().unwrap().is_empty());
    assert_eq!(
        fs::read_dir(&fixture.0).unwrap().count(),
        1,
        "only the ownership marker remains"
    );
}

#[test]
fn streamed_spool_mkv_and_mp4_decode_with_audio_and_cross_cluster_timestamps() {
    let fixture = Fixture::new();
    let (encoded, stream) = encoded_fixture();
    let settings = ReplaySettings {
        duration: ReplayDuration::Seconds60,
        ..ReplaySettings::default()
    };
    let store = ReplayClipStore::open(
        fixture.0.join("clips"),
        ReplayBudget::from_settings(settings),
    )
    .unwrap();
    let mut spool = ReplaySegmentSpool::open(fixture.0.join("spool"), settings).unwrap();
    for index in 0..60 {
        spool
            .try_submit(
                h264_annex_b_access_unit(index * 600_000_000, 600_000_000, &encoded).unwrap(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while spool.stats().accepted_packets < index + 1 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    let audio = ReplayAudioSnapshot {
        stream: redunar_capture_audio::OpusStreamDescription::default(),
        packets: (0..1800)
            .filter(|index| !(100..150).contains(index))
            .map(|index| redunar_capture_audio::EncodedOpusPacket {
                timestamp_ns: index * 20_000_000,
                duration_ns: 20_000_000,
                bytes: vec![0xf8, 0xff, 0xfe],
            })
            .collect(),
    };
    for format in [ReplayOutputFormat::Matroska, ReplayOutputFormat::Mp4] {
        let clip = spool
            .save_async_as(
                ReplayDuration::Seconds60,
                settings,
                stream.clone(),
                store.clone(),
                format,
                audio.clone(),
            )
            .unwrap()
            .join()
            .unwrap();
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_entries",
                "stream=codec_name,nb_read_frames:format=duration",
                "-of",
                "default=nw=1",
            ])
            .arg(&clip.path)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let output = String::from_utf8(probe.stdout).unwrap();
        assert!(
            output.contains("codec_name=h264")
                && output.contains("codec_name=opus")
                && output.contains("nb_read_frames=60"),
            "{output}"
        );
        let duration = output
            .lines()
            .find_map(|line| line.strip_prefix("duration="))
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!((duration - 36.0).abs() < 0.1, "{output}");
        let decode = Command::new("ffmpeg")
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&clip.path)
            .args(["-f", "null", "-"])
            .output()
            .unwrap();
        assert!(
            decode.status.success(),
            "{}",
            String::from_utf8_lossy(&decode.stderr)
        );
    }
    spool.shutdown();
}

#[test]
fn saved_mixed_and_microphone_only_clips_decode_as_one_audible_track() {
    let fixture = Fixture::new();
    let (encoded, stream) = encoded_fixture();
    let settings = ReplaySettings::default();
    let store = ReplayClipStore::open(
        fixture.0.join("clips"),
        ReplayBudget::from_settings(settings),
    )
    .unwrap();
    let mut spool = ReplaySegmentSpool::open(fixture.0.join("spool"), settings).unwrap();
    for index in 0..20 {
        spool
            .try_submit(
                h264_annex_b_access_unit(index * 100_000_000, 100_000_000, &encoded).unwrap(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while spool.stats().accepted_packets < index + 1 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    for system in [
        mixed_audio_fixture(true),
        ReplayAudioSnapshot {
            stream: redunar_capture_audio::OpusStreamDescription::default(),
            packets: vec![],
        },
    ] {
        for format in [ReplayOutputFormat::Matroska, ReplayOutputFormat::Mp4] {
            let clip = spool
                .save_async_tracks_as(
                    ReplayDuration::Seconds15,
                    settings,
                    stream.clone(),
                    store.clone(),
                    format,
                    ReplayAudioTracks {
                        system: system.clone(),
                        microphone: Some(mixed_audio_fixture(false)),
                    },
                )
                .unwrap()
                .join()
                .unwrap();
            assert_mixed_audio(&clip.path);
        }
    }
    let before = store.inventory().unwrap();
    let malformed = ReplayAudioSnapshot {
        stream: redunar_capture_audio::OpusStreamDescription::default(),
        packets: vec![redunar_capture_audio::EncodedOpusPacket {
            timestamp_ns: 0,
            duration_ns: 20_000_000,
            bytes: vec![0xff],
        }],
    };
    assert!(
        spool
            .save_async_tracks_as(
                ReplayDuration::Seconds15,
                settings,
                stream,
                store.clone(),
                ReplayOutputFormat::Matroska,
                ReplayAudioTracks {
                    system: mixed_audio_fixture(false),
                    microphone: Some(malformed)
                }
            )
            .unwrap()
            .join()
            .is_err()
    );
    assert_eq!(
        store.inventory().unwrap().len(),
        before.len(),
        "failed mixing preserves committed clips"
    );
    spool.shutdown();
}

fn mixed_audio_fixture(missing_system: bool) -> ReplayAudioSnapshot {
    let mut opus = redunar_capture_audio::OpusEncoder::open().unwrap();
    ReplayAudioSnapshot {
        stream: opus.stream(),
        packets: (0..100)
            .filter(|index| !missing_system || !(25..75).contains(index))
            .map(|index| {
                let pcm = std::array::from_fn::<_, 1920, _>(|sample| {
                    if sample / 2 % 120 < 60 { 2000 } else { -2000 }
                });
                opus.encode_frame(index * 20_000_000, &pcm).unwrap()
            })
            .collect(),
    }
}

fn assert_mixed_audio(path: &std::path::Path) {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let codecs = String::from_utf8(probe.stdout).unwrap();
    assert_eq!(
        codecs.lines().filter(|name| *name == "opus").count(),
        1,
        "{codecs}"
    );
    let decode = Command::new("ffmpeg")
        .args(["-v", "error", "-xerror", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        decode.status.success(),
        "{}",
        String::from_utf8_lossy(&decode.stderr)
    );
    let samples = decode
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(f32::from_le_bytes)
        .collect::<Vec<_>>();
    assert!(samples.len() >= 95_000, "full clip tail remains audible");
    let middle_peak = samples[48_000..60_000]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    assert!(
        middle_peak > 0.01,
        "microphone remains audible inside the system gap: {middle_peak}"
    );
}

#[test]
fn failed_streaming_read_discards_temporary_output_and_keeps_existing_clips() {
    use crate::replay_packet_source::ReplayPacketSource;
    struct Broken {
        packet: Option<EncodedReplayPacket>,
    }
    impl ReplayPacketSource for Broken {
        fn window(&self) -> (u64, u64) {
            (0, 2_000_000_000)
        }
        fn next_packet(&mut self) -> std::io::Result<Option<EncodedReplayPacket>> {
            if let Some(packet) = self.packet.take() {
                Ok(Some(packet))
            } else {
                Err(std::io::Error::other("fixture read failed"))
            }
        }
    }
    let fixture = Fixture::new();
    let (encoded, stream) = encoded_fixture();
    let store = ReplayClipStore::open(
        &fixture.0,
        ReplayBudget::from_settings(ReplaySettings::default()),
    )
    .unwrap();
    let audio = ReplayAudioSnapshot {
        stream: redunar_capture_audio::OpusStreamDescription::default(),
        packets: vec![],
    };
    let mut original = ReplayRing::new(ReplaySettings::default());
    original
        .push(h264_annex_b_access_unit(0, 1_000_000_000, &encoded).unwrap())
        .unwrap();
    let existing = store.save_video_only(&stream, &original).unwrap();
    let preserved = fs::read(&existing.path).unwrap();
    for format in [ReplayOutputFormat::Matroska, ReplayOutputFormat::Mp4] {
        let mut source = Broken {
            packet: Some(h264_annex_b_access_unit(0, 1_000_000_000, &encoded).unwrap()),
        };
        assert!(
            store
                .save_streaming(&stream, &mut source, &audio, format)
                .is_err()
        );
        assert_eq!(store.inventory().unwrap().len(), 1);
        assert_eq!(fs::read(&existing.path).unwrap(), preserved);
        assert_eq!(
            fs::read_dir(&fixture.0).unwrap().count(),
            2,
            "only the existing clip and ownership marker remain"
        );
    }
}
