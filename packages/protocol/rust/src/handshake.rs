//! The pairing handshake.
//!
//! Four messages, two round trips, mutual authentication with a pre-shared
//! key. Both nonces are fresh per connection, so the transcript is unique and
//! a captured handshake cannot be replayed.
//!
//! ```text
//! client -> server   HELLO      client_nonce, device_id, capabilities
//! server -> client   CHALLENGE  server_nonce
//! client -> server   AUTH       HMAC(PSK, "kvm-auth-v1|c2s", transcript)
//! server -> client   AUTH_OK    HMAC(PSK, "kvm-auth-v1|s2c", transcript)
//! ```
//!
//! The transcript for the client proof is
//! `client_nonce || server_nonce || device_id`; for the server proof it is
//! `client_nonce || server_nonce`. The HMAC key is the pairing secret, so a
//! peer that does not know it cannot produce either proof.

use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::crypto::{self, Psk, SessionKeys, NONCE_LEN};
use crate::error::ProtocolError;
use crate::frame::{read_message, write_message, Header, MessageType};
use crate::limits::MAX_HANDSHAKE_PAYLOAD;
use crate::messages::{Auth, AuthOk, Challenge, Hello};

const CLIENT_PROOF_LABEL: &[u8] = b"kvm-auth-v1|c2s";
const SERVER_PROOF_LABEL: &[u8] = b"kvm-auth-v1|s2c";

/// Transcript signed by the client. It binds both nonces and the device id.
pub fn client_transcript(
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
    device_id: &[u8; 16],
) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(NONCE_LEN * 2 + 16);
    transcript.extend_from_slice(client_nonce);
    transcript.extend_from_slice(server_nonce);
    transcript.extend_from_slice(device_id);
    transcript
}

/// Transcript signed by the server. It binds both nonces.
pub fn server_transcript(
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(NONCE_LEN * 2);
    transcript.extend_from_slice(client_nonce);
    transcript.extend_from_slice(server_nonce);
    transcript
}

/// Server side of the handshake.
pub async fn server_handshake<S>(
    stream: &mut S,
    psk: &Psk,
    server_name: &str,
) -> Result<(SessionKeys, Hello), ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    server_handshake_with(stream, server_name, |_| Some(psk.clone())).await
}

/// Server side of the handshake, resolving the pairing secret from the
/// greeting. The server learns which secret to use from the device id the
/// client presents, so the device store stays outside this package.
pub async fn server_handshake_with<S, F>(
    stream: &mut S,
    server_name: &str,
    mut resolve: F,
) -> Result<(SessionKeys, Hello), ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(&Hello) -> Option<Psk>,
{
    let (header, payload) = read_message(stream, MAX_HANDSHAKE_PAYLOAD).await?;
    if header.kind != MessageType::Hello {
        return Err(ProtocolError::Handshake("expected HELLO"));
    }
    let hello = Hello::decode(&payload)?;
    let psk = resolve(&hello).ok_or(ProtocolError::UnknownDevice(hello.device_id))?;

    let mut server_nonce = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut server_nonce);
    let challenge = Challenge {
        server_nonce,
        server_name: server_name.to_string(),
    };
    let header = Header::new(MessageType::Challenge, 1, 0, challenge.encode().len());
    write_message(stream, &header, &challenge.encode()).await?;

    let (header, payload) = read_message(stream, MAX_HANDSHAKE_PAYLOAD).await?;
    if header.kind != MessageType::Auth {
        return Err(ProtocolError::Handshake("expected AUTH"));
    }
    let auth = Auth::decode(&payload)?;
    let expected = crypto::proof(
        &psk,
        CLIENT_PROOF_LABEL,
        &client_transcript(&hello.client_nonce, &server_nonce, &hello.device_id),
    );
    if !crypto::proofs_equal(&expected, &auth.proof) {
        return Err(ProtocolError::ProofMismatch {
            kind: MessageType::Auth,
        });
    }

    let keys = crypto::derive_session_keys(&psk, &hello.client_nonce, &server_nonce);
    let auth_ok = AuthOk {
        server_proof: crypto::proof(
            &psk,
            SERVER_PROOF_LABEL,
            &server_transcript(&hello.client_nonce, &server_nonce),
        ),
    };
    let header = Header::new(MessageType::AuthOk, 2, 0, auth_ok.encode().len());
    write_message(stream, &header, &auth_ok.encode()).await?;

    Ok((keys, hello))
}

/// Client side of the handshake.
pub async fn client_handshake<S>(
    stream: &mut S,
    psk: &Psk,
    hello: &Hello,
) -> Result<SessionKeys, ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let header = Header::new(MessageType::Hello, 1, 0, hello.encode().len());
    write_message(stream, &header, &hello.encode()).await?;

    let (header, payload) = read_message(stream, MAX_HANDSHAKE_PAYLOAD).await?;
    if header.kind != MessageType::Challenge {
        return Err(ProtocolError::Handshake("expected CHALLENGE"));
    }
    let challenge = Challenge::decode(&payload)?;

    let auth = Auth {
        proof: crypto::proof(
            psk,
            CLIENT_PROOF_LABEL,
            &client_transcript(
                &hello.client_nonce,
                &challenge.server_nonce,
                &hello.device_id,
            ),
        ),
    };
    let header = Header::new(MessageType::Auth, 2, 0, auth.encode().len());
    write_message(stream, &header, &auth.encode()).await?;

    let (header, payload) = read_message(stream, MAX_HANDSHAKE_PAYLOAD).await?;
    if header.kind != MessageType::AuthOk {
        return Err(ProtocolError::Handshake("expected AUTH_OK"));
    }
    let auth_ok = AuthOk::decode(&payload)?;
    let expected = crypto::proof(
        psk,
        SERVER_PROOF_LABEL,
        &server_transcript(&hello.client_nonce, &challenge.server_nonce),
    );
    if !crypto::proofs_equal(&expected, &auth_ok.server_proof) {
        return Err(ProtocolError::ProofMismatch {
            kind: MessageType::AuthOk,
        });
    }

    Ok(crypto::derive_session_keys(
        psk,
        &hello.client_nonce,
        &challenge.server_nonce,
    ))
}
