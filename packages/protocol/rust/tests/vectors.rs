//! The golden vectors check.
//!
//! `packages/protocol/testdata/vectors.json` is produced by the Rust
//! implementation and consumed by the TypeScript one. Both must agree, so
//! the two ends of a session cannot drift apart silently.

use kvm_protocol::crypto::{derive_session_keys, proof, safety_code, Psk};
use kvm_protocol::frame::{Header, MessageType};
use kvm_protocol::handshake::{client_transcript, server_transcript};
use kvm_protocol::messages::{
    caps, codec, Close, DisplayInfo, ErrorInfo, FrameAck, FrameHeader, Heartbeat, Hello, Key,
    Mouse, Touch,
};
use kvm_protocol::pairing::PairingBlob;
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn value() -> serde_json::Value {
    serde_json::from_str(VECTORS).expect("valid json")
}

#[test]
fn the_keys_and_the_proofs_match_the_vectors() {
    let vectors = value();
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

    assert_eq!(
        safety_code(&psk) as u64,
        vectors["safetyCode"].as_u64().unwrap()
    );
}

#[test]
fn the_sealed_messages_match_the_vectors() {
    let vectors = value();
    let keys = derive_session_keys(
        &Psk::from_bytes(unhex_array::<32>(vectors["psk"].as_str().unwrap())),
        &unhex_array::<32>(vectors["clientNonce"].as_str().unwrap()),
        &unhex_array::<32>(vectors["serverNonce"].as_str().unwrap()),
    );

    let mut client = Session::client(keys.clone());
    for name in ["sealedFrame", "sealedFrame2"] {
        let expected = &vectors[name];
        let (header, sealed) = client
            .seal(
                MessageType::Frame,
                0,
                &unhex(expected["plaintext"].as_str().unwrap()),
            )
            .expect("seal");
        assert_eq!(hex(&header.encode()), expected["header"].as_str().unwrap());
        assert_eq!(hex(&sealed), expected["payload"].as_str().unwrap());
    }

    let expected = &vectors["sealedFrameServerToClient"];
    let mut server = Session::server(keys);
    let (header, sealed) = server
        .seal(
            MessageType::Frame,
            0,
            &unhex(expected["plaintext"].as_str().unwrap()),
        )
        .expect("seal");
    assert_eq!(hex(&header.encode()), expected["header"].as_str().unwrap());
    assert_eq!(hex(&sealed), expected["payload"].as_str().unwrap());
}

#[test]
fn the_payload_layouts_match_the_vectors() {
    let vectors = value();
    let payloads = &vectors["payloads"];
    let psk = Psk::from_bytes(unhex_array::<32>(vectors["psk"].as_str().unwrap()));
    let device_id = unhex_array::<16>(vectors["deviceId"].as_str().unwrap());

    let hello = Hello {
        client_nonce: unhex_array::<32>(vectors["clientNonce"].as_str().unwrap()),
        device_id,
        device_name: "phone".to_string(),
        max_width: 1408,
        max_height: 792,
        max_fps: 30,
        codec_mask: codec::H264 | codec::JPEG,
        caps_requested: caps::VIEW | caps::CONTROL,
    };
    assert_eq!(hex(&hello.encode()), payloads["hello"].as_str().unwrap());

    let display_info = DisplayInfo {
        width: 1408,
        height: 792,
        fps: 30,
        codec: codec::wire_value(codec::JPEG),
        granted_caps: caps::VIEW | caps::CONTROL,
        server_name: "desktop".to_string(),
    };
    assert_eq!(
        hex(&display_info.encode()),
        payloads["displayInfo"].as_str().unwrap()
    );

    let frame_header = FrameHeader {
        pts_ms: 1234,
        keyframe: true,
        width: 1408,
        height: 792,
        codec: codec::wire_value(codec::JPEG),
    };
    assert_eq!(
        hex(&frame_header.encode()),
        payloads["frameHeader"].as_str().unwrap()
    );
    assert_eq!(frame_header.encode().len(), FrameHeader::LEN);

    let ack = FrameAck {
        acked_seq: 12,
        acked_bytes: 4096,
    };
    assert_eq!(hex(&ack.encode()), payloads["frameAck"].as_str().unwrap());

    let mouse = Mouse {
        x: 100,
        y: 200,
        buttons: 1,
        pressed: true,
        wheel: -1,
    };
    assert_eq!(hex(&mouse.encode()), payloads["mouse"].as_str().unwrap());

    let key = Key {
        keycode: 42,
        pressed: true,
        modifiers: 3,
    };
    assert_eq!(hex(&key.encode()), payloads["key"].as_str().unwrap());

    let touch = Touch {
        pointer_id: 1,
        action: 2,
        x: 100,
        y: 200,
        pressure: 1024,
    };
    assert_eq!(hex(&touch.encode()), payloads["touch"].as_str().unwrap());

    let heartbeat = Heartbeat {
        client_time_ms: 123456789,
    };
    assert_eq!(
        hex(&heartbeat.encode()),
        payloads["heartbeat"].as_str().unwrap()
    );

    let close = Close { reason: 0 };
    assert_eq!(hex(&close.encode()), payloads["close"].as_str().unwrap());

    let error = ErrorInfo {
        code: 5,
        text: "boom".to_string(),
    };
    assert_eq!(hex(&error.encode()), payloads["error"].as_str().unwrap());

    let blob = PairingBlob::new(device_id, psk, "desktop", "192.168.1.10", 28252);
    assert_eq!(blob.encode(), vectors["pairingBlob"].as_str().unwrap());
}

#[test]
fn the_codec_preference_matches_the_vectors() {
    let vectors = value();
    let expected: Vec<u16> = vectors["codecPreference"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|value| value.as_u64().expect("a number") as u16)
        .collect();
    assert_eq!(codec::PREFERENCE.to_vec(), expected);
    assert_eq!(
        codec::negotiate(codec::H264 | codec::JPEG, codec::H264 | codec::JPEG),
        Some(codec::H264)
    );
    assert_eq!(codec::negotiate(codec::AV1, codec::JPEG), None);
}

#[test]
fn the_header_layout_matches_the_vectors() {
    let vectors = value();
    let expected = vectors["sealedFrame"]["header"].as_str().unwrap();
    let header = Header::new(MessageType::Frame, 1, 0, 32);
    assert_eq!(hex(&header.encode()), expected);
}
