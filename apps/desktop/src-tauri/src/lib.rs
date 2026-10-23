//! The desktop server.
//!
//! The Tauri app is a thin shell around [`server::Server`]. The server is a
//! plain library, so it can also run headless on a machine without a
//! desktop session, and it can be tested without a webview.

pub mod config;
pub mod devices;
pub mod server;
pub mod source;

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kvm_protocol::pairing::{format_safety_code, PairingBlob};
use kvm_protocol::Psk;
use rand::RngCore;
use tauri::State;

use crate::config::ServerConfig;
use crate::devices::{DeviceEntry, DeviceStore, CAP_CONTROL, CAP_VIEW};
use crate::server::Server;
use crate::source::{FrameSource, SyntheticSource};

/// How long a pairing code stays valid.
const PAIRING_TTL_SECONDS: u64 = 300;

pub struct AppState {
    server: Arc<Server>,
    store: Arc<Mutex<DeviceStore>>,
    store_path: PathBuf,
    config: ServerConfig,
}

#[derive(serde::Serialize)]
pub struct ServerStatus {
    running: bool,
    port: u16,
    address: String,
    sessions: usize,
    paired_devices: usize,
}

#[derive(serde::Serialize)]
pub struct PairingInfo {
    blob: String,
    safety_code: String,
    expires_in_seconds: u64,
}

#[tauri::command]
fn server_status(state: State<'_, AppState>) -> ServerStatus {
    ServerStatus {
        running: true,
        port: state.config.port,
        address: local_address(),
        sessions: state.server.sessions(),
        paired_devices: state.store.lock().expect("store is not poisoned").paired_count(),
    }
}

#[tauri::command]
fn create_pairing(state: State<'_, AppState>) -> Result<PairingInfo, String> {
    let mut device_id = [0u8; 16];
    rand::rng().fill_bytes(&mut device_id);
    let psk = Psk::generate();

    let entry = DeviceEntry {
        id: hex(&device_id),
        name: "pending phone".to_string(),
        psk: hex(psk.as_bytes()),
        caps: CAP_VIEW | CAP_CONTROL,
        paired: false,
        created_at: now_iso8601(),
    };

    {
        let mut store = state.store.lock().expect("store is not poisoned");
        store.upsert(entry);
        store.save(&state.store_path).map_err(|error| error.to_string())?;
    }

    let blob = PairingBlob::new(
        device_id,
        psk,
        &state.config.server_name,
        &local_address(),
        state.config.port,
    );

    Ok(PairingInfo {
        blob: blob.encode(),
        safety_code: format_safety_code(blob.safety_code()),
        expires_in_seconds: PAIRING_TTL_SECONDS,
    })
}

#[tauri::command]
fn paired_devices(state: State<'_, AppState>) -> Vec<String> {
    let store = state.store.lock().expect("store is not poisoned");
    store
        .devices
        .iter()
        .filter(|entry| entry.paired)
        .map(|entry| entry.name.clone())
        .collect()
}

/// Builds the server and the state the commands need.
pub fn build_state(
    config: ServerConfig,
    store: DeviceStore,
    store_path: PathBuf,
    source: Arc<dyn FrameSource>,
) -> (AppState, Arc<Server>) {
    let server = Arc::new(Server::new(
        config.clone(),
        store.clone(),
        store_path.clone(),
        source,
    ));
    let state = AppState {
        server: server.clone(),
        store: Arc::new(Mutex::new(store)),
        store_path,
        config,
    };
    (state, server)
}

/// The default frame source, until the portal capture lands.
pub fn default_source(config: &ServerConfig) -> Arc<dyn FrameSource> {
    Arc::new(SyntheticSource::new(
        config.width,
        config.height,
        config.jpeg_quality,
    ))
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::load(&config::config_path()).unwrap_or_default();
    let store_path = config::device_store_path();
    let store = DeviceStore::load(&store_path).unwrap_or_default();
    let source = default_source(&config);
    let (state, server) = build_state(config, store, store_path, source);

    tauri::async_runtime::spawn(async move {
        if let Err(error) = server.run().await {
            tracing::error!(%error, "the server stopped");
        }
    });

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            server_status,
            create_pairing,
            paired_devices
        ])
        .run(tauri::generate_context!())
        .expect("cannot start the application");
}

/// Runs the server without the user interface.
pub fn run_headless() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::load(&config::config_path()).map_err(|error| error.to_string())?;
    let store_path = config::device_store_path();
    let store = DeviceStore::load(&store_path).unwrap_or_default();
    let source = default_source(&config);
    let server = Arc::new(Server::new(config, store, store_path, source));

    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        server.run().await.map_err(|error| error.to_string())
    })
}

/// The address other devices on the network can reach.
pub fn local_address() -> String {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok();
    let Some(socket) = socket else {
        return "127.0.0.1".to_string();
    };
    if socket.connect("192.0.2.1:9").is_err() {
        return "127.0.0.1".to_string();
    }
    match socket.local_addr() {
        Ok(SocketAddr::V4(addr)) => addr.ip().to_string(),
        _ => "127.0.0.1".to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}
