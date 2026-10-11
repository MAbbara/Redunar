use redunar_capture_audio::{EncodedOpusPacket, OpusStreamDescription};
use redunar_core::ReplayDuration;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

const MAX_AUDIO_PACKET_BYTES: usize = 4_096;
pub(crate) const MAX_AUDIO_BUFFER_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_AUDIO_PACKETS: usize = 45_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayAudioError {
    InvalidPacket,
    TimestampNotMonotonic,
    BufferLimitExceeded,
}

impl fmt::Display for ReplayAudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPacket => "Replay audio packet is invalid",
            Self::TimestampNotMonotonic => "Replay audio timestamp is not monotonic",
            Self::BufferLimitExceeded => "Replay audio buffer exceeded its fixed limit",
        })
    }
}

impl Error for ReplayAudioError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayAudioSnapshot {
    pub stream: OpusStreamDescription,
    pub packets: Vec<EncodedOpusPacket>,
}

/// Independent optional audio tracks retained for one replay save.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayAudioTracks {
    pub system: ReplayAudioSnapshot,
    pub microphone: Option<ReplayAudioSnapshot>,
}

pub struct ReplayAudioBuffer {
    stream: OpusStreamDescription,
    duration_ns: u64,
    bytes: usize,
    packets: VecDeque<EncodedOpusPacket>,
}

impl ReplayAudioBuffer {
    #[must_use]
    pub fn new(duration: ReplayDuration, stream: OpusStreamDescription) -> Self {
        Self {
            stream,
            duration_ns: u64::from(duration.seconds()).saturating_mul(1_000_000_000),
            bytes: 0,
            packets: VecDeque::new(),
        }
    }

    /// Add one already-encoded packet and evict history older than the saved
    /// Replay duration.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayAudioError`] for malformed/non-monotonic packets or if
    /// fixed packet/byte bounds cannot be preserved.
    pub fn push(&mut self, packet: EncodedOpusPacket) -> Result<(), ReplayAudioError> {
        if packet.bytes.is_empty()
            || packet.bytes.len() > MAX_AUDIO_PACKET_BYTES
            || packet.duration_ns == 0
            || packet
                .timestamp_ns
                .checked_add(packet.duration_ns)
                .is_none()
        {
            return Err(ReplayAudioError::InvalidPacket);
        }
        if self
            .packets
            .back()
            .is_some_and(|last| packet.timestamp_ns <= last.timestamp_ns)
        {
            return Err(ReplayAudioError::TimestampNotMonotonic);
        }
        self.bytes = self
            .bytes
            .checked_add(packet.bytes.len())
            .ok_or(ReplayAudioError::BufferLimitExceeded)?;
        self.packets.push_back(packet);
        self.evict_old();
        if self.bytes > MAX_AUDIO_BUFFER_BYTES || self.packets.len() > MAX_AUDIO_PACKETS {
            if let Some(rejected) = self.packets.pop_back() {
                self.bytes = self.bytes.saturating_sub(rejected.bytes.len());
            }
            return Err(ReplayAudioError::BufferLimitExceeded);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }

    #[must_use]
    pub fn packet_count(&self) -> u64 {
        self.packets.len() as u64
    }

    #[must_use]
    pub fn byte_count(&self) -> u64 {
        self.bytes as u64
    }

    #[must_use]
    pub fn snapshot_between(&self, start_ns: u64, end_ns: u64) -> ReplayAudioSnapshot {
        let packets = self
            .packets
            .iter()
            .filter(|packet| {
                packet.timestamp_ns < end_ns
                    && packet.timestamp_ns.saturating_add(packet.duration_ns) > start_ns
            })
            .cloned()
            .collect();
        ReplayAudioSnapshot {
            stream: self.stream.clone(),
            packets,
        }
    }

    fn evict_old(&mut self) {
        let Some(end) = self
            .packets
            .back()
            .and_then(|packet| packet.timestamp_ns.checked_add(packet.duration_ns))
        else {
            return;
        };
        let keep_from = end.saturating_sub(self.duration_ns);
        while self.packets.front().is_some_and(|packet| {
            packet.timestamp_ns.saturating_add(packet.duration_ns) <= keep_from
        }) {
            if let Some(packet) = self.packets.pop_front() {
                self.bytes = self.bytes.saturating_sub(packet.bytes.len());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(timestamp_ns: u64) -> EncodedOpusPacket {
        EncodedOpusPacket {
            timestamp_ns,
            duration_ns: 20_000_000,
            bytes: vec![1, 2, 3],
        }
    }

    #[test]
    fn duration_eviction_and_snapshot_are_timestamp_bounded() {
        let mut buffer =
            ReplayAudioBuffer::new(ReplayDuration::Seconds15, OpusStreamDescription::default());
        for index in 0..=800 {
            buffer.push(packet(index * 20_000_000)).expect("packet");
        }
        let snapshot = buffer.snapshot_between(15_500_000_000, 16_020_000_000);
        assert_eq!(
            snapshot.packets.first().unwrap().timestamp_ns,
            15_500_000_000
        );
        assert_eq!(
            snapshot.packets.last().unwrap().timestamp_ns,
            16_000_000_000
        );
    }

    #[test]
    fn duplicate_or_oversized_packets_fail_closed() {
        let mut buffer =
            ReplayAudioBuffer::new(ReplayDuration::Seconds15, OpusStreamDescription::default());
        buffer.push(packet(1)).expect("first");
        assert_eq!(
            buffer.push(packet(1)),
            Err(ReplayAudioError::TimestampNotMonotonic)
        );
        let mut oversized = packet(2);
        oversized.bytes = vec![0; MAX_AUDIO_PACKET_BYTES + 1];
        assert_eq!(buffer.push(oversized), Err(ReplayAudioError::InvalidPacket));
    }
}
