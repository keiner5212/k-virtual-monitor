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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kvm_protocol::crypto::Psk;
use kvm_protocol::pairing::{format_safety_code, PairingBlob};
use rand::RngCore;
use tauri::State;

use crate::config::ServerConfig;
use crate::devices::{hex, unhex, unix_now, DeviceEntry, DeviceStore, CAP_CONTROL, CAP_VIEW};
use crate::server::Server;
use crate::source::{FrameSource, SyntheticSource};

/// How long a pairing code stays valid.
const PAIRING_TTL_SECONDS: u64 = 300;

/// The name of a device that has not connected yet.
const PENDING_DEVICE_NAME: &str = "pending phone";

pub struct AppState {
    server: Arc<Server>,
    store: Arc<Mutex<DeviceStore>>,
    store_path: PathBuf,
    config: ServerConfig,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    running: bool,
    port: u16,
    address: String,
    sessions: usize,
    paired_devices: usize,
    codec: Option<String>,
    fps: Option<u16>,
    device_name: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    id: String,
    name: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    blob: String,
    safety_code: String,
    expires_in_seconds: u64,
    /// Unix seconds when the code stops being usable.
    expires_at: u64,
}

#[tauri::command]
fn server_status(state: State<'_, AppState>) -> ServerStatus {
    let session = state.server.session_info();
    ServerStatus {
        running: true,
        port: state.config.port,
        address: local_address(),
        sessions: state.server.sessions(),
        paired_devices: state
            .store
            .lock()
            .expect("store is not poisoned")
            .paired_count(),
        codec: session
            .as_ref()
            .map(|info| kvm_protocol::messages::codec::name(info.codec).to_uppercase()),
        fps: session.as_ref().map(|info| info.fps),
        device_name: session.map(|info| info.device_name),
    }
}

/// The configuration the server is running with.
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> ServerConfig {
    state.config.clone()
}

/// Writes the configuration. The listener is not restarted, so the port and
/// the bind address only change on the next start of the application.
#[tauri::command]
fn save_config(config: ServerConfig) -> Result<ServerConfig, String> {
    config
        .save(&config::config_path())
        .map_err(|error| error.to_string())?;
    Ok(config)
}

#[tauri::command]
fn create_pairing(state: State<'_, AppState>) -> Result<PairingInfo, String> {
    let mut store = state.store.lock().expect("store is not poisoned");
    create_pairing_entry(&state.config, &mut store, &state.store_path)
}

/// The pairing code that is alive right now, if there is one.
#[tauri::command]
fn current_pairing(state: State<'_, AppState>) -> Result<Option<PairingInfo>, String> {
    let mut store = state.store.lock().expect("store is not poisoned");
    refresh(&mut store, &state.store_path);
    let now = unix_now();
    prune(&mut store, &state.store_path, now);
    let Some(entry) = store.pending().cloned() else {
        return Ok(None);
    };
    Ok(Some(pairing_info(&state.config, &entry, now)?))
}

/// Creates a pairing code and stores the device as pending. The new code
/// replaces the previous one, so there is only ever one pending device.
fn create_pairing_entry(
    config: &ServerConfig,
    store: &mut DeviceStore,
    store_path: &Path,
) -> Result<PairingInfo, String> {
    let now = unix_now();
    refresh(store, store_path);
    prune(store, store_path, now);
    store.remove_pending();

    let mut device_id = [0u8; 16];
    rand::rng().fill_bytes(&mut device_id);
    let psk = Psk::generate();

    let entry = DeviceEntry {
        id: hex(&device_id),
        name: PENDING_DEVICE_NAME.to_string(),
        psk: hex(psk.as_bytes()),
        caps: CAP_VIEW | CAP_CONTROL,
        paired: false,
        revoked: false,
        created_at: now_stamp(),
        expires_at: now + PAIRING_TTL_SECONDS,
    };

    store.upsert(entry.clone());
    store.save(store_path).map_err(|error| error.to_string())?;

    pairing_info(config, &entry, now)
}

/// Prints a pairing code, for the headless mode.
pub fn print_pairing() -> Result<(), String> {
    let config = ServerConfig::load(&config::config_path()).map_err(|error| error.to_string())?;
    let store_path = config::device_store_path();
    let mut store = DeviceStore::load(&store_path).unwrap_or_default();
    let info = create_pairing_entry(&config, &mut store, &store_path)?;
    println!("pairing code: {}", info.blob);
    println!("safety code:  {}", info.safety_code);
    Ok(())
}

/// The phones that completed the handshake.
#[tauri::command]
fn paired_devices(state: State<'_, AppState>) -> Vec<DeviceInfo> {
    let mut store = state.store.lock().expect("store is not poisoned");
    refresh(&mut store, &state.store_path);
    prune(&mut store, &state.store_path, unix_now());
    store
        .devices
        .iter()
        .filter(|entry| entry.paired)
        .map(|entry| DeviceInfo {
            id: entry.id.clone(),
            name: entry.name.clone(),
        })
        .collect()
}

/// Removes a device. The pairing secret is gone, so the phone cannot connect
/// again, not even with the code it kept.
#[tauri::command]
fn revoke_device(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut store = state.store.lock().expect("store is not poisoned");
    refresh(&mut store, &state.store_path);
    if !store.remove(&id) {
        return Err("no such device".to_string());
    }
    store
        .save(&state.store_path)
        .map_err(|error| error.to_string())
}

/// Builds the server and the state the commands need. The server and the
/// commands share one store, so a device the interface pairs is a device the
/// server accepts.
pub fn build_state(
    config: ServerConfig,
    store: DeviceStore,
    store_path: PathBuf,
    source: Arc<dyn FrameSource>,
) -> (AppState, Arc<Server>) {
    let store = Arc::new(Mutex::new(store));
    let server = Arc::new(Server::new(
        config.clone(),
        store.clone(),
        store_path.clone(),
        source,
    ));
    let state = AppState {
        server: server.clone(),
        store,
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
            current_pairing,
            paired_devices,
            revoke_device,
            get_config,
            save_config
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

    let config = ServerConfig::load(&config::config_path()).unwrap_or_default();
    let store_path = config::device_store_path();
    let store = Arc::new(Mutex::new(
        DeviceStore::load(&store_path).unwrap_or_default(),
    ));
    let source = default_source(&config);
    let server = Arc::new(Server::new(config, store, store_path, source));

    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async move { server.run().await.map_err(|error| error.to_string()) })
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

/// Rebuilds the pairing code of a pending device.
fn pairing_info(
    config: &ServerConfig,
    entry: &DeviceEntry,
    now: u64,
) -> Result<PairingInfo, String> {
    let device_id: [u8; 16] = unhex(&entry.id)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("the device id is corrupt")?;
    let psk = Psk::from_slice(&unhex(&entry.psk).ok_or("the pairing secret is corrupt")?)
        .map_err(|error| error.to_string())?;
    let blob = PairingBlob::new(
        device_id,
        psk,
        &config.server_name,
        &local_address(),
        config.port,
    );
    Ok(PairingInfo {
        blob: blob.encode(),
        safety_code: format_safety_code(blob.safety_code()),
        expires_in_seconds: entry.expires_at.saturating_sub(now),
        expires_at: entry.expires_at,
    })
}

/// The store file is the source of truth. A code created by the command line
/// while the app was running has to be seen by the running server.
fn refresh(store: &mut DeviceStore, store_path: &Path) {
    if let Err(error) = store.reload(store_path) {
        tracing::warn!(%error, "cannot read the device store");
    }
}

/// Drops the entries that cannot be used any more, and writes the file when
/// something was dropped.
fn prune(store: &mut DeviceStore, store_path: &Path, now: u64) {
    if store.prune(now).is_empty() {
        return;
    }
    if let Err(error) = store.save(store_path) {
        tracing::warn!(%error, "cannot save the device store");
    }
}

fn now_stamp() -> String {
    format!("unix:{}", unix_now())
}
