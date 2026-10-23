//! The authenticated session.
//!
//! After the handshake every message is sealed with ChaCha20-Poly1305. The
//! header is the associated data, so the type, the sequence number and the
//! length are authenticated together with the payload.
//!
//! The sequence number starts at 1 and increases by exactly one per message
//! in each direction. A gap means the peer is broken or an attacker is
//! injecting, so the session is closed instead of resynchronising.

use std::collections::BTreeMap;

use crate::crypto::{self, SessionKey, SessionKeys, TAG_LEN};
use crate::error::ProtocolError;
use crate::frame::{Header, MessageType};
use crate::limits::{MAX_WINDOW_BYTES, MAX_WINDOW_FRAMES};

/// One direction of an established session.
///
/// The session knows which direction it sends and which direction it
/// receives, because the AEAD nonce carries a direction bit. The client
/// sends with the client to server key and receives with the server to
/// client key, and the other way around for the server.
pub struct Session {
    send_key: SessionKey,
    recv_key: SessionKey,
    outbound_client_to_server: bool,
    inbound_client_to_server: bool,
    next_send_seq: u64,
    next_recv_seq: u64,
}

impl Session {
    /// The client side: sends with the client to server key.
    pub fn client(keys: SessionKeys) -> Self {
        Session {
            send_key: keys.client_to_server.clone(),
            recv_key: keys.server_to_client.clone(),
            outbound_client_to_server: true,
            inbound_client_to_server: false,
            next_send_seq: 1,
            next_recv_seq: 1,
        }
    }

    /// The server side: sends with the server to client key.
    pub fn server(keys: SessionKeys) -> Self {
        Session {
            send_key: keys.server_to_client.clone(),
            recv_key: keys.client_to_server.clone(),
            outbound_client_to_server: false,
            inbound_client_to_server: true,
            next_send_seq: 1,
            next_recv_seq: 1,
        }
    }

    /// Seals a message. The returned payload is `ciphertext || tag`.
    pub fn seal(
        &mut self,
        kind: MessageType,
        flags: u16,
        plaintext: &[u8],
    ) -> Result<(Header, Vec<u8>), ProtocolError> {
        let seq = self.next_send_seq;
        let header = Header::new(kind, seq, flags, plaintext.len() + TAG_LEN);
        let aad = header.encode();
        let sealed = crypto::seal(
            &self.send_key,
            self.outbound_client_to_server,
            seq,
            &aad,
            plaintext,
        )?;
        self.next_send_seq += 1;
        Ok((header, sealed))
    }

    /// Opens a sealed message and checks the sequence number.
    pub fn open(&mut self, header: &Header, sealed: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        if header.seq != self.next_recv_seq {
            return Err(ProtocolError::BadSequence {
                expected: self.next_recv_seq,
                received: header.seq,
            });
        }
        let aad = header.encode();
        let plaintext = crypto::open(
            &self.recv_key,
            self.inbound_client_to_server,
            header.seq,
            &aad,
            sealed,
        )?;
        self.next_recv_seq += 1;
        Ok(plaintext)
    }

    pub fn next_send_seq(&self) -> u64 {
        self.next_send_seq
    }
}

/// Flow control window for the frame stream.
///
/// The window is the number of frames and the number of bytes the sender may
/// have outstanding. It is large enough that the frame rate is not bounded by
/// the round trip time, and small enough that a stalled client cannot make the
/// server buffer an unbounded backlog.
pub struct SendWindow {
    max_frames: u64,
    max_bytes: u64,
    in_flight: BTreeMap<u64, u64>,
    in_flight_bytes: u64,
}

impl Default for SendWindow {
    fn default() -> Self {
        SendWindow::new(MAX_WINDOW_FRAMES, MAX_WINDOW_BYTES)
    }
}

impl SendWindow {
    pub fn new(max_frames: u64, max_bytes: u64) -> Self {
        SendWindow {
            max_frames,
            max_bytes,
            in_flight: BTreeMap::new(),
            in_flight_bytes: 0,
        }
    }

    pub fn in_flight_frames(&self) -> u64 {
        self.in_flight.len() as u64
    }

    pub fn in_flight_bytes(&self) -> u64 {
        self.in_flight_bytes
    }

    /// True when a frame of `bytes` fits in the window.
    pub fn can_send(&self, bytes: u64) -> bool {
        (self.in_flight.len() as u64) < self.max_frames
            && self.in_flight_bytes + bytes <= self.max_bytes
    }

    /// Records a frame as sent.
    pub fn push(&mut self, seq: u64, bytes: u64) {
        self.in_flight.insert(seq, bytes);
        self.in_flight_bytes += bytes;
    }

    /// Releases every frame up to and including `acked_seq`.
    pub fn ack(&mut self, acked_seq: u64) -> u64 {
        let mut released = 0;
        let done: Vec<u64> = self
            .in_flight
            .range(..=acked_seq)
            .map(|(seq, _)| *seq)
            .collect();
        for seq in done {
            if let Some(bytes) = self.in_flight.remove(&seq) {
                self.in_flight_bytes -= bytes;
                released += bytes;
            }
        }
        released
    }

    /// Drops the whole backlog, used when the link is considered dead.
    pub fn clear(&mut self) {
        self.in_flight.clear();
        self.in_flight_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Psk;

    fn session_pair() -> (Session, Session) {
        let psk = Psk::generate();
        let keys = crypto::derive_session_keys(&psk, &[1u8; 32], &[2u8; 32]);
        (Session::client(keys.clone()), Session::server(keys.clone()))
    }

    #[test]
    fn sealed_message_opens_on_the_other_side() {
        let (mut client, mut server) = session_pair();
        let (header, sealed) = client.seal(MessageType::Frame, 0, b"frame-bytes").unwrap();
        let plaintext = server.open(&header, &sealed).unwrap();
        assert_eq!(plaintext, b"frame-bytes");
    }

    #[test]
    fn a_replayed_sequence_is_rejected() {
        let (mut client, mut server) = session_pair();
        let (header, sealed) = client.seal(MessageType::Frame, 0, b"a").unwrap();
        server.open(&header, &sealed).unwrap();
        assert!(matches!(
            server.open(&header, &sealed),
            Err(ProtocolError::BadSequence { .. })
        ));
    }

    #[test]
    fn a_skipped_sequence_is_rejected() {
        let (mut client, mut server) = session_pair();
        let (header, sealed) = client.seal(MessageType::Frame, 0, b"a").unwrap();
        let mut skipped = header;
        skipped.seq += 1;
        assert!(matches!(
            server.open(&skipped, &sealed),
            Err(ProtocolError::BadSequence { .. })
        ));
    }

    #[test]
    fn window_blocks_when_full_and_drains_on_ack() {
        let mut window = SendWindow::new(2, 1000);
        assert!(window.can_send(100));
        window.push(1, 100);
        assert!(window.can_send(100));
        window.push(2, 100);
        assert!(!window.can_send(100));
        window.ack(1);
        assert_eq!(window.in_flight_frames(), 1);
        assert_eq!(window.in_flight_bytes(), 100);
        assert!(window.can_send(100));
    }

    #[test]
    fn window_respects_the_byte_cap() {
        let mut window = SendWindow::new(8, 1000);
        window.push(1, 900);
        assert!(!window.can_send(200));
        assert!(window.can_send(50));
    }
}
