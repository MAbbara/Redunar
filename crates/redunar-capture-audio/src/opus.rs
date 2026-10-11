use crate::{
    AUDIO_CHANNELS, AUDIO_FRAME_DURATION_NS, AUDIO_FRAME_SAMPLES_PER_CHANNEL, AUDIO_SAMPLE_RATE,
};
use std::error::Error;
use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_void};
use std::fmt;
use std::ptr;

const OPUS_APPLICATION_AUDIO: c_int = 2049;
const OPUS_OK: c_int = 0;
const MAX_OPUS_PACKET_BYTES: usize = 4_096;

type EncoderCreate = unsafe extern "C" fn(c_int, c_int, c_int, *mut c_int) -> *mut c_void;
type Encode = unsafe extern "C" fn(*mut c_void, *const i16, c_int, *mut c_uchar, c_int) -> c_int;
type EncoderDestroy = unsafe extern "C" fn(*mut c_void);
type DecoderCreate = unsafe extern "C" fn(c_int, c_int, *mut c_int) -> *mut c_void;
type Decode =
    unsafe extern "C" fn(*mut c_void, *const c_uchar, c_int, *mut i16, c_int, c_int) -> c_int;
type DecoderDestroy = unsafe extern "C" fn(*mut c_void);
type StrError = unsafe extern "C" fn(c_int) -> *const c_char;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpusStreamDescription {
    pub sample_rate: u32,
    pub channels: u8,
    pub codec_private: [u8; 19],
}

impl Default for OpusStreamDescription {
    fn default() -> Self {
        let mut codec_private = [0_u8; 19];
        codec_private[..8].copy_from_slice(b"OpusHead");
        codec_private[8] = 1;
        codec_private[9] = AUDIO_CHANNELS;
        codec_private[10..12].copy_from_slice(&312_u16.to_le_bytes());
        codec_private[12..16].copy_from_slice(&AUDIO_SAMPLE_RATE.to_le_bytes());
        Self {
            sample_rate: AUDIO_SAMPLE_RATE,
            channels: AUDIO_CHANNELS,
            codec_private,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedOpusPacket {
    pub timestamp_ns: u64,
    pub duration_ns: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct OpusEncoderError(String);

impl fmt::Display for OpusEncoderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for OpusEncoderError {}

pub struct OpusEncoder {
    library: DynamicLibrary,
    encoder: *mut c_void,
    encode: Encode,
    destroy: EncoderDestroy,
    strerror: StrError,
}

// The encoder is owned and used by one audio worker thread.
unsafe impl Send for OpusEncoder {}

impl OpusEncoder {
    /// Open the system `libopus` encoder with Redunar's fixed stereo contract.
    ///
    /// # Errors
    ///
    /// Returns [`OpusEncoderError`] when the library, required symbols, or
    /// encoder profile are unavailable.
    pub fn open() -> Result<Self, OpusEncoderError> {
        let library = DynamicLibrary::open("libopus.so.0")?;
        let create = unsafe { library.symbol::<EncoderCreate>("opus_encoder_create")? };
        let encode = unsafe { library.symbol::<Encode>("opus_encode")? };
        let destroy = unsafe { library.symbol::<EncoderDestroy>("opus_encoder_destroy")? };
        let strerror = unsafe { library.symbol::<StrError>("opus_strerror")? };
        let mut error = 0;
        let encoder = unsafe {
            create(
                c_int::try_from(AUDIO_SAMPLE_RATE).unwrap_or(48_000),
                c_int::from(AUDIO_CHANNELS),
                OPUS_APPLICATION_AUDIO,
                &raw mut error,
            )
        };
        if encoder.is_null() || error != OPUS_OK {
            return Err(OpusEncoderError(format!(
                "could not initialize Opus encoder: {error}"
            )));
        }
        Ok(Self {
            library,
            encoder,
            encode,
            destroy,
            strerror,
        })
    }

    /// Encode exactly one interleaved 20 ms stereo PCM frame.
    ///
    /// # Errors
    ///
    /// Returns [`OpusEncoderError`] for invalid input bounds, timestamp
    /// overflow, or a native encoder failure.
    pub fn encode_frame(
        &mut self,
        timestamp_ns: u64,
        samples: &[i16],
    ) -> Result<EncodedOpusPacket, OpusEncoderError> {
        let expected = AUDIO_FRAME_SAMPLES_PER_CHANNEL * usize::from(AUDIO_CHANNELS);
        if samples.len() != expected || timestamp_ns.checked_add(AUDIO_FRAME_DURATION_NS).is_none()
        {
            return Err(OpusEncoderError(
                "invalid bounded Opus input frame".to_owned(),
            ));
        }
        // Keep scratch capacity off the retained packet: thousands of small
        // frames must not each reserve the encoder's maximum output allocation.
        let mut output = [0_u8; MAX_OPUS_PACKET_BYTES];
        let encoded = unsafe {
            (self.encode)(
                self.encoder,
                samples.as_ptr(),
                c_int::try_from(AUDIO_FRAME_SAMPLES_PER_CHANNEL).unwrap_or(960),
                output.as_mut_ptr(),
                c_int::try_from(output.len()).unwrap_or(4_096),
            )
        };
        if encoded <= 0 {
            let message = unsafe { CStr::from_ptr((self.strerror)(encoded)) }.to_string_lossy();
            return Err(OpusEncoderError(format!("Opus encode failed: {message}")));
        }
        let length = usize::try_from(encoded).unwrap_or(0);
        let bytes = output
            .get(..length)
            .ok_or_else(|| {
                OpusEncoderError("Opus encoded packet exceeds its fixed bound".to_owned())
            })?
            .to_vec();
        Ok(EncodedOpusPacket {
            timestamp_ns,
            duration_ns: AUDIO_FRAME_DURATION_NS,
            bytes,
        })
    }

    #[must_use]
    pub fn stream(&self) -> OpusStreamDescription {
        OpusStreamDescription::default()
    }
}

impl Drop for OpusEncoder {
    fn drop(&mut self) {
        if !self.encoder.is_null() {
            unsafe { (self.destroy)(self.encoder) };
            self.encoder = ptr::null_mut();
        }
        let _ = &self.library;
    }
}

/// Stateful Opus decoder used when combining the system and microphone tracks.
pub struct OpusDecoder {
    library: DynamicLibrary,
    decoder: *mut c_void,
    decode: Decode,
    destroy: DecoderDestroy,
}

// SAFETY: the decoder owns its opaque state and library; only &mut self decodes.
unsafe impl Send for OpusDecoder {}

impl OpusDecoder {
    /// Load libopus and create a stereo decoder.
    ///
    /// # Errors
    /// Returns an error when the library, symbols or decoder cannot be initialized.
    pub fn open() -> Result<Self, OpusEncoderError> {
        let library = DynamicLibrary::open("libopus.so.0")?;
        let create = unsafe { library.symbol::<DecoderCreate>("opus_decoder_create")? };
        let decode = unsafe { library.symbol::<Decode>("opus_decode")? };
        let destroy = unsafe { library.symbol::<DecoderDestroy>("opus_decoder_destroy")? };
        let mut error = 0;
        let decoder = unsafe {
            create(
                c_int::try_from(AUDIO_SAMPLE_RATE).unwrap_or(48_000),
                c_int::from(AUDIO_CHANNELS),
                &raw mut error,
            )
        };
        if decoder.is_null() || error != OPUS_OK {
            return Err(OpusEncoderError(format!(
                "could not initialize Opus decoder: {error}"
            )));
        }
        Ok(Self {
            library,
            decoder,
            decode,
            destroy,
        })
    }

    /// Decode one 20 ms stereo frame.
    ///
    /// # Errors
    /// Returns an error for an empty, oversized, malformed or non-20-ms packet.
    pub fn decode_frame(
        &mut self,
        packet: &EncodedOpusPacket,
    ) -> Result<Vec<i16>, OpusEncoderError> {
        if packet.bytes.is_empty() || packet.bytes.len() > MAX_OPUS_PACKET_BYTES {
            return Err(OpusEncoderError("Opus packet size is invalid".to_owned()));
        }
        let mut samples =
            vec![0_i16; AUDIO_FRAME_SAMPLES_PER_CHANNEL * usize::from(AUDIO_CHANNELS)];
        // SAFETY: opaque decoder state and input are valid; samples has room for
        // the fixed stereo frame supplied as the maximum decoder output size.
        let decoded = unsafe {
            (self.decode)(
                self.decoder,
                packet.bytes.as_ptr(),
                c_int::try_from(packet.bytes.len()).unwrap_or(0),
                samples.as_mut_ptr(),
                c_int::try_from(AUDIO_FRAME_SAMPLES_PER_CHANNEL).unwrap_or(960),
                0,
            )
        };
        if decoded != c_int::try_from(AUDIO_FRAME_SAMPLES_PER_CHANNEL).unwrap_or(960) {
            return Err(OpusEncoderError("Opus decode failed".to_owned()));
        }
        Ok(samples)
    }
}

impl Drop for OpusDecoder {
    fn drop(&mut self) {
        if !self.decoder.is_null() {
            unsafe { (self.destroy)(self.decoder) };
            self.decoder = ptr::null_mut();
        }
        let _ = &self.library;
    }
}

struct DynamicLibrary(*mut c_void);

impl DynamicLibrary {
    fn open(name: &str) -> Result<Self, OpusEncoderError> {
        let name =
            CString::new(name).map_err(|_| OpusEncoderError("invalid library name".to_owned()))?;
        let handle = unsafe { dlopen(name.as_ptr(), 2) };
        if handle.is_null() {
            return Err(OpusEncoderError("libopus is unavailable".to_owned()));
        }
        Ok(Self(handle))
    }

    unsafe fn symbol<T: Copy>(&self, name: &str) -> Result<T, OpusEncoderError> {
        let name =
            CString::new(name).map_err(|_| OpusEncoderError("invalid symbol name".to_owned()))?;
        let value = unsafe { dlsym(self.0, name.as_ptr()) };
        if value.is_null() {
            return Err(OpusEncoderError(format!(
                "libopus symbol is unavailable: {name:?}"
            )));
        }
        Ok(unsafe { std::mem::transmute_copy(&value) })
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { dlclose(self.0) };
        }
    }
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_head_is_bounded_and_describes_stereo_48khz() {
        let stream = OpusStreamDescription::default();
        assert_eq!(&stream.codec_private[..8], b"OpusHead");
        assert_eq!(stream.codec_private[9], 2);
        assert_eq!(
            u32::from_le_bytes(stream.codec_private[12..16].try_into().unwrap()),
            48_000
        );
    }

    #[test]
    fn local_opus_encoder_produces_one_bounded_packet() {
        let Ok(mut encoder) = OpusEncoder::open() else {
            return;
        };
        let samples = vec![0_i16; AUDIO_FRAME_SAMPLES_PER_CHANNEL * usize::from(AUDIO_CHANNELS)];
        let packet = encoder.encode_frame(1, &samples).expect("silence packet");
        assert!(!packet.bytes.is_empty());
        assert!(packet.bytes.len() <= MAX_OPUS_PACKET_BYTES);
        assert_eq!(
            packet.bytes.capacity(),
            packet.bytes.len(),
            "retention accounts allocated payload, not unused encoder scratch"
        );
        assert_eq!(packet.duration_ns, AUDIO_FRAME_DURATION_NS);
    }
}
