//! End to end test of a session: a paired client connects, completes the
//! handshake and receives frames over a real TCP socket.

use std::sync::Arc;

use kvm_desktop_lib::config::ServerConfig;
use kvm_desktop_lib::devices::{DeviceEntry, DeviceStore, CAP_VIEW};
use kvm_desktop_lib::server::Server;
use kvm_desktop_lib::source::SyntheticSource;
use kvm_protocol::crypto::Psk;
use kvm_protocol::frame::{read_message, MessageType};
use kvm_protocol::handshake::client_handshake;
use kvm_protocol::limits::MAX_PAYLOAD;
use kvm_protocol::messages::{codec, Hello};
use kvm_protocol::session::Session;
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

    let (header, payload) = read_message(&mut stream, MAX_PAYLOAD)
        .await
        .expect("the display info arrives");
    assert_eq!(header.kind, MessageType::DisplayInfo);
    let plaintext = session.open(&header, &payload).expect("the info opens");
    let info = kvm_protocol::messages::DisplayInfo::decode(&plaintext).expect("the info decodes");
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert_eq!(info.granted_caps, CAP_VIEW);

    let (header, payload) = read_message(&mut stream, MAX_PAYLOAD)
        .await
        .expect("a frame arrives");
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
