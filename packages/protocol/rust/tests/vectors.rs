//! The golden vectors check.
//!
//! `packages/protocol/testdata/vectors.json` is produced by the Rust
//! implementation and consumed by the TypeScript one. Both must agree, so
//! the two ends of a session cannot drift apart silently.

use kvm_protocol::crypto::{derive_session_keys, proof, Psk};
use kvm_protocol::frame::{Header, MessageType};
use kvm_protocol::handshake::{client_transcript, server_transcript};
use kvm_protocol::session::Session;

const VECTORS: &str = include_str!("../../testdata/vectors.json");

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn unhex_array<const N: usize>(text: &str) -> [u8; N] {
    unhex(text).try_into().expect("fixed size")
}

#[test]
fn rust_matches_the_golden_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(VECTORS).expect("valid json");

    let psk = Psk::from_bytes(unhex_array::<32>(vectors["psk"].as_str().unwrap()));
    let client_nonce = unhex_array::<32>(vectors["clientNonce"].as_str().unwrap());
    let server_nonce = unhex_array::<32>(vectors["serverNonce"].as_str().unwrap());
    let device_id = unhex_array::<16>(vectors["deviceId"].as_str().unwrap());

    let client_proof = proof(
        &psk,
        b"kvm-auth-v1|c2s",
        &client_transcript(&client_nonce, &server_nonce, &device_id),
    );
    assert_eq!(hex(&client_proof), vectors["clientProof"].as_str().unwrap());

    let server_proof = proof(
        &psk,
        b"kvm-auth-v1|s2c",
        &server_transcript(&client_nonce, &server_nonce),
    );
    assert_eq!(hex(&server_proof), vectors["serverProof"].as_str().unwrap());

    let keys = derive_session_keys(&psk, &client_nonce, &server_nonce);
    assert_eq!(
        hex(keys.client_to_server.as_bytes()),
        vectors["sessionKeyC2s"].as_str().unwrap()
    );
    assert_eq!(
        hex(keys.server_to_client.as_bytes()),
        vectors["sessionKeyS2c"].as_str().unwrap()
    );

    let frame = &vectors["sealedFrame"];
    let mut session = Session::client(kvm_protocol::crypto::derive_session_keys(
        &psk,
        &client_nonce,
        &server_nonce,
    ));
    let (header, sealed) = session
        .seal(
            MessageType::from_u8(frame["type"].as_u64().unwrap() as u8).unwrap(),
            frame["flags"].as_u64().unwrap() as u16,
            &unhex(frame["plaintext"].as_str().unwrap()),
        )
        .expect("seal");
    assert_eq!(hex(&header.encode()), frame["header"].as_str().unwrap());
    assert_eq!(hex(&sealed), frame["payload"].as_str().unwrap());

    let expected_header =
        Header::decode(&unhex_array::<18>(frame["header"].as_str().unwrap())).unwrap();
    assert_eq!(expected_header, header);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
