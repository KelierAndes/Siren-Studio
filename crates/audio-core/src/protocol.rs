//! Custom low-latency UDP frame format.
//!
//! ```text
//!  0      2   3   4      6      8          12                   20        28
//! +------+---+---+------+------+----------+--------------------+---------+---------+
//! | magic|ver|cdc|flags |stream|    seq   |   timestamp_us     | origin  | payload |
//! +------+---+---+------+------+----------+--------------------+---------+---------+
//!   2B    1B  1B   2B     2B       4B             8B               8B
//! ```
//!
//! `flags` carries the sender's frame duration in milliseconds. The receiver
//! needs it to synthesize the right number of concealment frames after loss.
//! One UDP datagram always contains exactly one audio frame.
//!
//! `origin` is a stable per-node id. The same encoded frame may reach a peer
//! over two carriers at once (Wi-Fi UDP and a Bluetooth tunnel); the receiver
//! uses `(origin, stream_id)` as the logical stream identity and `seq` to drop
//! the second copy, so a dual-stack link never mixes the same audio twice.
//!
//! Version 3 is the compact Bluetooth header: same fields minus the
//! diagnostics timestamp (20 B instead of 28 B). At 5 ms frames that is
//! 16 kbps per stream of saved airtime, which matters on a shared piconet.

use crate::format::{CHANNELS, SAMPLE_BYTES};

pub const MAGIC: [u8; 2] = *b"SR";
pub const VERSION: u8 = 2;
pub const HEADER_LEN: usize = 28;
/// Version 1 datagrams (no origin id) are still accepted.
pub const HEADER_LEN_V1: usize = 20;
/// Compact Bluetooth framing: no timestamp, origin kept for peer identity.
pub const VERSION_COMPACT: u8 = 3;
pub const HEADER_LEN_COMPACT: usize = 20;
/// Version 4: compact header plus the bitmask of origins bundled in a hub
/// N-1 mix (bit = origin % 64).
pub const VERSION_MIX: u8 = 4;
pub const HEADER_LEN_MIX: usize = 28;
/// Reserved stream id for a hub-built N-1 mix carried over Bluetooth.
pub const MIX_STREAM_ID: u16 = 0xFFF0;
/// Payload is fixed-size stereo f32.
pub const RAW_FRAME_BYTES_PER_SAMPLE_FRAME: usize = SAMPLE_BYTES * CHANNELS;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("datagram shorter than header ({0} bytes)")]
    Short(usize),
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported version {0}")]
    BadVersion(u8),
    #[error("unknown codec id {0}")]
    BadCodec(u8),
    #[error("invalid frame duration flag {0}")]
    BadFrameMs(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Codec {
    RawF32 = 0,
    Opus = 1,
}

impl Codec {
    pub fn from_id(id: u8) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Codec::RawF32),
            1 => Ok(Codec::Opus),
            other => Err(ProtocolError::BadCodec(other)),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Codec::RawF32 => "raw",
            Codec::Opus => "opus",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub codec: Codec,
    /// Frame duration in milliseconds (1..=60).
    pub frame_ms: u16,
    pub stream_id: u16,
    pub seq: u32,
    /// Sender wall clock in microseconds since the Unix epoch.
    pub timestamp_us: u64,
    /// Stable sender id; zero for legacy version 1 datagrams.
    pub origin: u64,
    /// v4 mix frames: bit set = that origin's audio is inside this mix.
    pub mix_mask: u64,
}

impl FrameHeader {
    pub fn new(
        codec: Codec,
        frame_ms: u16,
        stream_id: u16,
        seq: u32,
        timestamp_us: u64,
        origin: u64,
    ) -> Self {
        Self {
            codec,
            frame_ms,
            stream_id,
            seq,
            timestamp_us,
            origin,
            mix_mask: 0,
        }
    }

    pub fn encode(self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = VERSION;
        buf[3] = self.codec as u8;
        buf[4..6].copy_from_slice(&self.frame_ms.to_le_bytes());
        buf[6..8].copy_from_slice(&self.stream_id.to_le_bytes());
        buf[8..12].copy_from_slice(&self.seq.to_le_bytes());
        buf[12..20].copy_from_slice(&self.timestamp_us.to_le_bytes());
        buf[20..28].copy_from_slice(&self.origin.to_le_bytes());
        buf
    }

    /// Version 3 layout: timestamp dropped, origin moved forward.
    pub fn encode_compact(self) -> [u8; HEADER_LEN_COMPACT] {
        let mut buf = [0u8; HEADER_LEN_COMPACT];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = VERSION_COMPACT;
        buf[3] = self.codec as u8;
        buf[4..6].copy_from_slice(&self.frame_ms.to_le_bytes());
        buf[6..8].copy_from_slice(&self.stream_id.to_le_bytes());
        buf[8..12].copy_from_slice(&self.seq.to_le_bytes());
        buf[12..20].copy_from_slice(&self.origin.to_le_bytes());
        buf
    }

    /// Version 4: compact layout + a 64-bit origin mask.
    pub fn encode_mix(self, mask: u64) -> [u8; HEADER_LEN_MIX] {
        let mut buf = [0u8; HEADER_LEN_MIX];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = VERSION_MIX;
        buf[3] = self.codec as u8;
        buf[4..6].copy_from_slice(&self.frame_ms.to_le_bytes());
        buf[6..8].copy_from_slice(&self.stream_id.to_le_bytes());
        buf[8..12].copy_from_slice(&self.seq.to_le_bytes());
        buf[12..20].copy_from_slice(&self.origin.to_le_bytes());
        buf[20..28].copy_from_slice(&mask.to_le_bytes());
        buf
    }

    pub fn decode(datagram: &[u8]) -> Result<(Self, &[u8]), ProtocolError> {
        if datagram.len() < HEADER_LEN_V1 {
            return Err(ProtocolError::Short(datagram.len()));
        }
        if datagram[0..2] != MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        let version = datagram[2];
        let header_len = match version {
            1 => HEADER_LEN_V1,
            2 => HEADER_LEN,
            3 => HEADER_LEN_COMPACT,
            4 => HEADER_LEN_MIX,
            other => return Err(ProtocolError::BadVersion(other)),
        };
        if datagram.len() < header_len {
            return Err(ProtocolError::Short(datagram.len()));
        }
        let codec = Codec::from_id(datagram[3])?;
        let frame_ms = u16::from_le_bytes([datagram[4], datagram[5]]);
        if frame_ms == 0 || frame_ms > 60 {
            return Err(ProtocolError::BadFrameMs(frame_ms));
        }
        let stream_id = u16::from_le_bytes([datagram[6], datagram[7]]);
        let seq = u32::from_le_bytes([datagram[8], datagram[9], datagram[10], datagram[11]]);
        let (timestamp_us, origin) = match version {
            1 => (u64::from_le_bytes(datagram[12..20].try_into().unwrap()), 0),
            3 | 4 => (0, u64::from_le_bytes(datagram[12..20].try_into().unwrap())),
            _ => (
                u64::from_le_bytes(datagram[12..20].try_into().unwrap()),
                u64::from_le_bytes(datagram[20..28].try_into().unwrap()),
            ),
        };
        let mix_mask = if version == VERSION_MIX {
            u64::from_le_bytes(datagram[20..28].try_into().unwrap())
        } else {
            0
        };
        Ok((
            Self {
                codec,
                frame_ms,
                stream_id,
                seq,
                timestamp_us,
                origin,
                mix_mask,
            },
            &datagram[header_len..],
        ))
    }
}

/// `(origin, stream_id, seq)` for dedupe/loop checks, for any header version.
pub fn frame_identity(payload: &[u8]) -> Option<(u64, u16, u32)> {
    // stream/seq live in the first 12 bytes of every version.
    if payload.len() < 12 || payload[0..2] != MAGIC {
        return None;
    }
    let stream_id = u16::from_le_bytes([payload[6], payload[7]]);
    let seq = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    let origin = match payload[2] {
        1 => 0,
        3 | 4 => u64::from_le_bytes(payload[12..20].try_into().ok()?),
        _ => {
            if payload.len() < HEADER_LEN {
                return None;
            }
            u64::from_le_bytes(payload[20..28].try_into().ok()?)
        }
    };
    Some((origin, stream_id, seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let header = FrameHeader::new(
            Codec::Opus,
            5,
            42,
            123_456,
            1_700_000_000_000_000,
            0xDEAD_BEEF_CAFE_F00D,
        );
        let encoded = header.encode();
        let (decoded, payload) = FrameHeader::decode(&encoded).unwrap();
        assert_eq!(header, decoded);
        assert!(payload.is_empty());
    }

    #[test]
    fn header_with_payload() {
        let mut datagram = FrameHeader::new(Codec::RawF32, 10, 7, 9, 99, 0)
            .encode()
            .to_vec();
        datagram.extend_from_slice(&[1, 2, 3]);
        let (header, payload) = FrameHeader::decode(&datagram).unwrap();
        assert_eq!(header.codec, Codec::RawF32);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn accepts_legacy_v1_without_origin() {
        let mut datagram = vec![0u8; HEADER_LEN_V1];
        datagram[0..2].copy_from_slice(&MAGIC);
        datagram[2] = 1;
        datagram[3] = Codec::RawF32 as u8;
        datagram[4..6].copy_from_slice(&5u16.to_le_bytes());
        datagram[6..8].copy_from_slice(&3u16.to_le_bytes());
        datagram[8..12].copy_from_slice(&7u32.to_le_bytes());
        datagram.extend_from_slice(&[9, 9]);
        let (header, payload) = FrameHeader::decode(&datagram).unwrap();
        assert_eq!(header.origin, 0);
        assert_eq!(header.stream_id, 3);
        assert_eq!(payload, &[9, 9]);
    }

    #[test]
    fn compact_header_roundtrip_and_identity() {
        let header = FrameHeader::new(Codec::Opus, 5, 9, 1000, 123, 0xABCD);
        let encoded = header.encode_compact();
        assert_eq!(encoded.len(), HEADER_LEN_COMPACT);
        let (decoded, payload) = FrameHeader::decode(&encoded).unwrap();
        assert!(payload.is_empty());
        assert_eq!(decoded.origin, 0xABCD);
        assert_eq!(decoded.seq, 1000);
        assert_eq!(decoded.timestamp_us, 0);
        assert_eq!(frame_identity(&encoded), Some((0xABCD, 9, 1000)));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut datagram = FrameHeader::new(Codec::RawF32, 5, 1, 1, 1, 0).encode();
        datagram[0] = b'X';
        assert_eq!(FrameHeader::decode(&datagram), Err(ProtocolError::BadMagic));
    }

    #[test]
    fn rejects_short() {
        assert_eq!(FrameHeader::decode(&[0; 4]), Err(ProtocolError::Short(4)));
    }

    #[test]
    fn rejects_unknown_codec() {
        let mut datagram = FrameHeader::new(Codec::RawF32, 5, 1, 1, 1, 0).encode();
        datagram[3] = 9;
        assert_eq!(
            FrameHeader::decode(&datagram),
            Err(ProtocolError::BadCodec(9))
        );
    }

    #[test]
    fn raw_frame_size_matches_format() {
        assert_eq!(
            crate::format::frames_for_ms(5.0) * RAW_FRAME_BYTES_PER_SAMPLE_FRAME,
            1920
        );
    }
}
