//! Payload structures for every message type.
//!
//! Payloads are encoded little endian except where the wire format is fixed.
//! Every decoder validates lengths and ranges, so a malformed payload is
//! rejected at the boundary instead of reaching the domain logic.

use crate::error::ProtocolError;
use crate::limits::{MAX_ERROR_TEXT, MAX_NAME_LEN};

/// Video codecs the protocol can carry, as a bit mask.
pub mod codec {
    pub const JPEG: u16 = 1 << 0;
    pub const H264: u16 = 1 << 1;
    pub const H265: u16 = 1 << 2;
    pub const VP9: u16 = 1 << 3;
    pub const AV1: u16 = 1 << 4;

    /// Codecs in the order the server prefers them when the client supports
    /// more than one. Higher compression first, because the goal is the
    /// highest frame rate the link can carry.
    pub const PREFERENCE: [u16; 5] = [AV1, H265, H264, VP9, JPEG];

    pub fn name(bit: u16) -> &'static str {
        match bit {
            JPEG => "jpeg",
            H264 => "h264",
            H265 => "h265",
            VP9 => "vp9",
            AV1 => "av1",
            _ => "unknown",
        }
    }
}

/// Capabilities a paired device may hold.
pub mod caps {
    /// May receive the screen.
    pub const VIEW: u8 = 1 << 0;
    /// May send mouse, key and touch input.
    pub const CONTROL: u8 = 1 << 1;
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Writer { bytes: Vec::new() }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i16(&mut self, value: i16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    fn string(&mut self, value: &str) {
        let raw = value.as_bytes();
        let len = raw.len().min(MAX_NAME_LEN);
        self.u16(len as u16);
        self.bytes.extend_from_slice(&raw[..len]);
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(ProtocolError::Violation("length overflow"))?;
        if end > self.bytes.len() {
            return Err(ProtocolError::PayloadSize {
                kind: "payload",
                actual: self.bytes.len() - self.pos,
                expected: n,
            });
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn i16(&mut self) -> Result<i16, ProtocolError> {
        Ok(i16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        Ok(self.take(N)?.try_into().expect("fixed size"))
    }

    fn string(&mut self) -> Result<String, ProtocolError> {
        let len = self.u16()? as usize;
        if len > MAX_NAME_LEN {
            return Err(ProtocolError::Violation("name is too long"));
        }
        let raw = self.take(len)?;
        String::from_utf8(raw.to_vec()).map_err(|_| ProtocolError::Violation("name is not utf-8"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub client_nonce: [u8; 32],
    pub device_id: [u8; 16],
    pub device_name: String,
    pub max_width: u16,
    pub max_height: u16,
    pub max_fps: u16,
    pub codec_mask: u16,
    pub caps_requested: u8,
}

impl Hello {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(&self.client_nonce);
        w.bytes(&self.device_id);
        w.string(&self.device_name);
        w.u16(self.max_width);
        w.u16(self.max_height);
        w.u16(self.max_fps);
        w.u16(self.codec_mask);
        w.u8(self.caps_requested);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        let hello = Hello {
            client_nonce: r.array()?,
            device_id: r.array()?,
            device_name: r.string()?,
            max_width: r.u16()?,
            max_height: r.u16()?,
            max_fps: r.u16()?,
            codec_mask: r.u16()?,
            caps_requested: r.u8()?,
        };
        Ok(hello)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub server_nonce: [u8; 32],
    pub server_name: String,
}

impl Challenge {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(&self.server_nonce);
        w.string(&self.server_name);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Challenge {
            server_nonce: r.array()?,
            server_name: r.string()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auth {
    pub proof: [u8; 32],
}

impl Auth {
    pub fn encode(&self) -> Vec<u8> {
        self.proof.to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Auth { proof: r.array()? })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthOk {
    pub server_proof: [u8; 32],
}

impl AuthOk {
    pub fn encode(&self) -> Vec<u8> {
        self.server_proof.to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(AuthOk {
            server_proof: r.array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: u16,
    pub text: String,
}

impl ErrorInfo {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.code);
        let raw = self.text.as_bytes();
        let len = raw.len().min(MAX_ERROR_TEXT);
        w.u8(len as u8);
        w.bytes(&raw[..len]);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        let code = r.u16()?;
        let len = r.u8()? as usize;
        let raw = r.take(len)?;
        Ok(ErrorInfo {
            code,
            text: String::from_utf8_lossy(raw).into_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub codec: u8,
    pub granted_caps: u8,
    pub server_name: String,
}

impl DisplayInfo {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.width);
        w.u16(self.height);
        w.u16(self.fps);
        w.u8(self.codec);
        w.u8(self.granted_caps);
        w.string(&self.server_name);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(DisplayInfo {
            width: r.u16()?,
            height: r.u16()?,
            fps: r.u16()?,
            codec: r.u8()?,
            granted_caps: r.u8()?,
            server_name: r.string()?,
        })
    }
}

/// Header of a frame message, in front of the codec payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub pts_ms: u64,
    pub keyframe: bool,
    pub width: u16,
    pub height: u16,
    pub codec: u8,
}

impl FrameHeader {
    /// pts (8) + keyframe (1) + width (2) + height (2) + codec (1).
    pub const LEN: usize = 14;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(self.pts_ms);
        w.u8(u8::from(self.keyframe));
        w.u16(self.width);
        w.u16(self.height);
        w.u8(self.codec);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(FrameHeader {
            pts_ms: r.u64()?,
            keyframe: r.u8()? != 0,
            width: r.u16()?,
            height: r.u16()?,
            codec: r.u8()?,
        })
    }
}

/// Acknowledgement of the frames received so far.
///
/// The client acknowledges on receipt, before decoding, so a slow decoder
/// never stalls the sender. `acked_seq` is the highest message sequence
/// received, `acked_bytes` the total payload bytes received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameAck {
    pub acked_seq: u64,
    pub acked_bytes: u64,
}

impl FrameAck {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(self.acked_seq);
        w.u64(self.acked_bytes);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(FrameAck {
            acked_seq: r.u64()?,
            acked_bytes: r.u64()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mouse {
    pub x: u16,
    pub y: u16,
    pub buttons: u8,
    pub pressed: bool,
    pub wheel: i16,
}

impl Mouse {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.x);
        w.u16(self.y);
        w.u8(self.buttons);
        w.u8(u8::from(self.pressed));
        w.i16(self.wheel);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Mouse {
            x: r.u16()?,
            y: r.u16()?,
            buttons: r.u8()?,
            pressed: r.u8()? != 0,
            wheel: r.i16()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub keycode: u16,
    pub pressed: bool,
    pub modifiers: u8,
}

impl Key {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.keycode);
        w.u8(u8::from(self.pressed));
        w.u8(self.modifiers);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Key {
            keycode: r.u16()?,
            pressed: r.u8()? != 0,
            modifiers: r.u8()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    pub pointer_id: u8,
    pub action: u8,
    pub x: u16,
    pub y: u16,
    pub pressure: u16,
}

impl Touch {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.pointer_id);
        w.u8(self.action);
        w.u16(self.x);
        w.u16(self.y);
        w.u16(self.pressure);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Touch {
            pointer_id: r.u8()?,
            action: r.u8()?,
            x: r.u16()?,
            y: r.u16()?,
            pressure: r.u16()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    pub client_time_ms: u64,
}

impl Heartbeat {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(self.client_time_ms);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Heartbeat {
            client_time_ms: r.u64()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Close {
    pub reason: u16,
}

impl Close {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.reason);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let mut r = Reader::new(bytes);
        Ok(Close { reason: r.u16()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip() {
        let hello = Hello {
            client_nonce: [1u8; 32],
            device_id: [2u8; 16],
            device_name: "pixel".into(),
            max_width: 1408,
            max_height: 792,
            max_fps: 30,
            codec_mask: codec::H264 | codec::JPEG,
            caps_requested: caps::VIEW | caps::CONTROL,
        };
        assert_eq!(Hello::decode(&hello.encode()).unwrap(), hello);
    }

    #[test]
    fn hello_rejects_a_truncated_payload() {
        let hello = Hello {
            client_nonce: [1u8; 32],
            device_id: [2u8; 16],
            device_name: "pixel".into(),
            max_width: 1,
            max_height: 1,
            max_fps: 1,
            codec_mask: 0,
            caps_requested: 0,
        };
        let bytes = hello.encode();
        assert!(Hello::decode(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn display_info_round_trip() {
        let info = DisplayInfo {
            width: 1408,
            height: 792,
            fps: 30,
            codec: 2,
            granted_caps: caps::VIEW,
            server_name: "desktop".into(),
        };
        assert_eq!(DisplayInfo::decode(&info.encode()).unwrap(), info);
    }

    #[test]
    fn frame_ack_round_trip() {
        let ack = FrameAck {
            acked_seq: 12,
            acked_bytes: 4096,
        };
        assert_eq!(FrameAck::decode(&ack.encode()).unwrap(), ack);
    }

    #[test]
    fn frame_header_round_trip() {
        let header = FrameHeader {
            pts_ms: 1234,
            keyframe: true,
            width: 1408,
            height: 792,
            codec: 2,
        };
        let encoded = header.encode();
        assert_eq!(encoded.len(), FrameHeader::LEN);
        assert_eq!(FrameHeader::decode(&encoded).unwrap(), header);
    }

    #[test]
    fn mouse_round_trip() {
        let mouse = Mouse {
            x: 100,
            y: 200,
            buttons: 1,
            pressed: true,
            wheel: -1,
        };
        assert_eq!(Mouse::decode(&mouse.encode()).unwrap(), mouse);
    }
}
