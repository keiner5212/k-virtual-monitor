//! Cryptographic primitives for the session.
//!
//! The pairing secret is a 32 byte PSK shared out of band. From it we derive
//! one key per direction. Every message after the handshake is sealed with
//! ChaCha20-Poly1305, using the header as associated data, so the type, the
//! sequence number and the length cannot be altered on the wire.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::ProtocolError;

pub const PSK_LEN: usize = 32;
pub const NONCE_LEN: usize = 32;
pub const KEY_LEN: usize = 32;
pub const TAG_LEN: usize = 16;
const AEAD_NONCE_LEN: usize = 12;

/// Pre-shared key established during pairing.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Psk([u8; PSK_LEN]);

impl Psk {
    pub fn from_bytes(bytes: [u8; PSK_LEN]) -> Self {
        Psk(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != PSK_LEN {
            return Err(ProtocolError::Handshake("psk must be 32 bytes"));
        }
        let mut key = [0u8; PSK_LEN];
        key.copy_from_slice(bytes);
        Ok(Psk(key))
    }

    pub fn generate() -> Self {
        let mut key = [0u8; PSK_LEN];
        rand::rng().fill_bytes(&mut key);
        Psk(key)
    }

    pub fn as_bytes(&self) -> &[u8; PSK_LEN] {
        &self.0
    }
}

/// Per-direction session key.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SessionKey([u8; KEY_LEN]);

impl SessionKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        SessionKey(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

/// The two keys of a session, one per direction.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SessionKeys {
    pub client_to_server: SessionKey,
    pub server_to_client: SessionKey,
}

/// Derives the session keys from the pairing secret and both nonces.
///
/// The two directions use different `info` labels, so a key is never reused
/// for two purposes. The nonces are per connection, so a reconnect produces a
/// different key pair and the AEAD nonce space may restart.
pub fn derive_session_keys(
    psk: &Psk,
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
) -> SessionKeys {
    let mut salt = [0u8; NONCE_LEN * 2];
    salt[..NONCE_LEN].copy_from_slice(client_nonce);
    salt[NONCE_LEN..].copy_from_slice(server_nonce);
    let hk = Hkdf::<Sha256>::new(Some(&salt), psk.as_bytes());
    let mut c2s = [0u8; KEY_LEN];
    let mut s2c = [0u8; KEY_LEN];
    hk.expand(b"kvm-session-v1|c2s", &mut c2s)
        .expect("32 bytes is a valid HKDF output length");
    hk.expand(b"kvm-session-v1|s2c", &mut s2c)
        .expect("32 bytes is a valid HKDF output length");
    SessionKeys {
        client_to_server: SessionKey::from_bytes(c2s),
        server_to_client: SessionKey::from_bytes(s2c),
    }
}

/// Computes the handshake proof for one direction.
pub fn proof(psk: &Psk, label: &[u8], transcript: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(psk.as_bytes()).expect("hmac accepts any key length");
    mac.update(label);
    mac.update(transcript);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

/// Compares two proofs in constant time.
pub fn proofs_equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.ct_eq(b).into()
}

/// Builds the AEAD nonce for a message: one direction bit plus the message
/// sequence number. The sequence number is strictly monotonic inside a
/// session, and every session has fresh keys, so a nonce is never reused.
pub fn aead_nonce(direction_client_to_server: bool, seq: u64) -> [u8; AEAD_NONCE_LEN] {
    let mut nonce = [0u8; AEAD_NONCE_LEN];
    nonce[0] = u8::from(direction_client_to_server);
    nonce[1..9].copy_from_slice(&seq.to_be_bytes());
    nonce
}

/// Seals a payload. The output is `ciphertext || tag`.
pub fn seal(
    key: &SessionKey,
    direction_client_to_server: bool,
    seq: u64,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let raw_nonce = aead_nonce(direction_client_to_server, seq);
    let nonce = Nonce::from_slice(&raw_nonce);
    cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| ProtocolError::Handshake("aead encryption failed"))
}

/// Opens a sealed payload.
pub fn open(
    key: &SessionKey,
    direction_client_to_server: bool,
    seq: u64,
    aad: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    if sealed.len() < TAG_LEN {
        return Err(ProtocolError::PayloadSize {
            kind: "sealed payload",
            actual: sealed.len(),
            expected: TAG_LEN,
        });
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let raw_nonce = aead_nonce(direction_client_to_server, seq);
    let nonce = Nonce::from_slice(&raw_nonce);
    cipher
        .decrypt(nonce, Payload { msg: sealed, aad })
        .map_err(|_| ProtocolError::TagMismatch {
            kind: crate::frame::MessageType::Frame,
        })
}

/// Six digit safety code derived from the pairing secret. Both devices show
/// it so the user can confirm they paired with the right peer.
pub fn safety_code(psk: &Psk) -> u32 {
    let digest = Sha256::digest(psk.as_bytes());
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    value % 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_keys_differ_per_direction() {
        let psk = Psk::generate();
        let c = [1u8; NONCE_LEN];
        let s = [2u8; NONCE_LEN];
        let keys = derive_session_keys(&psk, &c, &s);
        assert_ne!(
            keys.client_to_server.as_bytes(),
            keys.server_to_client.as_bytes()
        );
    }

    #[test]
    fn different_nonces_give_different_keys() {
        let psk = Psk::generate();
        let c = [1u8; NONCE_LEN];
        let a = derive_session_keys(&psk, &c, &[2u8; NONCE_LEN]);
        let b = derive_session_keys(&psk, &c, &[3u8; NONCE_LEN]);
        assert_ne!(a.client_to_server.as_bytes(), b.client_to_server.as_bytes());
    }

    #[test]
    fn seal_and_open_round_trip() {
        let key = SessionKey::from_bytes([7u8; KEY_LEN]);
        let aad = b"header-bytes";
        let sealed = seal(&key, true, 9, aad, b"payload").unwrap();
        let opened = open(&key, true, 9, aad, &sealed).unwrap();
        assert_eq!(opened, b"payload");
    }

    #[test]
    fn a_modified_payload_fails_to_open() {
        let key = SessionKey::from_bytes([7u8; KEY_LEN]);
        let aad = b"header-bytes";
        let mut sealed = seal(&key, true, 9, aad, b"payload").unwrap();
        sealed[0] ^= 0xff;
        assert!(open(&key, true, 9, aad, &sealed).is_err());
    }

    #[test]
    fn a_modified_header_fails_to_open() {
        let key = SessionKey::from_bytes([7u8; KEY_LEN]);
        let sealed = seal(&key, true, 9, b"header-bytes", b"payload").unwrap();
        assert!(open(&key, true, 9, b"other-header", &sealed).is_err());
    }

    #[test]
    fn a_different_sequence_fails_to_open() {
        let key = SessionKey::from_bytes([7u8; KEY_LEN]);
        let aad = b"header-bytes";
        let sealed = seal(&key, true, 9, aad, b"payload").unwrap();
        assert!(open(&key, true, 10, aad, &sealed).is_err());
    }

    #[test]
    fn safety_code_is_stable_and_six_digits() {
        let psk = Psk::from_bytes([3u8; PSK_LEN]);
        let code = safety_code(&psk);
        assert_eq!(code, safety_code(&psk));
        assert!(code < 1_000_000);
    }
}
