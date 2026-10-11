use crate::{
    EncodedReplayPacket, ReplayAudioSnapshot, ReplayAudioTracks, ReplayRing, ReplayVideoStream,
};
use redunar_capture_audio::EncodedOpusPacket;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};

const TIMESTAMP_SCALE_NS: u64 = 1_000_000;
const MAX_CLUSTER_DURATION_MS: u64 = 30_000;
const MAX_EBML_SIZE: u64 = (1_u64 << 56) - 2;

const ID_EBML: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3];
const ID_SEGMENT: &[u8] = &[0x18, 0x53, 0x80, 0x67];
const ID_INFO: &[u8] = &[0x15, 0x49, 0xa9, 0x66];
const ID_DURATION: &[u8] = &[0x44, 0x89];
const ID_TRACKS: &[u8] = &[0x16, 0x54, 0xae, 0x6b];
const ID_CLUSTER: &[u8] = &[0x1f, 0x43, 0xb6, 0x75];
const ID_SIMPLE_BLOCK: &[u8] = &[0xa3];

/// Unknown-sized clusters permit ordered blocks without buffering payloads.
/// A new level-one Cluster closes the previous one, just as the unknown-sized
/// Segment is terminated by EOF. Timing and codec validation remain explicit.
#[cfg(test)]
pub(crate) fn write_streaming_matroska(
    destination: &mut impl Write,
    stream: &ReplayVideoStream,
    source: &mut impl crate::replay_packet_source::ReplayPacketSource,
    audio: &ReplayAudioSnapshot,
) -> Result<u64, MatroskaVideoError> {
    let tracks = ReplayAudioTracks {
        system: audio.clone(),
        microphone: None,
    };
    write_streaming_matroska_tracks(destination, stream, source, &tracks)
}

pub(crate) fn write_streaming_matroska_tracks(
    destination: &mut impl Write,
    stream: &ReplayVideoStream,
    source: &mut impl crate::replay_packet_source::ReplayPacketSource,
    audio: &ReplayAudioTracks,
) -> Result<u64, MatroskaVideoError> {
    let (origin, end) = source.window();
    // Merge bounded packet references before writing either track. Appending
    // the microphone after video can move a cluster backwards in time.
    let mut audio_packets = audio
        .system
        .packets
        .iter()
        .map(|packet| (packet, 2))
        .chain(
            audio
                .microphone
                .iter()
                .flat_map(|microphone| microphone.packets.iter().map(|packet| (packet, 3))),
        )
        .filter(|(packet, _)| packet.timestamp_ns >= origin && packet.timestamp_ns < end)
        .collect::<Vec<_>>();
    audio_packets.sort_by_key(|(packet, _)| packet.timestamp_ns);
    let mut output = CountingWriter::new(destination);
    write_ebml_header(&mut output)?;
    output.write_all(ID_SEGMENT)?;
    output.write_all(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])?;
    write_info(
        &mut output,
        end.checked_sub(origin)
            .ok_or(MatroskaVideoError::TimestampOverflow)?,
    )?;
    if audio_packets.is_empty() {
        write_tracks(&mut output, stream)?;
    } else {
        write_audio_video_tracks(
            &mut output,
            stream,
            &audio.system,
            audio.microphone.as_ref(),
        )?;
    }
    let mut cluster = None;
    let mut next_audio = 0;
    let mut first = true;
    while let Some(packet) = source.next_packet()? {
        if first && !packet.is_keyframe() {
            return Err(MatroskaVideoError::MissingInitialKeyframe);
        }
        first = false;
        stream
            .validate_packet(packet.bytes())
            .map_err(|_| MatroskaVideoError::InvalidPacket)?;
        while next_audio < audio_packets.len()
            && audio_packets[next_audio].0.timestamp_ns < packet.timestamp_ns()
        {
            write_streaming_block(
                &mut output,
                MatroskaBlock::Audio(audio_packets[next_audio].0, audio_packets[next_audio].1),
                origin,
                &mut cluster,
            )?;
            next_audio += 1;
        }
        write_streaming_block(
            &mut output,
            MatroskaBlock::Video(&packet),
            origin,
            &mut cluster,
        )?;
    }
    if first {
        return Err(MatroskaVideoError::EmptyRing);
    }
    for packet in &audio_packets[next_audio..] {
        source.check_cancel()?;
        write_streaming_block(
            &mut output,
            MatroskaBlock::Audio(packet.0, packet.1),
            origin,
            &mut cluster,
        )?;
    }
    Ok(output.written)
}

fn invalid_audio_packet(packet: &EncodedOpusPacket) -> bool {
    packet.bytes.is_empty()
        || packet.bytes.len() > 4096
        || packet.duration_ns == 0
        || packet
            .timestamp_ns
            .checked_add(packet.duration_ns)
            .is_none()
}

fn write_streaming_block(
    output: &mut impl Write,
    block: MatroskaBlock<'_>,
    origin: u64,
    cluster: &mut Option<u64>,
) -> Result<(), MatroskaVideoError> {
    if let MatroskaBlock::Audio(packet, _) = block
        && invalid_audio_packet(packet)
    {
        return Err(MatroskaVideoError::InvalidPacket);
    }
    let timestamp = relative_milliseconds(block.timestamp_ns(), origin)?;
    if cluster.is_none_or(|start| timestamp.saturating_sub(start) > MAX_CLUSTER_DURATION_MS) {
        output.write_all(ID_CLUSTER)?;
        output.write_all(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])?;
        write_unsigned(output, ID_TIMESTAMP, timestamp)?;
        *cluster = Some(timestamp);
    }
    let relative = timestamp
        .checked_sub(cluster.unwrap_or(timestamp))
        .and_then(|value| i16::try_from(value).ok())
        .ok_or(MatroskaVideoError::TimestampOverflow)?;
    write_master_header(output, ID_SIMPLE_BLOCK, 4 + block.bytes().len() as u64)?;
    output.write_all(&[block.track_byte()])?;
    output.write_all(&relative.to_be_bytes())?;
    output.write_all(&[block.flags()])?;
    output.write_all(block.bytes())?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatroskaVideoSummary {
    pub packets: u32,
    pub clusters: u32,
    pub duration_ns: u64,
    pub bytes_written: u64,
}

#[derive(Debug)]
pub enum MatroskaVideoError {
    EmptyRing,
    MissingInitialKeyframe,
    InvalidPacket,
    TimestampOverflow,
    ContainerTooLarge,
    Io(io::Error),
}

impl fmt::Display for MatroskaVideoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRing => formatter.write_str("replay ring contains no encoded video"),
            Self::MissingInitialKeyframe => {
                formatter.write_str("replay clip does not start at a keyframe")
            }
            Self::InvalidPacket => formatter.write_str("replay packet cannot be muxed"),
            Self::TimestampOverflow => formatter.write_str("replay clip timestamp overflows"),
            Self::ContainerTooLarge => formatter.write_str("replay container exceeds EBML limits"),
            Self::Io(error) => write!(formatter, "could not write replay container: {error}"),
        }
    }
}

impl Error for MatroskaVideoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for MatroskaVideoError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Write a seek-free, video-only Matroska stream from one bounded codec epoch.
///
/// The Segment uses an unknown size so the writer works directly with the
/// private atomic clip temporary file and does not duplicate the rolling ring
/// in memory. Clusters are capped at 30 seconds and signed block timecodes are
/// checked before any payload is written.
///
/// # Errors
///
/// Returns [`MatroskaVideoError`] for empty/undecodable history, invalid packet
/// framing, timestamp overflow, EBML size overflow, or output failures.
pub fn write_video_only_matroska(
    destination: &mut impl Write,
    stream: &ReplayVideoStream,
    ring: &ReplayRing,
) -> Result<MatroskaVideoSummary, MatroskaVideoError> {
    let packets: Vec<&EncodedReplayPacket> = ring.packets().collect();
    let first = packets.first().ok_or(MatroskaVideoError::EmptyRing)?;
    if !first.is_keyframe() {
        return Err(MatroskaVideoError::MissingInitialKeyframe);
    }
    for packet in &packets {
        stream
            .validate_packet(packet.bytes())
            .map_err(|_| MatroskaVideoError::InvalidPacket)?;
    }

    let last = packets.last().ok_or(MatroskaVideoError::EmptyRing)?;
    let duration_ns = last
        .timestamp_ns()
        .checked_add(last.duration_ns())
        .and_then(|end| end.checked_sub(first.timestamp_ns()))
        .ok_or(MatroskaVideoError::TimestampOverflow)?;

    let mut output = CountingWriter::new(destination);
    write_ebml_header(&mut output)?;
    output.write_all(ID_SEGMENT)?;
    output.write_all(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])?;
    write_info(&mut output, duration_ns)?;
    write_tracks(&mut output, stream)?;

    let origin_ns = first.timestamp_ns();
    let mut clusters = 0_u32;
    let mut index = 0_usize;
    while index < packets.len() {
        let cluster_timestamp_ms = relative_milliseconds(packets[index].timestamp_ns(), origin_ns)?;
        let end = cluster_end(&packets, index, origin_ns, cluster_timestamp_ms)?;
        write_cluster(
            &mut output,
            &packets[index..end],
            origin_ns,
            cluster_timestamp_ms,
        )?;
        clusters = clusters.saturating_add(1);
        index = end;
    }

    Ok(MatroskaVideoSummary {
        packets: u32::try_from(packets.len()).unwrap_or(u32::MAX),
        clusters,
        duration_ns,
        bytes_written: output.written,
    })
}

/// Write a seek-free Matroska stream with H.264 video and synchronized Opus
/// game audio. Audio outside the selected video epoch is ignored.
///
/// # Errors
///
/// Returns [`MatroskaVideoError`] under the same bounds as
/// [`write_video_only_matroska`], including malformed audio packets or timing.
pub fn write_audio_video_matroska(
    destination: &mut impl Write,
    stream: &ReplayVideoStream,
    ring: &ReplayRing,
    audio: &ReplayAudioSnapshot,
) -> Result<MatroskaVideoSummary, MatroskaVideoError> {
    let tracks = ReplayAudioTracks {
        system: audio.clone(),
        microphone: None,
    };
    write_audio_video_matroska_tracks(destination, stream, ring, &tracks)
}

/// Write a Matroska stream with optional independent microphone audio.
///
/// # Errors
/// Returns an error on invalid media, timestamp overflow or output failure.
pub fn write_audio_video_matroska_tracks(
    destination: &mut impl Write,
    stream: &ReplayVideoStream,
    ring: &ReplayRing,
    audio: &ReplayAudioTracks,
) -> Result<MatroskaVideoSummary, MatroskaVideoError> {
    let video = ring.packets().collect::<Vec<_>>();
    let first = video.first().ok_or(MatroskaVideoError::EmptyRing)?;
    if !first.is_keyframe() {
        return Err(MatroskaVideoError::MissingInitialKeyframe);
    }
    for packet in &video {
        stream
            .validate_packet(packet.bytes())
            .map_err(|_| MatroskaVideoError::InvalidPacket)?;
    }
    let video_end = video
        .last()
        .and_then(|packet| packet.timestamp_ns().checked_add(packet.duration_ns()))
        .ok_or(MatroskaVideoError::TimestampOverflow)?;
    let origin_ns = first.timestamp_ns();
    let audio_packets = audio
        .system
        .packets
        .iter()
        .filter(|packet| packet.timestamp_ns >= origin_ns && packet.timestamp_ns < video_end)
        .collect::<Vec<_>>();
    let microphone_packets = audio
        .microphone
        .as_ref()
        .map(|microphone| {
            microphone
                .packets
                .iter()
                .filter(|packet| {
                    packet.timestamp_ns >= origin_ns && packet.timestamp_ns < video_end
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if audio_packets
        .iter()
        .chain(microphone_packets.iter())
        .copied()
        .any(invalid_audio_packet)
    {
        return Err(MatroskaVideoError::InvalidPacket);
    }

    let mut blocks = video
        .iter()
        .copied()
        .map(MatroskaBlock::Video)
        .chain(
            audio_packets
                .into_iter()
                .map(|packet| MatroskaBlock::Audio(packet, 2)),
        )
        .chain(
            microphone_packets
                .into_iter()
                .map(|packet| MatroskaBlock::Audio(packet, 3)),
        )
        .collect::<Vec<_>>();
    blocks.sort_by_key(MatroskaBlock::timestamp_ns);

    let mut output = CountingWriter::new(destination);
    write_ebml_header(&mut output)?;
    output.write_all(ID_SEGMENT)?;
    output.write_all(&[0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])?;
    write_info(&mut output, video_end.saturating_sub(origin_ns))?;
    write_audio_video_tracks(
        &mut output,
        stream,
        &audio.system,
        audio.microphone.as_ref(),
    )?;
    let mut clusters = 0_u32;
    let mut start = 0_usize;
    while start < blocks.len() {
        let cluster_timestamp_ms = relative_milliseconds(blocks[start].timestamp_ns(), origin_ns)?;
        let mut end = start + 1;
        while end < blocks.len()
            && relative_milliseconds(blocks[end].timestamp_ns(), origin_ns)?
                .saturating_sub(cluster_timestamp_ms)
                <= MAX_CLUSTER_DURATION_MS
        {
            end += 1;
        }
        write_av_cluster(
            &mut output,
            &blocks[start..end],
            origin_ns,
            cluster_timestamp_ms,
        )?;
        clusters = clusters.saturating_add(1);
        start = end;
    }
    Ok(MatroskaVideoSummary {
        packets: u32::try_from(video.len()).unwrap_or(u32::MAX),
        clusters,
        duration_ns: video_end.saturating_sub(origin_ns),
        bytes_written: output.written,
    })
}

#[derive(Clone, Copy)]
enum MatroskaBlock<'a> {
    Video(&'a EncodedReplayPacket),
    Audio(&'a EncodedOpusPacket, u8),
}

impl MatroskaBlock<'_> {
    fn timestamp_ns(&self) -> u64 {
        match *self {
            Self::Video(packet) => packet.timestamp_ns(),
            Self::Audio(packet, _) => packet.timestamp_ns,
        }
    }

    fn bytes(&self) -> &[u8] {
        match *self {
            Self::Video(packet) => packet.bytes(),
            Self::Audio(packet, _) => &packet.bytes,
        }
    }

    const fn track_byte(&self) -> u8 {
        match self {
            Self::Video(_) => 0x81,
            Self::Audio(_, track) => 0x80 | *track,
        }
    }

    const fn flags(&self) -> u8 {
        match self {
            Self::Video(packet) if packet.is_keyframe() => 0x80,
            Self::Audio(_, _) => 0x80,
            Self::Video(_) => 0,
        }
    }
}

fn write_audio_video_tracks(
    output: &mut impl Write,
    stream: &ReplayVideoStream,
    audio: &ReplayAudioSnapshot,
    microphone: Option<&ReplayAudioSnapshot>,
) -> Result<(), MatroskaVideoError> {
    let mut video = Vec::new();
    append_unsigned(&mut video, &[0xb0], u64::from(stream.width()))?;
    append_unsigned(&mut video, &[0xba], u64::from(stream.height()))?;
    let mut video_track = Vec::new();
    append_unsigned(&mut video_track, &[0xd7], 1)?;
    append_unsigned(&mut video_track, &[0x73, 0xc5], 1)?;
    append_unsigned(&mut video_track, &[0x83], 1)?;
    append_unsigned(&mut video_track, &[0x9c], 0)?;
    append_bytes(
        &mut video_track,
        &[0x86],
        stream.codec().matroska_codec_id(),
    )?;
    append_bytes(&mut video_track, &[0x63, 0xa2], stream.codec_private())?;
    append_master(&mut video_track, &[0xe0], &video)?;

    let mut audio_settings = Vec::new();
    append_float(
        &mut audio_settings,
        &[0xb5],
        f64::from(audio.stream.sample_rate),
    )?;
    append_unsigned(
        &mut audio_settings,
        &[0x9f],
        u64::from(audio.stream.channels),
    )?;
    let mut audio_track = Vec::new();
    append_unsigned(&mut audio_track, &[0xd7], 2)?;
    append_unsigned(&mut audio_track, &[0x73, 0xc5], 2)?;
    append_bytes(&mut audio_track, &[0x53, 0x6e], b"System audio")?;
    append_unsigned(&mut audio_track, &[0x83], 2)?;
    append_unsigned(&mut audio_track, &[0x9c], 0)?;
    append_unsigned(&mut audio_track, &[0x56, 0xaa], 6_500_000)?;
    append_unsigned(&mut audio_track, &[0x56, 0xbb], 80_000_000)?;
    append_bytes(&mut audio_track, &[0x86], b"A_OPUS")?;
    append_bytes(&mut audio_track, &[0x63, 0xa2], &audio.stream.codec_private)?;
    append_master(&mut audio_track, &[0xe1], &audio_settings)?;

    let mut tracks = Vec::new();
    append_master(&mut tracks, &[0xae], &video_track)?;
    append_master(&mut tracks, &[0xae], &audio_track)?;
    if let Some(microphone) = microphone {
        let mut settings = Vec::new();
        append_float(
            &mut settings,
            &[0xb5],
            f64::from(microphone.stream.sample_rate),
        )?;
        append_unsigned(
            &mut settings,
            &[0x9f],
            u64::from(microphone.stream.channels),
        )?;
        let mut track = Vec::new();
        append_unsigned(&mut track, &[0xd7], 3)?;
        append_unsigned(&mut track, &[0x73, 0xc5], 3)?;
        append_bytes(&mut track, &[0x53, 0x6e], b"Microphone")?;
        append_unsigned(&mut track, &[0x83], 2)?;
        append_unsigned(&mut track, &[0x9c], 0)?;
        append_unsigned(&mut track, &[0x56, 0xaa], 6_500_000)?;
        append_unsigned(&mut track, &[0x56, 0xbb], 80_000_000)?;
        append_bytes(&mut track, &[0x86], b"A_OPUS")?;
        append_bytes(&mut track, &[0x63, 0xa2], &microphone.stream.codec_private)?;
        append_master(&mut track, &[0xe1], &settings)?;
        append_master(&mut tracks, &[0xae], &track)?;
    }
    write_element(output, ID_TRACKS, &tracks)
}

fn write_av_cluster(
    output: &mut impl Write,
    blocks: &[MatroskaBlock<'_>],
    origin_ns: u64,
    cluster_timestamp_ms: u64,
) -> Result<(), MatroskaVideoError> {
    let content_size = blocks.iter().try_fold(
        element_size(ID_TIMESTAMP, unsigned_width(cluster_timestamp_ms)),
        |total, block| {
            let payload =
                4_u64.saturating_add(u64::try_from(block.bytes().len()).unwrap_or(u64::MAX));
            total
                .checked_add(element_size(ID_SIMPLE_BLOCK, payload))
                .ok_or(MatroskaVideoError::ContainerTooLarge)
        },
    )?;
    write_master_header(output, ID_CLUSTER, content_size)?;
    write_unsigned(output, ID_TIMESTAMP, cluster_timestamp_ms)?;
    for block in blocks {
        let relative = relative_milliseconds(block.timestamp_ns(), origin_ns)?
            .checked_sub(cluster_timestamp_ms)
            .and_then(|value| i16::try_from(value).ok())
            .ok_or(MatroskaVideoError::TimestampOverflow)?;
        write_master_header(
            output,
            ID_SIMPLE_BLOCK,
            4_u64.saturating_add(u64::try_from(block.bytes().len()).unwrap_or(u64::MAX)),
        )?;
        output.write_all(&[block.track_byte()])?;
        output.write_all(&relative.to_be_bytes())?;
        output.write_all(&[block.flags()])?;
        output.write_all(block.bytes())?;
    }
    Ok(())
}

fn write_ebml_header(output: &mut impl Write) -> Result<(), MatroskaVideoError> {
    let mut contents = Vec::with_capacity(40);
    append_unsigned(&mut contents, &[0x42, 0x86], 1)?;
    append_unsigned(&mut contents, &[0x42, 0xf7], 1)?;
    append_unsigned(&mut contents, &[0x42, 0xf2], 4)?;
    append_unsigned(&mut contents, &[0x42, 0xf3], 8)?;
    append_bytes(&mut contents, &[0x42, 0x82], b"matroska")?;
    append_unsigned(&mut contents, &[0x42, 0x87], 4)?;
    append_unsigned(&mut contents, &[0x42, 0x85], 2)?;
    write_element(output, ID_EBML, &contents)
}

fn write_info(output: &mut impl Write, duration_ns: u64) -> Result<(), MatroskaVideoError> {
    let mut contents = Vec::with_capacity(64);
    append_unsigned(&mut contents, &[0x2a, 0xd7, 0xb1], TIMESTAMP_SCALE_NS)?;
    // Matroska stores Duration as a floating-point value. The bounded replay
    // durations fit well within f64's practical precision for this metadata.
    #[allow(clippy::cast_precision_loss)]
    let duration = duration_ns as f64 / TIMESTAMP_SCALE_NS as f64;
    append_float(&mut contents, ID_DURATION, duration)?;
    append_bytes(&mut contents, &[0x4d, 0x80], b"Redunar")?;
    append_bytes(&mut contents, &[0x57, 0x41], b"Redunar")?;
    write_element(output, ID_INFO, &contents)
}

fn write_tracks(
    output: &mut impl Write,
    stream: &ReplayVideoStream,
) -> Result<(), MatroskaVideoError> {
    let mut video = Vec::with_capacity(24);
    append_unsigned(&mut video, &[0xb0], u64::from(stream.width()))?;
    append_unsigned(&mut video, &[0xba], u64::from(stream.height()))?;

    let mut track = Vec::with_capacity(128 + stream.codec_private().len());
    append_unsigned(&mut track, &[0xd7], 1)?;
    append_unsigned(&mut track, &[0x73, 0xc5], 1)?;
    append_unsigned(&mut track, &[0x83], 1)?;
    append_unsigned(&mut track, &[0x9c], 0)?;
    append_unsigned(
        &mut track,
        &[0x23, 0xe3, 0x83],
        1_000_000_000_u64 / u64::from(stream.frames_per_second()),
    )?;
    append_bytes(&mut track, &[0x86], stream.codec().matroska_codec_id())?;
    if !stream.codec_private().is_empty() {
        append_bytes(&mut track, &[0x63, 0xa2], stream.codec_private())?;
    }
    append_master(&mut track, &[0xe0], &video)?;

    let mut tracks = Vec::with_capacity(track.len() + 16);
    append_master(&mut tracks, &[0xae], &track)?;
    write_element(output, ID_TRACKS, &tracks)
}

fn cluster_end(
    packets: &[&EncodedReplayPacket],
    start: usize,
    origin_ns: u64,
    cluster_timestamp_ms: u64,
) -> Result<usize, MatroskaVideoError> {
    let mut end = start + 1;
    while end < packets.len() {
        let timestamp_ms = relative_milliseconds(packets[end].timestamp_ns(), origin_ns)?;
        let relative = timestamp_ms
            .checked_sub(cluster_timestamp_ms)
            .ok_or(MatroskaVideoError::TimestampOverflow)?;
        if relative > MAX_CLUSTER_DURATION_MS || relative > i16::MAX as u64 {
            break;
        }
        end += 1;
    }
    Ok(end)
}

fn write_cluster(
    output: &mut impl Write,
    packets: &[&EncodedReplayPacket],
    origin_ns: u64,
    cluster_timestamp_ms: u64,
) -> Result<(), MatroskaVideoError> {
    let timestamp_size = element_size(ID_TIMESTAMP, unsigned_width(cluster_timestamp_ms));
    let content_size = packets.iter().try_fold(timestamp_size, |total, packet| {
        let block_payload = 4_u64
            .checked_add(u64::try_from(packet.bytes().len()).unwrap_or(u64::MAX))
            .ok_or(MatroskaVideoError::ContainerTooLarge)?;
        total
            .checked_add(element_size(ID_SIMPLE_BLOCK, block_payload))
            .ok_or(MatroskaVideoError::ContainerTooLarge)
    })?;
    write_master_header(output, ID_CLUSTER, content_size)?;
    write_unsigned(output, ID_TIMESTAMP, cluster_timestamp_ms)?;
    for packet in packets {
        let timestamp_ms = relative_milliseconds(packet.timestamp_ns(), origin_ns)?;
        let relative = timestamp_ms
            .checked_sub(cluster_timestamp_ms)
            .and_then(|value| i16::try_from(value).ok())
            .ok_or(MatroskaVideoError::TimestampOverflow)?;
        let payload_size = 4_u64
            .checked_add(u64::try_from(packet.bytes().len()).unwrap_or(u64::MAX))
            .ok_or(MatroskaVideoError::ContainerTooLarge)?;
        write_master_header(output, ID_SIMPLE_BLOCK, payload_size)?;
        output.write_all(&[0x81])?;
        output.write_all(&relative.to_be_bytes())?;
        output.write_all(&[if packet.is_keyframe() { 0x80 } else { 0 }])?;
        output.write_all(packet.bytes())?;
    }
    Ok(())
}

const ID_TIMESTAMP: &[u8] = &[0xe7];

fn relative_milliseconds(timestamp_ns: u64, origin_ns: u64) -> Result<u64, MatroskaVideoError> {
    timestamp_ns
        .checked_sub(origin_ns)
        .map(|relative| relative / TIMESTAMP_SCALE_NS)
        .ok_or(MatroskaVideoError::TimestampOverflow)
}

fn append_master(
    destination: &mut Vec<u8>,
    id: &[u8],
    contents: &[u8],
) -> Result<(), MatroskaVideoError> {
    destination.extend_from_slice(id);
    append_vint(
        destination,
        u64::try_from(contents.len()).unwrap_or(u64::MAX),
    )?;
    destination.extend_from_slice(contents);
    Ok(())
}

fn append_bytes(
    destination: &mut Vec<u8>,
    id: &[u8],
    value: &[u8],
) -> Result<(), MatroskaVideoError> {
    append_master(destination, id, value)
}

fn append_unsigned(
    destination: &mut Vec<u8>,
    id: &[u8],
    value: u64,
) -> Result<(), MatroskaVideoError> {
    destination.extend_from_slice(id);
    let width = unsigned_width(value);
    append_vint(destination, width)?;
    let width = usize::try_from(width).map_err(|_| MatroskaVideoError::ContainerTooLarge)?;
    destination.extend_from_slice(&value.to_be_bytes()[8 - width..]);
    Ok(())
}

fn append_float(
    destination: &mut Vec<u8>,
    id: &[u8],
    value: f64,
) -> Result<(), MatroskaVideoError> {
    destination.extend_from_slice(id);
    append_vint(destination, 8)?;
    destination.extend_from_slice(&value.to_be_bytes());
    Ok(())
}

fn append_vint(destination: &mut Vec<u8>, value: u64) -> Result<(), MatroskaVideoError> {
    let encoded = encode_vint(value)?;
    destination.extend_from_slice(&encoded.0[..encoded.1]);
    Ok(())
}

fn write_element(
    output: &mut impl Write,
    id: &[u8],
    contents: &[u8],
) -> Result<(), MatroskaVideoError> {
    write_master_header(
        output,
        id,
        u64::try_from(contents.len()).unwrap_or(u64::MAX),
    )?;
    output.write_all(contents)?;
    Ok(())
}

fn write_master_header(
    output: &mut impl Write,
    id: &[u8],
    content_size: u64,
) -> Result<(), MatroskaVideoError> {
    output.write_all(id)?;
    let encoded = encode_vint(content_size)?;
    output.write_all(&encoded.0[..encoded.1])?;
    Ok(())
}

fn write_unsigned(
    output: &mut impl Write,
    id: &[u8],
    value: u64,
) -> Result<(), MatroskaVideoError> {
    output.write_all(id)?;
    let width = unsigned_width(value);
    let encoded = encode_vint(width)?;
    output.write_all(&encoded.0[..encoded.1])?;
    let width = usize::try_from(width).map_err(|_| MatroskaVideoError::ContainerTooLarge)?;
    output.write_all(&value.to_be_bytes()[8 - width..])?;
    Ok(())
}

fn encode_vint(value: u64) -> Result<([u8; 8], usize), MatroskaVideoError> {
    if value > MAX_EBML_SIZE {
        return Err(MatroskaVideoError::ContainerTooLarge);
    }
    let width = (1..=8)
        .find(|width| value < (1_u64 << (7 * width)) - 1)
        .ok_or(MatroskaVideoError::ContainerTooLarge)?;
    let encoded = value | (1_u64 << (7 * width));
    let bytes = encoded.to_be_bytes();
    let mut result = [0_u8; 8];
    result[..width].copy_from_slice(&bytes[8 - width..]);
    Ok((result, width))
}

fn unsigned_width(value: u64) -> u64 {
    let leading_bytes = usize::try_from(value.leading_zeros() / 8).unwrap_or(0);
    u64::try_from(8_usize.saturating_sub(leading_bytes).max(1)).unwrap_or(8)
}

fn vint_width(value: u64) -> u64 {
    (1..=8)
        .find(|width| value < (1_u64 << (7 * width)) - 1)
        .unwrap_or(8)
}

fn element_size(id: &[u8], content_size: u64) -> u64 {
    u64::try_from(id.len())
        .unwrap_or(u64::MAX)
        .saturating_add(vint_width(content_size))
        .saturating_add(content_size)
}

struct CountingWriter<'a, W> {
    inner: &'a mut W,
    written: u64,
}

impl<'a, W> CountingWriter<'a, W> {
    const fn new(inner: &'a mut W) -> Self {
        Self { inner, written: 0 }
    }
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncodedReplayPacket, ReplayPacketFormat, ReplayVideoCodec, ReplayVideoStream};
    use redunar_core::ReplaySettings;
    use std::fs;
    use std::process::Command;

    fn stream() -> ReplayVideoStream {
        ReplayVideoStream::new(
            ReplayVideoCodec::H264,
            ReplayPacketFormat::H264LengthPrefixed4,
            1_920,
            1_080,
            60,
            vec![
                1, 66, 0, 30, 0xff, 0xe1, 0, 2, 0x67, 0x42, 1, 0, 2, 0x68, 0xce,
            ],
        )
        .expect("valid stream")
    }

    fn packet(timestamp_ns: u64, keyframe: bool, nal: u8) -> EncodedReplayPacket {
        EncodedReplayPacket::new(
            timestamp_ns,
            16_666_667,
            keyframe,
            vec![0, 0, 0, 2, nal, 0x88],
        )
        .expect("packet")
    }

    #[test]
    fn video_only_writer_emits_ebml_tracks_and_bounded_blocks() {
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(packet(2_000_000_000, true, 0x65))
            .expect("keyframe");
        ring.push(packet(2_016_666_667, false, 0x41))
            .expect("delta");
        let mut output = Vec::new();
        let summary = write_video_only_matroska(&mut output, &stream(), &ring).expect("mux");
        assert_eq!(&output[..4], ID_EBML);
        assert!(
            output
                .windows(ID_SEGMENT.len())
                .any(|bytes| bytes == ID_SEGMENT)
        );
        assert!(
            output
                .windows(b"V_MPEG4/ISO/AVC".len())
                .any(|bytes| bytes == b"V_MPEG4/ISO/AVC")
        );
        assert!(
            output
                .windows(ID_DURATION.len())
                .any(|bytes| bytes == ID_DURATION)
        );
        assert_eq!(summary.packets, 2);
        assert_eq!(summary.clusters, 1);
        assert_eq!(summary.duration_ns, 33_333_334);
        assert_eq!(summary.bytes_written, output.len() as u64);
    }

    #[test]
    fn long_clip_starts_new_clusters_before_block_timecode_overflow() {
        let mut ring = ReplayRing::new(ReplaySettings {
            duration: redunar_core::ReplayDuration::Seconds60,
            ..ReplaySettings::default()
        });
        ring.push(packet(0, true, 0x65)).expect("first");
        ring.push(packet(31_000_000_000, true, 0x65))
            .expect("second");
        let mut output = Vec::new();
        let summary = write_video_only_matroska(&mut output, &stream(), &ring).expect("mux");
        assert_eq!(summary.clusters, 2);
        assert_eq!(
            output
                .windows(ID_CLUSTER.len())
                .filter(|bytes| *bytes == ID_CLUSTER)
                .count(),
            2
        );
    }

    #[test]
    fn audio_video_writer_adds_owned_opus_track_and_interleaved_blocks() {
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(packet(2_000_000_000, true, 0x65)).expect("video");
        ring.push(packet(2_016_666_667, false, 0x41))
            .expect("video");
        let audio = ReplayAudioSnapshot {
            stream: redunar_capture_audio::OpusStreamDescription::default(),
            packets: vec![redunar_capture_audio::EncodedOpusPacket {
                timestamp_ns: 2_000_000_000,
                duration_ns: 20_000_000,
                bytes: vec![0xf8, 0xff, 0xfe],
            }],
        };
        let mut output = Vec::new();
        let summary = write_audio_video_matroska(&mut output, &stream(), &ring, &audio)
            .expect("audio/video Matroska");
        assert!(output.windows(6).any(|bytes| bytes == b"A_OPUS"));
        assert!(output.windows(8).any(|bytes| bytes == b"OpusHead"));
        assert!(output.windows(4).any(|bytes| bytes == [0x82, 0, 0, 0x80]));
        let tracks = ReplayAudioTracks {
            system: audio.clone(),
            microphone: Some(audio.clone()),
        };
        let mut dual = Vec::new();
        write_audio_video_matroska_tracks(&mut dual, &stream(), &ring, &tracks)
            .expect("dual audio/video Matroska");
        assert!(dual.windows(4).any(|bytes| bytes == [0x83, 0, 0, 0x80]));
        assert_eq!(summary.packets, 2);
    }

    #[test]
    fn empty_or_malformed_history_is_rejected_before_output() {
        let empty = ReplayRing::new(ReplaySettings::default());
        let mut output = Vec::new();
        assert!(matches!(
            write_video_only_matroska(&mut output, &stream(), &empty),
            Err(MatroskaVideoError::EmptyRing)
        ));
        assert!(output.is_empty());

        let mut malformed = ReplayRing::new(ReplaySettings::default());
        malformed
            .push(
                EncodedReplayPacket::new(0, 1, true, vec![0, 0, 0, 8, 1])
                    .expect("ring-valid opaque packet"),
            )
            .expect("ring");
        assert!(matches!(
            write_video_only_matroska(&mut output, &stream(), &malformed),
            Err(MatroskaVideoError::InvalidPacket)
        ));
        assert!(output.is_empty());
    }

    #[test]
    #[ignore = "requires the optional ffprobe executable and local libopus"]
    fn audio_video_matroska_reports_h264_and_opus_streams() {
        let mut ring = ReplayRing::new(ReplaySettings::default());
        ring.push(packet(1, true, 0x65)).expect("video");
        let mut encoder = redunar_capture_audio::OpusEncoder::open().expect("libopus");
        let samples = vec![
            0_i16;
            redunar_capture_audio::AUDIO_FRAME_SAMPLES_PER_CHANNEL
                * usize::from(redunar_capture_audio::AUDIO_CHANNELS)
        ];
        let audio = ReplayAudioSnapshot {
            stream: encoder.stream(),
            packets: vec![encoder.encode_frame(1, &samples).expect("Opus")],
        };
        let mut output = Vec::new();
        write_audio_video_matroska(&mut output, &stream(), &ring, &audio).expect("Matroska");
        let path =
            std::env::temp_dir().join(format!("redunar-av-mkv-ffprobe-{}.mkv", std::process::id()));
        fs::write(&path, output).expect("write Matroska");
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,codec_type",
                "-of",
                "default=nw=1",
            ])
            .arg(&path)
            .output()
            .expect("ffprobe");
        let _ = fs::remove_file(path);
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let stdout = String::from_utf8_lossy(&probe.stdout);
        assert!(stdout.contains("codec_name=h264"), "{stdout}");
        assert!(stdout.contains("codec_name=opus"), "{stdout}");
    }
}
