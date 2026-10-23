//! kvm-protocol: the wire protocol shared by the desktop server and the phone
//! client.
//!
//! The protocol is ours end to end. It does not implement the spacedesk wire
//! format and does not need to: both ends of a session are our own software.
//!
//! Layers, bottom up:
//!
//! * [`frame`] - the 18 byte header and the length checked reader and writer.
//! * [`crypto`] - HKDF, HMAC and ChaCha20-Poly1305, all from RustCrypto.
//! * [`handshake`] - pairing based mutual authentication.
//! * [`session`] - the sealed message channel and the frame send window.
//! * [`messages`] - the payload of every message type.
//!
//! The same protocol is implemented in TypeScript in `packages/protocol/ts`.
//! Both implementations are checked against the same golden vectors.

pub mod crypto;
pub mod error;
pub mod frame;
pub mod handshake;
pub mod limits;
pub mod messages;
pub mod pairing;
pub mod session;

pub use error::{ErrorCode, ProtocolError};
pub use frame::{Header, MessageType, HEADER_LEN};
pub use pairing::PairingBlob;
pub use session::{SendWindow, Session};
