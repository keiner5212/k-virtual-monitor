//! End to end test of a session: a paired client connects, completes the
//! handshake and receives frames over a real TCP socket.

use std::sync::Arc;

use kvm_desktop_lib::config::ServerConfig;
use kvm_desktop_lib::devices::{DeviceEntry, DeviceStore, CAP_VIEW};
use kvm_desktop_lib::server::Server;
use kvm_desktop_lib::source::SyntheticSource;
use kvm_protocol::crypto::Psk;
use kvm_protocol::frame::{read_message, write_message, MessageType};
use kvm_protocol::handshake::client_handshake;
use kvm_protocol::limits::MAX_WINDOW_FRAMES;
use kvm_protocol::messages::{codec, FrameAck, Hello};
use kvm_protocol::session::Session;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Fixture {
    port: u16,
    psk: Psk,
    device_id: [u8; 16],
    dir: std::path::PathBuf,
}

async fn start() -> Fixture {
    let dir = std::env::temp_dir().join(format!("kvm-session-{}", std::process::id()));
    let store_path = dir.join("devices.json");

    let psk = Psk::generate();
    let device_id = [7u8; 16];
    let mut store = DeviceStore::default();
    store.upsert(DeviceEntry {
        id: hex(&device_id),
        name: "phone".to_string(),
        psk: hex(psk.as_bytes()),
        caps: CAP_VIEW,
        paired: true,
        revoked: false,
        created_at: "test".to_string(),
    });

    let config = ServerConfig {
        port: 0,
        bind: "127.0.0.1".to_string(),
        width: 320,
        height: 240,
        fps: 20,
        ..ServerConfig::default()
    };
    let server = Arc::new(Server::new(
        config,
        store,
        store_path,
        Arc::new(SyntheticSource::new(320, 240, 70)),
    ));
    let listener = server.bind().await.expect("the listener binds");
    let port = listener
        .local_addr()
        .expect("the listener has an address")
        .port();
    let serving = server.clone();
    tokio::spawn(async move {
        let _ = serving.serve(listener).await;
    });

    Fixture {
        port,
        psk,
        device_id,
        dir,
    }
}

fn hello(device_id: [u8; 16]) -> Hello {
    Hello {
        client_nonce: [3u8; 32],
        device_id,
        device_name: "phone".to_string(),
        max_width: 320,
        max_height: 240,
        max_fps: 20,
        codec_mask: codec::JPEG,
        caps_requested: CAP_VIEW,
    }
}

#[tokio::test]
async fn a_paired_client_completes_the_handshake_and_receives_frames() {
    let fixture = start().await;
    let mut stream = TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .expect("the client connects");

    let keys = client_handshake(&mut stream, &fixture.psk, &hello(fixture.device_id))
        .await
        .expect("the handshake succeeds");
    let mut session = Session::client(keys);

    let (header, payload) = read_message(&mut stream)
        .await
        .expect("the display info arrives");
    assert_eq!(header.kind, MessageType::DisplayInfo);
    let plaintext = session.open(&header, &payload).expect("the info opens");
    let info = kvm_protocol::messages::DisplayInfo::decode(&plaintext).expect("the info decodes");
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert_eq!(info.granted_caps, CAP_VIEW);

    let (header, payload) = read_message(&mut stream).await.expect("a frame arrives");
    assert_eq!(header.kind, MessageType::Frame);
    let plaintext = session.open(&header, &payload).expect("the frame opens");
    assert!(plaintext.len() > 100);
    let header_len = kvm_protocol::messages::FrameHeader::LEN;
    assert_eq!(
        &plaintext[header_len..header_len + 2],
        &[0xff, 0xd8],
        "the frame carries a JPEG"
    );

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn a_client_with_the_wrong_secret_is_refused() {
    let fixture = start().await;
    let mut stream = TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .expect("the client connects");

    let wrong = Psk::generate();
    let result = client_handshake(&mut stream, &wrong, &hello(fixture.device_id)).await;
    assert!(result.is_err(), "the handshake must fail");

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn an_unknown_device_is_refused() {
    let fixture = start().await;
    let mut stream = TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .expect("the client connects");

    let result = client_handshake(&mut stream, &fixture.psk, &hello([9u8; 16])).await;
    assert!(result.is_err(), "the handshake must fail");

    std::fs::remove_dir_all(&fixture.dir).ok();
}

/// Connects and completes the handshake, returning the session.
async fn connect(fixture: &Fixture) -> (TcpStream, Session) {
    let mut stream = TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .expect("the client connects");
    let keys = client_handshake(&mut stream, &fixture.psk, &hello(fixture.device_id))
        .await
        .expect("the handshake succeeds");
    let mut session = Session::client(keys);
    let (header, payload) = read_message(&mut stream)
        .await
        .expect("the display info arrives");
    session.open(&header, &payload).expect("the info opens");
    (stream, session)
}

#[tokio::test]
async fn a_tampered_message_closes_the_session() {
    let fixture = start().await;
    let (mut stream, mut session) = connect(&fixture).await;

    let payload = FrameAck {
        acked_seq: 1,
        acked_bytes: 1,
    }
    .encode();
    let (header, mut sealed) = session
        .seal(MessageType::FrameAck, 0, &payload)
        .expect("the ack is sealed");
    sealed[0] ^= 0xff;
    stream
        .write_all(&header.encode())
        .await
        .expect("the header is written");
    stream
        .write_all(&sealed)
        .await
        .expect("the payload is written");

    // The server must verify the tag before acting on the message, so the
    // session ends instead of the acknowledgement being counted.
    let mut buffer = Vec::new();
    let closed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_to_end(&mut buffer),
    )
    .await;
    assert!(closed.is_ok(), "the server must close the session");

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn a_tampered_header_closes_the_session() {
    let fixture = start().await;
    let (mut stream, mut session) = connect(&fixture).await;

    let payload = FrameAck {
        acked_seq: 1,
        acked_bytes: 1,
    }
    .encode();
    let (header, sealed) = session
        .seal(MessageType::FrameAck, 0, &payload)
        .expect("the ack is sealed");
    // The type is part of the associated data, so changing it breaks the tag.
    let mut raw = header.encode();
    raw[3] = MessageType::Close as u8;
    stream.write_all(&raw).await.expect("the header is written");
    stream
        .write_all(&sealed)
        .await
        .expect("the payload is written");

    let mut buffer = Vec::new();
    let closed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_to_end(&mut buffer),
    )
    .await;
    assert!(closed.is_ok(), "the server must close the session");

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn the_window_stops_the_server_when_the_client_does_not_acknowledge() {
    let fixture = start().await;
    let (mut stream, _session) = connect(&fixture).await;

    let mut frames = 0u64;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(Ok((header, _payload))) =
            tokio::time::timeout(remaining, read_message(&mut stream)).await
        else {
            break;
        };
        if header.kind == MessageType::Frame {
            frames += 1;
        }
    }

    // The window holds eight frames. Without acknowledgements the server may
    // not send more than that, however fast the source produces frames.
    assert!(
        frames <= MAX_WINDOW_FRAMES,
        "the server sent {frames} frames without an acknowledgement"
    );

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn an_acknowledgement_drains_the_window() {
    let fixture = start().await;
    let (mut stream, mut session) = connect(&fixture).await;

    let mut highest;
    let mut sent = 0u64;
    for _ in 0..MAX_WINDOW_FRAMES + 2 {
        let Ok(Ok((header, payload))) =
            tokio::time::timeout(std::time::Duration::from_secs(2), read_message(&mut stream))
                .await
        else {
            break;
        };
        if header.kind != MessageType::Frame {
            continue;
        }
        highest = header.seq;
        sent += 1;
        let plaintext = session.open(&header, &payload).expect("the frame opens");
        let ack = FrameAck {
            acked_seq: highest,
            acked_bytes: plaintext.len() as u64,
        }
        .encode();
        let (ack_header, sealed) = session
            .seal(MessageType::FrameAck, 0, &ack)
            .expect("the ack is sealed");
        write_message(&mut stream, &ack_header, &sealed)
            .await
            .expect("the ack is written");
    }

    // With an acknowledgement per frame the server keeps sending.
    assert!(
        sent > MAX_WINDOW_FRAMES,
        "the server stopped at {sent} frames"
    );

    std::fs::remove_dir_all(&fixture.dir).ok();
}

#[tokio::test]
async fn a_view_only_device_cannot_send_input() {
    let fixture = start().await;
    let (mut stream, mut session) = connect(&fixture).await;

    let payload = kvm_protocol::messages::Mouse {
        x: 10,
        y: 10,
        buttons: 1,
        pressed: true,
        wheel: 0,
    }
    .encode();
    let (header, sealed) = session
        .seal(MessageType::InputMouse, 0, &payload)
        .expect("the input is sealed");
    write_message(&mut stream, &header, &sealed)
        .await
        .expect("the input is written");

    let mut buffer = Vec::new();
    let closed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_to_end(&mut buffer),
    )
    .await;
    assert!(closed.is_ok(), "the server must close the session");

    std::fs::remove_dir_all(&fixture.dir).ok();
}
