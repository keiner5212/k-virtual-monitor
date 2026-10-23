//! Wire framing.
//!
//! Every message is a fixed 18 byte header followed by a payload:
//!
//! ```text
//! 0      2   magic "KM"
//! 2      1   protocol version
//! 3      1   message type
//! 4      2   flags
//! 6      8   sequence number (per direction, per session)
//! 14     4   payload length
//! ```
//!
//! Handshake messages carry a plaintext payload. Every other message carries
//! `ciphertext || tag`, produced by the session AEAD over the header as
//! associated data.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::ProtocolError;
use crate::limits::PROTOCOL_VERSION;

pub const MAGIC: [u8; 2] = *b"KM";
pub const HEADER_LEN: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    Hello = 0,
    Challenge = 1,
    Auth = 2,
    AuthOk = 3,
    Error = 4,
    DisplayInfo = 5,
    Frame = 6,
    FrameAck = 7,
    InputMouse = 8,
    InputKey = 9,
    InputTouch = 10,
    Heartbeat = 11,
    HeartbeatAck = 12,
    Close = 13,
}

impl MessageType {
    pub fn from_u8(value: u8) -> Result<Self, ProtocolError> {
        Ok(match value {
            0 => MessageType::Hello,
            1 => MessageType::Challenge,
            2 => MessageType::Auth,
            3 => MessageType::AuthOk,
            4 => MessageType::Error,
            5 => MessageType::DisplayInfo,
            6 => MessageType::Frame,
            7 => MessageType::FrameAck,
            8 => MessageType::InputMouse,
            9 => MessageType::InputKey,
            10 => MessageType::InputTouch,
            11 => MessageType::Heartbeat,
            12 => MessageType::HeartbeatAck,
            13 => MessageType::Close,
            other => return Err(ProtocolError::UnknownType(other)),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            MessageType::Hello => "HELLO",
            MessageType::Challenge => "CHALLENGE",
            MessageType::Auth => "AUTH",
            MessageType::AuthOk => "AUTH_OK",
            MessageType::Error => "ERROR",
            MessageType::DisplayInfo => "DISPLAY_INFO",
            MessageType::Frame => "FRAME",
            MessageType::FrameAck => "FRAME_ACK",
            MessageType::InputMouse => "INPUT_MOUSE",
            MessageType::InputKey => "INPUT_KEY",
            MessageType::InputTouch => "INPUT_TOUCH",
            MessageType::Heartbeat => "HEARTBEAT",
            MessageType::HeartbeatAck => "HEARTBEAT_ACK",
            MessageType::Close => "CLOSE",
        }
    }

    /// True for the messages exchanged before the session keys exist.
    pub fn is_handshake(self) -> bool {
        matches!(
            self,
            MessageType::Hello | MessageType::Challenge | MessageType::Auth | MessageType::AuthOk
        )
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub kind: MessageType,
    pub flags: u16,
    pub seq: u64,
    pub payload_len: u32,
}

impl Header {
    pub fn new(kind: MessageType, seq: u64, flags: u16, payload_len: usize) -> Self {
        Header {
            version: PROTOCOL_VERSION,
            kind,
            flags,
            seq,
            payload_len: payload_len as u32,
        }
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..2].copy_from_slice(&MAGIC);
        out[2] = self.version;
        out[3] = self.kind as u8;
        out[4..6].copy_from_slice(&self.flags.to_be_bytes());
        out[6..14].copy_from_slice(&self.seq.to_be_bytes());
        out[14..18].copy_from_slice(&self.payload_len.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8; HEADER_LEN]) -> Result<Self, ProtocolError> {
        if bytes[0..2] != MAGIC {
            return Err(ProtocolError::BadMagic([bytes[0], bytes[1]]));
        }
        let version = bytes[2];
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::BadVersion(version, PROTOCOL_VERSION));
        }
        Ok(Header {
            version,
            kind: MessageType::from_u8(bytes[3])?,
            flags: u16::from_be_bytes([bytes[4], bytes[5]]),
            seq: u64::from_be_bytes(bytes[6..14].try_into().expect("8 bytes")),
            payload_len: u32::from_be_bytes(bytes[14..18].try_into().expect("4 bytes")),
        })
    }
}

/// Reads one message. The payload length is checked against the limit of the
/// message type before any buffer is reserved, so a hostile peer cannot make
/// us allocate more than the type allows.
pub async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(Header, Vec<u8>), ProtocolError> {
    let mut raw = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut raw)
        .await
        .map_err(|err| classify_read(err, "reading message header"))?;
    let header = Header::decode(&raw)?;
    let limit = crate::limits::max_payload(header.kind);
    let len = header.payload_len as usize;
    if len > limit {
        return Err(ProtocolError::PayloadTooLarge {
            kind: header.kind.name(),
            actual: len,
            limit,
        });
    }
    let mut payload = vec![0u8; len];
    if len > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|err| classify_read(err, "reading message payload"))?;
    }
    Ok((header, payload))
}

/// Reads one message with an explicit limit, for the cases where the caller
/// knows a tighter bound than the one of the message type.
pub async fn read_message_with_limit<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_payload: usize,
) -> Result<(Header, Vec<u8>), ProtocolError> {
    let mut raw = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut raw)
        .await
        .map_err(|err| classify_read(err, "reading message header"))?;
    let header = Header::decode(&raw)?;
    let len = header.payload_len as usize;
    if len > max_payload {
        return Err(ProtocolError::PayloadTooLarge {
            kind: header.kind.name(),
            actual: len,
            limit: max_payload,
        });
    }
    let mut payload = vec![0u8; len];
    if len > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|err| classify_read(err, "reading message payload"))?;
    }
    Ok((header, payload))
}

/// Writes one message. The payload is written as a single vectored write so a
/// frame is not split into two syscalls.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &Header,
    payload: &[u8],
) -> Result<(), ProtocolError> {
    let raw = header.encode();
    writer
        .write_all(&raw)
        .await
        .map_err(|err| ProtocolError::io("writing message header", err))?;
    if !payload.is_empty() {
        writer
            .write_all(payload)
            .await
            .map_err(|err| ProtocolError::io("writing message payload", err))?;
    }
    writer
        .flush()
        .await
        .map_err(|err| ProtocolError::io("flushing message", err))?;
    Ok(())
}

fn classify_read(err: std::io::Error, operation: &'static str) -> ProtocolError {
    match err.kind() {
        std::io::ErrorKind::UnexpectedEof => ProtocolError::UnexpectedEof { what: operation },
        _ => ProtocolError::io(operation, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let header = Header::new(MessageType::Frame, 42, 0x0102, 1234);
        let encoded = header.encode();
        assert_eq!(&encoded[0..2], b"KM");
        assert_eq!(Header::decode(&encoded).unwrap(), header);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut encoded = Header::new(MessageType::Hello, 1, 0, 0).encode();
        encoded[0] = b'X';
        assert!(matches!(
            Header::decode(&encoded),
            Err(ProtocolError::BadMagic(_))
        ));
    }

    #[test]
    fn unknown_version_is_rejected() {
        let mut encoded = Header::new(MessageType::Hello, 1, 0, 0).encode();
        encoded[2] = 9;
        assert!(matches!(
            Header::decode(&encoded),
            Err(ProtocolError::BadVersion(9, PROTOCOL_VERSION))
        ));
    }

    #[tokio::test]
    async fn oversized_payload_is_refused_before_allocation() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let header = Header::new(MessageType::Frame, 1, 0, 5_000_000);
        let raw = header.encode();
        tokio::spawn(async move {
            client.write_all(&raw).await.unwrap();
        });
        let err = read_message(&mut server).await.unwrap_err();
        assert!(matches!(
            err,
            ProtocolError::PayloadTooLarge {
                actual: 5_000_000,
                ..
            }
        ));
    }
}
