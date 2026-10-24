//! The server.
//!
//! One TCP listener, one session per connection. A connection must complete
//! the handshake before it can receive a single frame, and the pairing secret
//! is resolved from the device store, never from the wire.
//!
//! The frame sender and the input reader share one task, so the sequence
//! numbers of a session are only ever touched by one place.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kvm_protocol::crypto::Psk;
use kvm_protocol::crypto::TAG_LEN;
use kvm_protocol::frame::{read_message, write_message, Header, MessageType};
use kvm_protocol::handshake::server_handshake_with;
use kvm_protocol::messages::{caps, codec, DisplayInfo, FrameAck, Heartbeat, Hello};
use kvm_protocol::session::{SendWindow, Session};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::config::ServerConfig;
use crate::devices::{hex, unhex, unix_now, DeviceEntry, DeviceStore, CAP_CONTROL, CAP_VIEW};
use crate::now_stamp;
use crate::source::{EncodedFrame, FrameSource, StreamMode};
/// What the live session is doing, for the user interface.
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub device_name: String,
    pub codec: u16,
    pub fps: u16,
    pub frames_sent: u64,
}

pub struct Server {
    config: ServerConfig,
    store: Arc<Mutex<DeviceStore>>,
    store_path: PathBuf,
    source: Arc<dyn FrameSource>,
    sessions: Arc<AtomicUsize>,
    pending: Arc<AtomicUsize>,
    session_info: Arc<Mutex<Option<SessionInfo>>>,
    active_device: Arc<Mutex<Option<String>>>,
    connected: Arc<Mutex<HashSet<String>>>,
}

impl Server {
    pub fn new(
        config: ServerConfig,
        store: Arc<Mutex<DeviceStore>>,
        store_path: PathBuf,
        source: Arc<dyn FrameSource>,
    ) -> Self {
        Server {
            config,
            store,
            store_path,
            source,
            sessions: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(AtomicUsize::new(0)),
            session_info: Arc::new(Mutex::new(None)),
            active_device: Arc::new(Mutex::new(None)),
            connected: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn sessions(&self) -> usize {
        self.sessions.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn store(&self) -> &Arc<Mutex<DeviceStore>> {
        &self.store
    }

    /// What the live session is doing, if there is one.
    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session_info.lock().ok().and_then(|info| info.clone())
    }

    /// The paired device that owns the live session.
    pub fn active_device(&self) -> Option<String> {
        self.active_device.lock().ok().and_then(|id| id.clone())
    }

    /// Whether the device has a live session right now.
    pub fn is_connected(&self, id: &str) -> bool {
        self.connected
            .lock()
            .map(|devices| devices.contains(id))
            .unwrap_or(false)
    }

    /// What the capture wants the user to know.
    pub fn capture_status(&self) -> String {
        self.source.status()
    }

    /// Applies a size change when this device is the one on the virtual screen.
    pub fn apply_if_active(&self, id: &str, mode: StreamMode) {
        let active = self.active_device();
        if active.as_deref() == Some(id) {
            self.source.set_mode(mode);
        }
    }

    /// Opens the share dialog again after the user cancelled it.
    pub fn restart_capture(&self) {
        self.source.restart();
    }

    /// Stops the capture. The virtual screen and the encoder go away with it.
    pub fn stop_capture(&self) {
        self.source.stop();
    }

    /// Binds the listener. The port may be zero, in which case the system
    /// picks one and the caller can read it from the listener.
    pub async fn bind(&self) -> std::io::Result<TcpListener> {
        let listener = TcpListener::bind((self.config.bind.as_str(), self.config.port)).await?;
        tracing::info!(port = self.config.port, "server listening");
        Ok(listener)
    }

    /// Serves on an existing listener until the process ends.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, peer) = listener.accept().await?;
            // The slot is taken before the task is spawned, so a burst of
            // connections cannot spawn more tasks than the limit allows.
            if !self.reserve_pending() {
                tracing::warn!(%peer, "too many pending handshakes, dropping the connection");
                drop(stream);
                continue;
            }
            let server = self.clone();
            tokio::spawn(async move {
                server.handle(stream, peer).await;
            });
        }
    }

    /// Takes a pending handshake slot, or refuses when there are too many.
    fn reserve_pending(&self) -> bool {
        let limit = self.config.max_pending_handshakes.max(1);
        let mut current = self.pending.load(Ordering::Relaxed);
        loop {
            if current >= limit {
                return false;
            }
            match self.pending.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// Takes a session slot, or refuses when the server is full.
    fn reserve_session(&self) -> bool {
        let limit = self.config.max_sessions.max(1);
        let mut current = self.sessions.load(Ordering::Relaxed);
        loop {
            if current >= limit {
                return false;
            }
            match self.sessions.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// Binds the listener and serves until the process ends.
    pub async fn run(self: Arc<Self>) -> std::io::Result<()> {
        let listener = self.bind().await?;
        self.serve(listener).await
    }

    async fn handle(self: Arc<Self>, mut stream: TcpStream, peer: SocketAddr) {
        if let Err(error) = stream.set_nodelay(true) {
            tracing::warn!(%peer, %error, "cannot disable Nagle");
        }
        let handshake = tokio::time::timeout(
            Duration::from_secs(self.config.handshake_timeout_secs),
            server_handshake_with(&mut stream, &self.config.server_name, |hello| {
                self.psk_candidates(hello)
            }),
        )
        .await;
        self.pending.fetch_sub(1, Ordering::Relaxed);

        let (keys, hello, psk) = match handshake {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                tracing::info!(%peer, %error, "handshake refused");
                return;
            }
            Err(_) => {
                tracing::info!(%peer, "handshake timed out");
                return;
            }
        };

        let entry = self.adopt_device(&hello, &psk);
        let Some(entry) = entry else {
            tracing::info!(%peer, "the device is not allowed to connect");
            return;
        };
        let mut mode = mode_for(&hello, &self.config, &entry);
        let Some(codec) = codec::negotiate(hello.codec_mask, self.source.codecs()) else {
            tracing::info!(%peer, "the client cannot decode any codec we produce");
            return;
        };
        mode.codec = codec;
        self.source.set_mode(mode);

        if !self.reserve_session() {
            tracing::warn!(%peer, "the server is full, refusing the session");
            return;
        }

        tracing::info!(
            %peer,
            device = %entry.name,
            codec = codec::name(codec),
            "session started"
        );
        if let Ok(mut active) = self.active_device.lock() {
            *active = Some(entry.id.clone());
        }
        if let Ok(mut devices) = self.connected.lock() {
            devices.insert(entry.id.clone());
        }
        let result = self
            .run_session(stream, keys, entry.caps, codec, entry.name.clone())
            .await;
        if let Ok(mut active) = self.active_device.lock() {
            if active.as_deref() == Some(entry.id.as_str()) {
                *active = None;
            }
        }
        if let Ok(mut devices) = self.connected.lock() {
            devices.remove(&entry.id);
        }
        self.sessions.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut info) = self.session_info.lock() {
            *info = None;
        }
        tracing::info!(%peer, ?result, "session ended");
    }

    /// The pairing secrets that may match this greeting: the secret of the
    /// phone that is already known and the secret of the pairing code that is
    /// waiting for a phone.
    fn psk_candidates(&self, hello: &Hello) -> Vec<Psk> {
        let mut store = self.store.lock().expect("device store is not poisoned");
        // The store file is the source of truth: a code created by the
        // command line while the app was running has to be accepted.
        if let Err(error) = store.reload(&self.store_path) {
            tracing::warn!(%error, "cannot read the device store");
        }
        let mut candidates = Vec::new();
        if let Some(entry) = store.find(&hex(&hello.device_id)) {
            if let Some(psk) = psk_of(entry) {
                candidates.push(psk);
            }
        }
        if let Some(entry) = store.pending() {
            // A pairing code that ran out of time is not accepted any more.
            if !entry.is_expired(unix_now()) {
                if let Some(psk) = psk_of(entry) {
                    candidates.push(psk);
                }
            }
        }
        candidates
    }

    /// The entry of this phone, ready to be used.
    ///
    /// The phone owns its id and sends it in the greeting, so the entry takes
    /// that id. The secret it proved decides which entry it is, so a phone
    /// that pairs again lands in the same place instead of a new one.
    fn adopt_device(&self, hello: &Hello, psk: &Psk) -> Option<DeviceEntry> {
        let mut store = self.store.lock().expect("device store is not poisoned");
        let psk_text = hex(psk.as_bytes());
        let id = hex(&hello.device_id);
        let mut updated = match store.find_by_psk(&psk_text).cloned() {
            Some(entry) => {
                if entry.revoked {
                    tracing::info!(device = %entry.name, "the device was revoked");
                    return None;
                }
                entry
            }
            None => DeviceEntry {
                id: id.clone(),
                name: hello.device_name.clone(),
                psk: psk_text.clone(),
                caps: CAP_VIEW | CAP_CONTROL,
                paired: false,
                revoked: false,
                created_at: now_stamp(),
                expires_at: unix_now(),
                width: 0,
                height: 0,
                fps: 0,
                jpeg_quality: 0,
            },
        };
        let old_id = updated.id.clone();
        updated.id = id;
        updated.name = hello.device_name.clone();
        updated.paired = true;
        // The phone only gets what it asked for, never more.
        updated.caps &= hello.caps_requested;
        // The entry of a phone is the only one: the old id is dropped.
        store.remove(&old_id);
        store.upsert(updated.clone());
        if let Err(error) = store.save(&self.store_path) {
            tracing::error!(%error, "cannot save the device store");
        }
        Some(updated)
    }

    async fn run_session(
        &self,
        mut stream: TcpStream,
        keys: kvm_protocol::crypto::SessionKeys,
        granted_caps: u8,
        codec: u16,
        device_name: String,
    ) -> Result<(), String> {
        let mut session = Session::server(keys);
        let mut window = SendWindow::default();

        let mut sent_mode = self.source.mode();
        self.write_display_info(&mut stream, &mut session, sent_mode, granted_caps, codec)
            .await?;

        let mut frames_sent = 0u64;
        let mut frames_this_second = 0u64;
        let mut second_started = std::time::Instant::now();
        if let Ok(mut info) = self.session_info.lock() {
            *info = Some(SessionInfo {
                device_name,
                codec,
                fps: 0,
                frames_sent: 0,
            });
        }

        let (frame_tx, mut frame_rx) = mpsc::channel::<EncodedFrame>(4);
        let source = self.source.clone();
        tokio::task::spawn_blocking(move || {
            while let Some(frame) = source.next_frame() {
                if frame_tx.blocking_send(frame).is_err() {
                    break;
                }
            }
        });

        // The reader lives in its own task. A `select!` that reads from the
        // socket directly would cancel a half read message when the other
        // branch wins, and the bytes of that message would be lost: the
        // stream would desynchronise and every message after it would be
        // garbage.
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (message_tx, mut message_rx) = mpsc::channel::<(Header, Vec<u8>)>(8);
        let reader_task = tokio::spawn(async move {
            while let Ok(message) = read_message(&mut reader).await {
                if message_tx.send(message).await.is_err() {
                    break;
                }
            }
        });

        let result = async {
            let idle = Duration::from_secs(self.config.session_idle_timeout_secs);
            let mut idle_deadline = tokio::time::Instant::now() + idle;
            let mut mode_tick = tokio::time::interval(Duration::from_millis(400));
            mode_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            mode_tick.tick().await;
            loop {
                enum Event {
                    Message(Option<(Header, Vec<u8>)>),
                    Frame(Option<EncodedFrame>),
                    Idle,
                    Mode,
                }

                let event = if window.in_flight_frames() >= window.max_frames() {
                    // The window is full: wait for an acknowledgement, not for a
                    // frame to drop.
                    tokio::select! {
                        message = message_rx.recv() => Event::Message(message),
                        _ = mode_tick.tick() => Event::Mode,
                        _ = tokio::time::sleep_until(idle_deadline) => Event::Idle,
                    }
                } else {
                    tokio::select! {
                        message = message_rx.recv() => Event::Message(message),
                        frame = frame_rx.recv() => Event::Frame(frame),
                        _ = mode_tick.tick() => Event::Mode,
                        _ = tokio::time::sleep_until(idle_deadline) => Event::Idle,
                    }
                };

                match event {
                    Event::Message(Some((header, payload))) => {
                        idle_deadline = tokio::time::Instant::now() + idle;
                        if !self
                            .handle_client_message(
                                header,
                                &payload,
                                &mut window,
                                &mut session,
                                &mut writer,
                                granted_caps,
                            )
                            .await?
                        {
                            return Ok(());
                        }
                    }
                    Event::Message(None) => return Ok(()),
                    Event::Frame(Some(frame)) => {
                        if self
                            .send_frame(frame, &mut window, &mut session, &mut writer)
                            .await?
                        {
                            frames_sent += 1;
                            frames_this_second += 1;
                        }
                    }
                    Event::Frame(None) => return Ok(()),
                    Event::Mode => {
                        let mode = self.source.mode();
                        if mode != sent_mode {
                            self.write_display_info(
                                &mut writer,
                                &mut session,
                                mode,
                                granted_caps,
                                codec,
                            )
                            .await?;
                            sent_mode = mode;
                        }
                    }
                    Event::Idle => return Err("the session was idle".to_string()),
                }

                if second_started.elapsed() >= Duration::from_secs(1) {
                    let fps = frames_this_second as u16;
                    if let Ok(mut info) = self.session_info.lock() {
                        if let Some(info) = info.as_mut() {
                            info.fps = fps;
                            info.frames_sent = frames_sent;
                        }
                    }
                    frames_this_second = 0;
                    second_started = std::time::Instant::now();
                }
            }
        }
        .await;

        // The reader holds the other half of the socket. It has to go, or
        // the connection stays open after the session is over.
        reader_task.abort();
        let _ = writer.shutdown().await;
        result
    }

    async fn write_display_info<W: AsyncWrite + Unpin>(
        &self,
        stream: &mut W,
        session: &mut Session,
        mode: StreamMode,
        granted_caps: u8,
        codec_bit: u16,
    ) -> Result<(), String> {
        let info = DisplayInfo {
            width: mode.width,
            height: mode.height,
            fps: mode.fps,
            codec: codec::wire_value(codec_bit),
            granted_caps,
            server_name: self.config.server_name.clone(),
        };
        let payload = info.encode();
        let (header, sealed) = session
            .seal(MessageType::DisplayInfo, 0, &payload)
            .map_err(|error| error.to_string())?;
        write_message(stream, &header, &sealed)
            .await
            .map_err(|error| error.to_string())
    }

    /// Handles one message from the client.
    ///
    /// The payload is opened before anything is read out of it: the tag is
    /// verified first, so a tampered message is rejected before it can be
    /// acted on. The sequence number is checked by the same call.
    async fn handle_client_message<W: AsyncWrite + Unpin>(
        &self,
        header: Header,
        payload: &[u8],
        window: &mut SendWindow,
        session: &mut Session,
        stream: &mut W,
        granted_caps: u8,
    ) -> Result<bool, String> {
        let plaintext = session
            .open(&header, payload)
            .map_err(|error| format!("the message was rejected: {error}"))?;
        match header.kind {
            MessageType::FrameAck => {
                let ack = FrameAck::decode(&plaintext).map_err(|error| error.to_string())?;
                window.ack(ack.acked_seq);
            }
            MessageType::Heartbeat => {
                let payload = Heartbeat::decode(&plaintext)
                    .map_err(|error| error.to_string())?
                    .encode();
                let (header, sealed) = session
                    .seal(MessageType::HeartbeatAck, 0, &payload)
                    .map_err(|error| error.to_string())?;
                write_message(stream, &header, &sealed)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            MessageType::InputMouse | MessageType::InputKey | MessageType::InputTouch => {
                if granted_caps & caps::CONTROL == 0 {
                    tracing::warn!(
                        kind = header.kind.name(),
                        "input from a device without control rights, closing the session"
                    );
                    return Ok(false);
                }
                tracing::debug!(
                    kind = header.kind.name(),
                    bytes = plaintext.len(),
                    "input received"
                );
            }
            MessageType::Close => return Ok(false),
            other => {
                tracing::warn!(kind = other.name(), "unexpected message from the client");
            }
        }
        Ok(true)
    }

    /// Sends one frame. Returns false when the frame was dropped because the
    /// window is full: a late frame is worth less than a fresh one.
    async fn send_frame<W: AsyncWrite + Unpin>(
        &self,
        frame: EncodedFrame,
        window: &mut SendWindow,
        session: &mut Session,
        stream: &mut W,
    ) -> Result<bool, String> {
        let mut plaintext = frame.header.encode();
        plaintext.extend_from_slice(&frame.payload);
        let size = plaintext.len() as u64 + TAG_LEN as u64;
        if !window.can_send(size) {
            return Ok(false);
        }
        let (header, sealed) = session
            .seal(MessageType::Frame, 0, &plaintext)
            .map_err(|error| error.to_string())?;
        window.push(header.seq, sealed.len() as u64);
        write_message(stream, &header, &sealed)
            .await
            .map_err(|error| error.to_string())?;
        Ok(true)
    }
}

/// The size of the stream for this phone.
///
/// The desktop decides: the size the user picked for this phone, never bigger
/// than the phone can show. The phone only says what it can take.
fn mode_for(hello: &Hello, config: &ServerConfig, entry: &DeviceEntry) -> StreamMode {
    let width = if entry.width == 0 {
        config.width
    } else {
        entry.width
    };
    let height = if entry.height == 0 {
        config.height
    } else {
        entry.height
    };
    let (width, height) = fit(width, height, hello.max_width, hello.max_height);
    let fps = if entry.fps == 0 {
        config.fps
    } else {
        entry.fps
    };
    let fps = if hello.max_fps == 0 {
        fps
    } else {
        fps.min(hello.max_fps)
    };
    let quality = if entry.jpeg_quality == 0 {
        config.jpeg_quality
    } else {
        entry.jpeg_quality
    };
    StreamMode {
        width,
        height,
        fps,
        quality,
        codec: 0,
    }
    .normalize()
}

/// Keeps the size inside what the phone can show, without changing its
/// proportions.
fn fit(width: u16, height: u16, limit_width: u16, limit_height: u16) -> (u16, u16) {
    if limit_width == 0 || limit_height == 0 {
        return (width, height);
    }
    let scale = (f32::from(limit_width) / f32::from(width))
        .min(f32::from(limit_height) / f32::from(height));
    if scale >= 1.0 {
        return (width, height);
    }
    (
        (f32::from(width) * scale) as u16,
        (f32::from(height) * scale) as u16,
    )
}

/// The pairing secret of an entry.
fn psk_of(entry: &DeviceEntry) -> Option<Psk> {
    Psk::from_slice(&unhex(&entry.psk)?).ok()
}
