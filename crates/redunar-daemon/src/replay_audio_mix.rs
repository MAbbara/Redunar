//! Save-worker audio mixing. Only encoded snapshots scale with clip length;
//! decoded PCM stays within one frame per input and one output frame.
use crate::{ReplayAudioSnapshot, ReplayAudioTracks};
use redunar_capture_audio::{
    AUDIO_CHANNELS, AUDIO_FRAME_DURATION_NS, AUDIO_FRAME_SAMPLES_PER_CHANNEL, AUDIO_SAMPLE_RATE,
    EncodedOpusPacket, OpusDecoder, OpusEncoder, OpusStreamDescription,
};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

const CHANNELS: usize = AUDIO_CHANNELS as usize;
const FRAME_SAMPLES: usize = AUDIO_FRAME_SAMPLES_PER_CHANNEL * CHANNELS;

pub(crate) fn mix_audio_tracks(
    tracks: ReplayAudioTracks,
    window: (u64, u64),
    cancel: &AtomicBool,
) -> io::Result<ReplayAudioTracks> {
    check_cancel(cancel)?;
    let Some(microphone) = tracks.microphone else {
        return Ok(ReplayAudioTracks {
            system: tracks.system,
            microphone: None,
        });
    };
    if microphone.packets.is_empty() {
        return Ok(ReplayAudioTracks {
            system: tracks.system,
            microphone: None,
        });
    }
    if tracks.system.packets.is_empty() {
        return Ok(ReplayAudioTracks {
            system: microphone,
            microphone: None,
        });
    }
    let mut system = PcmCursor::new(&tracks.system.packets)?;
    let mut input = PcmCursor::new(&microphone.packets)?;
    let mut encoder = OpusEncoder::open().map_err(audio_error)?;
    let mut packets = Vec::new();
    let mut bytes = 0_usize;
    let mut timestamp = window.0;
    while timestamp < window.1 {
        check_cancel(cancel)?;
        let next = system
            .next_timestamp()
            .into_iter()
            .chain(input.next_timestamp())
            .min();
        let Some(next) = next else { break };
        // Keep true gaps when neither input has sound; do not emit minutes of
        // fabricated silence or scan an empty horizon frame by frame.
        timestamp = timestamp.max(next);
        if timestamp >= window.1 {
            break;
        }
        let mut samples = [0_i32; FRAME_SAMPLES];
        system.add_frame(timestamp, &mut samples, cancel)?;
        input.add_frame(timestamp, &mut samples, cancel)?;
        let pcm = samples.map(|value| {
            // Preserve output volume when the microphone is silent. Saturation
            // bounds coincident peaks instead of wrapping signed PCM samples.
            i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)))
                .expect("clamped PCM sample")
        });
        let packet = encoder.encode_frame(timestamp, &pcm).map_err(audio_error)?;
        bytes = bytes.saturating_add(packet.bytes.len());
        if bytes > super::replay_audio::MAX_AUDIO_BUFFER_BYTES
            || packets.len() >= super::replay_audio::MAX_AUDIO_PACKETS
        {
            return Err(io::Error::other(
                "mixed Replay audio exceeds its fixed bound",
            ));
        }
        packets.push(packet);
        timestamp = timestamp.saturating_add(AUDIO_FRAME_DURATION_NS);
    }
    Ok(ReplayAudioTracks {
        system: ReplayAudioSnapshot {
            stream: OpusStreamDescription::default(),
            packets,
        },
        microphone: None,
    })
}

struct PcmFrame {
    timestamp: u64,
    samples: Vec<i16>,
}

struct PcmCursor<'a> {
    packets: std::slice::Iter<'a, EncodedOpusPacket>,
    decoder: OpusDecoder,
    frame: Option<PcmFrame>,
}

impl<'a> PcmCursor<'a> {
    fn new(packets: &'a [EncodedOpusPacket]) -> io::Result<Self> {
        Ok(Self {
            packets: packets.iter(),
            decoder: OpusDecoder::open().map_err(audio_error)?,
            frame: None,
        })
    }

    fn next_timestamp(&self) -> Option<u64> {
        self.frame
            .as_ref()
            .map(|frame| frame.timestamp)
            .or_else(|| {
                self.packets
                    .as_slice()
                    .first()
                    .map(|packet| packet.timestamp_ns)
            })
    }

    fn add_frame(
        &mut self,
        timestamp: u64,
        samples: &mut [i32; FRAME_SAMPLES],
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let end = timestamp.saturating_add(AUDIO_FRAME_DURATION_NS);
        loop {
            check_cancel(cancel)?;
            if self.frame.is_none() {
                if self.next_timestamp().is_none_or(|next| next >= end) {
                    return Ok(());
                }
                let packet = self.packets.next().expect("next timestamp checked");
                let pcm = self.decoder.decode_frame(packet).map_err(audio_error)?;
                if pcm.len() != FRAME_SAMPLES || packet.duration_ns != AUDIO_FRAME_DURATION_NS {
                    return Err(io::Error::other(
                        "Replay audio frame has an invalid duration",
                    ));
                }
                self.frame = Some(PcmFrame {
                    timestamp: packet.timestamp_ns,
                    samples: pcm,
                });
            }
            let frame = self.frame.as_ref().expect("decoded input frame");
            let frame_end = frame.timestamp.saturating_add(AUDIO_FRAME_DURATION_NS);
            if frame.timestamp >= end {
                return Ok(());
            }
            if frame_end > timestamp {
                add_samples(frame, timestamp, samples);
            }
            if frame_end > end {
                return Ok(());
            }
            self.frame = None;
        }
    }
}

fn add_samples(frame: &PcmFrame, timestamp: u64, output: &mut [i32; FRAME_SAMPLES]) {
    let overlap = frame.timestamp.max(timestamp);
    let source_start = sample_offset(overlap.saturating_sub(frame.timestamp));
    let destination_start = sample_offset(overlap.saturating_sub(timestamp));
    let count = FRAME_SAMPLES
        .saturating_sub(source_start)
        .min(FRAME_SAMPLES.saturating_sub(destination_start));
    for (destination, source) in output[destination_start..destination_start + count]
        .iter_mut()
        .zip(&frame.samples[source_start..source_start + count])
    {
        *destination += i32::from(*source);
    }
}

fn sample_offset(nanoseconds: u64) -> usize {
    usize::try_from(nanoseconds.saturating_mul(u64::from(AUDIO_SAMPLE_RATE)) / 1_000_000_000)
        .unwrap_or(FRAME_SAMPLES)
        .min(AUDIO_FRAME_SAMPLES_PER_CHANNEL)
        * CHANNELS
}

fn audio_error(error: redunar_capture_audio::OpusEncoderError) -> io::Error {
    io::Error::other(error)
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Replay audio mixing cancelled",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(times: &[u64], amplitude: i16) -> ReplayAudioSnapshot {
        let mut encoder = OpusEncoder::open().expect("libopus");
        let samples = [amplitude; FRAME_SAMPLES];
        ReplayAudioSnapshot {
            stream: OpusStreamDescription::default(),
            packets: times
                .iter()
                .map(|timestamp| encoder.encode_frame(*timestamp, &samples).unwrap())
                .collect(),
        }
    }

    #[test]
    fn microphone_only_and_system_gaps_preserve_speech() {
        let microphone = snapshot(&[0, 20_000_000, 40_000_000, 60_000_000], 1000);
        let mixed = mix_audio_tracks(
            ReplayAudioTracks {
                system: snapshot(&[], 0),
                microphone: Some(microphone.clone()),
            },
            (0, 80_000_000),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(mixed.system, microphone);
        assert!(mixed.microphone.is_none());
        let mixed = mix_audio_tracks(
            ReplayAudioTracks {
                system: snapshot(&[0, 60_000_000], 0),
                microphone: Some(microphone),
            },
            (0, 80_000_000),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            mixed
                .system
                .packets
                .iter()
                .map(|packet| packet.timestamp_ns)
                .collect::<Vec<_>>(),
            vec![0, 20_000_000, 40_000_000, 60_000_000]
        );
        let mut decoder = OpusDecoder::open().unwrap();
        assert!(mixed.system.packets.iter().all(|packet| {
            decoder
                .decode_frame(packet)
                .unwrap()
                .iter()
                .any(|sample| *sample != 0)
        }));
    }

    #[test]
    fn gaps_in_both_inputs_stay_timestamp_gaps() {
        let mixed = mix_audio_tracks(
            ReplayAudioTracks {
                system: snapshot(&[0, 200_000_000], 1000),
                microphone: Some(snapshot(&[0, 200_000_000], 1000)),
            },
            (0, 220_000_000),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            mixed
                .system
                .packets
                .iter()
                .map(|packet| packet.timestamp_ns)
                .collect::<Vec<_>>(),
            vec![0, 200_000_000]
        );
    }

    #[test]
    fn sample_offsets_preserve_phase_and_do_not_attenuate_silent_input() {
        let frame = PcmFrame {
            timestamp: 10_000_000,
            samples: vec![2000; FRAME_SAMPLES],
        };
        let mut samples = [1000; FRAME_SAMPLES];
        add_samples(&frame, 0, &mut samples);
        assert!(
            samples[..FRAME_SAMPLES / 2]
                .iter()
                .all(|sample| *sample == 1000)
        );
        assert!(
            samples[FRAME_SAMPLES / 2..]
                .iter()
                .all(|sample| *sample == 3000)
        );
        let silence = PcmFrame {
            timestamp: 0,
            samples: vec![0; FRAME_SAMPLES],
        };
        add_samples(&silence, 0, &mut samples);
        assert_eq!(samples[0], 1000);
        let mut tail = [0; FRAME_SAMPLES];
        add_samples(&frame, 20_000_000, &mut tail);
        assert!(
            tail[..FRAME_SAMPLES / 2]
                .iter()
                .all(|sample| *sample == 2000)
        );
        assert!(tail[FRAME_SAMPLES / 2..].iter().all(|sample| *sample == 0));
    }

    #[test]
    fn cancelled_and_malformed_audio_fail_without_a_partial_track() {
        let tracks = ReplayAudioTracks {
            system: snapshot(&[0], 1000),
            microphone: Some(snapshot(&[0], 1000)),
        };
        assert_eq!(
            mix_audio_tracks(tracks.clone(), (0, 20_000_000), &AtomicBool::new(true))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        let mut malformed = tracks;
        malformed.microphone.as_mut().unwrap().packets[0].bytes = vec![0xff];
        assert!(mix_audio_tracks(malformed, (0, 20_000_000), &AtomicBool::new(false)).is_err());
    }
}
