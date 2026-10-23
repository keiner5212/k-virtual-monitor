//! Hard limits enforced by the protocol layer.
//!
//! Every number here is a bound the implementation must enforce before it
//! allocates memory, before it spawns work, and before it trusts a peer.

/// Protocol version spoken by this build.
pub const PROTOCOL_VERSION: u8 = 1;

/// Maximum size of a single message payload, in bytes.
/// A 1920x1200 JPEG at quality 90 is under 1 MB; an H.264 access unit is far
/// smaller. 4 MB leaves room for a future codec without letting a peer make us
/// buffer an arbitrary amount.
pub const MAX_PAYLOAD: usize = 4 * 1024 * 1024;

/// Maximum payload for handshake messages.
pub const MAX_HANDSHAKE_PAYLOAD: usize = 1024;

/// Maximum payload for input messages.
pub const MAX_INPUT_PAYLOAD: usize = 4 * 1024;

/// Maximum payload for a frame message.
pub const MAX_FRAME_PAYLOAD: usize = 4 * 1024 * 1024;

/// Maximum payload for an ack message.
pub const MAX_ACK_PAYLOAD: usize = 64;

/// Maximum bytes for the server or client display name.
pub const MAX_NAME_LEN: usize = 64;

/// Maximum number of frames the sender may have in flight.
pub const MAX_WINDOW_FRAMES: u64 = 8;

/// Maximum bytes the sender may have in flight.
pub const MAX_WINDOW_BYTES: u64 = 2_000_000;

/// Maximum length of the error message carried by an ERROR message.
pub const MAX_ERROR_TEXT: usize = 255;

/// Number of digits in the pairing safety code shown on both devices.
pub const SAS_DIGITS: u32 = 6;

/// The biggest payload a message of this type may carry.
///
/// The limit is applied before the payload is read, so a peer cannot make
/// the other end reserve more memory than the message type allows.
pub fn max_payload(kind: crate::frame::MessageType) -> usize {
    use crate::frame::MessageType;
    match kind {
        MessageType::Hello | MessageType::Challenge | MessageType::Auth | MessageType::AuthOk => {
            MAX_HANDSHAKE_PAYLOAD
        }
        MessageType::Error => MAX_ACK_PAYLOAD,
        MessageType::DisplayInfo => MAX_HANDSHAKE_PAYLOAD,
        MessageType::Frame => MAX_FRAME_PAYLOAD,
        MessageType::FrameAck | MessageType::Heartbeat | MessageType::HeartbeatAck => {
            MAX_ACK_PAYLOAD
        }
        MessageType::InputMouse | MessageType::InputKey | MessageType::InputTouch => {
            MAX_INPUT_PAYLOAD
        }
        MessageType::Close => MAX_ACK_PAYLOAD,
    }
}
