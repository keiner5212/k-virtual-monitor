//! The server.
//!
//! One TCP listener, one session per connection. A connection must complete
//! the handshake before it can receive a single frame, and the pairing secret
//! is resolved from the device store, never from the wire.
//!
//! The frame sender and the input reader share one task, so the sequence
//! numbers of a session are only ever touched by one place.

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
use crate::devices::DeviceStore;
use crate::source::{EncodedFrame, FrameSource};

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
}

impl Server {
    pub fn new(
        config: ServerConfig,
        store: DeviceStore,
        store_path: PathBuf,
        source: Arc<dyn FrameSource>,
    ) -> Self {
        Server {
            config,
            store: Arc::new(Mutex::new(store)),
            store_path,
            source,
            sessions: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(AtomicUsize::new(0)),
            session_info: Arc::new(Mutex::new(None)),
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
                self.psk_for(hello)
            }),
        )
        .await;
        self.pending.fetch_sub(1, Ordering::Relaxed);

        let (keys, hello) = match handshake {
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

        let entry = {
            let mut store = self.store.lock().expect("device store is not poisoned");
            let Some(entry) = store.find(&hex(&hello.device_id)).cloned() else {
                return;
            };
            let mut entry = entry;
            if entry.revoked {
                tracing::info!(%peer, device = %entry.name, "the device was revoked");
                return;
            }
            let renamed = entry.name != hello.device_name;
            entry.name = hello.device_name.clone();
            entry.paired = true;
            store.upsert(entry.clone());
            (entry, renamed)
        };
        let (entry, renamed) = entry;
        if renamed {
            if let Err(error) = self
                .store
                .lock()
                .expect("device store is not poisoned")
                .save(&self.store_path)
            {
                tracing::error!(%error, "cannot save the device store");
            }
        }

        let Some(codec) = codec::negotiate(hello.codec_mask, self.source.codecs()) else {
            tracing::info!(%peer, "the client cannot decode any codec we produce");
            return;
        };

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
        let result = self
            .run_session(stream, keys, entry.caps, codec, entry.name.clone())
            .await;
        self.sessions.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut info) = self.session_info.lock() {
            *info = None;
        }
        tracing::info!(%peer, ?result, "session ended");
    }

    fn psk_for(&self, hello: &Hello) -> Option<Psk> {
        let id = hex(&hello.device_id);
        let store = self.store.lock().expect("device store is not poisoned");
        let entry = store.find(&id)?;
        if entry.revoked {
            return None;
        }
        Psk::from_slice(&hex_bytes(&entry.psk)?).ok()
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

        let info = DisplayInfo {
            width: self.config.width,
            height: self.config.height,
            fps: self.config.fps,
            codec: codec::wire_value(codec),
            granted_caps,
            server_name: self.config.server_name.clone(),
        };
        let payload = info.encode();
        let (header, sealed) = session
            .seal(MessageType::DisplayInfo, 0, &payload)
            .map_err(|error| error.to_string())?;
        write_message(&mut stream, &header, &sealed)
            .await
            .map_err(|error| error.to_string())?;

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
            loop {
                enum Event {
                    Message(Option<(Header, Vec<u8>)>),
                    Frame(Option<EncodedFrame>),
                    Idle,
                }

                let event = if window.in_flight_frames() >= window.max_frames() {
                    // The window is full: wait for an acknowledgement, not for a
                    // frame to drop.
                    tokio::select! {
                        message = message_rx.recv() => Event::Message(message),
                        _ = tokio::time::sleep_until(idle_deadline) => Event::Idle,
                    }
                } else {
                    tokio::select! {
                        message = message_rx.recv() => Event::Message(message),
                        frame = frame_rx.recv() => Event::Frame(frame),
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes a hex string. A corrupt value is an error, not a zero byte: a
/// device with a broken secret must not be able to connect.
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}
