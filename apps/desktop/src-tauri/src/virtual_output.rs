//! The virtual screen.
//!
//! A RemoteDesktop session with a ScreenCast source of type virtual makes KWin
//! add a new monitor. It does not mirror an existing one. The first allow
//! prompt is the remote-control prompt. The stream comes back as a PipeWire
//! node id. GStreamer (`pipewiresrc`) reads that node. The portal socket is
//! not used: handing that fd to a second PipeWire client aborts the stream.

use rand::RngCore;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use kvm_protocol::messages::codec;
use kvm_protocol::messages::FrameHeader;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{blocking::Connection, proxy};

use crate::source::{EncodedFrame, FrameSource, StreamMode};

/// How long the last picture is repeated when the screen does not change.
///
/// The compositor only sends a picture when something on the screen
/// changes. Without this the phone would wait for a frame that never comes.
const KEEPALIVE: Duration = Duration::from_millis(500);

struct ScreenOutput {
    id: i64,
    name: String,
}

/// The permission the portal gave us. It is the way back in without a
/// prompt, so it is kept with owner only permissions.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct PortalPermission {
    restore_token: Option<String>,
}

impl PortalPermission {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let _ = crate::devices::write_private(path, text.as_bytes());
    }
}

struct Inner {
    mode: Mutex<StreamMode>,
    output: Mutex<Option<ScreenOutput>>,
    frame: Mutex<Option<(u64, EncodedFrame)>>,
    cv: Condvar,
    stop: AtomicBool,
    status: Mutex<String>,
    thread: Mutex<Option<JoinHandle<()>>>,
    started: Instant,
    seq: AtomicU64,
    delivered: AtomicU64,
    /// When the last picture was handed out.
    last_sent: Mutex<Instant>,
    /// Size and rate last applied to the virtual screen. The portal picks
    /// its own, so the first apply happens even when the mode matches.
    resized: Mutex<Option<(u16, u16, u16)>>,
    /// When the last frame was produced. A screen with nothing on it produces
    /// no frames at all.
    last_frame: Mutex<Option<Instant>>,
    /// The GStreamer process. Stop kills it so the read unblocks.
    gst_pid: AtomicU32,
}

pub struct VirtualSource {
    inner: Arc<Inner>,
}

impl VirtualSource {
    pub fn start(mode: StreamMode) -> Self {
        let inner = Arc::new(Inner {
            mode: Mutex::new(mode.normalize()),
            output: Mutex::new(None),
            frame: Mutex::new(None),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            status: Mutex::new("Accept the prompt. It adds a new screen.".to_string()),
            thread: Mutex::new(None),
            started: Instant::now(),
            seq: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            last_sent: Mutex::new(Instant::now() - Duration::from_secs(1)),
            resized: Mutex::new(None),
            last_frame: Mutex::new(None),
            gst_pid: AtomicU32::new(0),
        });
        inner.spawn();
        VirtualSource { inner }
    }
}

impl Drop for VirtualSource {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FrameSource for VirtualSource {
    fn next_frame(&self) -> Option<EncodedFrame> {
        self.inner.next_frame()
    }

    fn codecs(&self) -> u16 {
        if gstreamer_h264() {
            codec::H264 | codec::JPEG
        } else {
            codec::JPEG
        }
    }

    fn mode(&self) -> StreamMode {
        *self.inner.mode.lock().expect("stream mode is not poisoned")
    }

    fn set_mode(&self, mode: StreamMode) {
        let incoming = mode.normalize();
        let mut slot = self.inner.mode.lock().expect("stream mode is not poisoned");
        let codec_bit = if incoming.codec == 0 {
            slot.codec
        } else {
            incoming.codec
        };
        let mode = StreamMode {
            codec: codec_bit,
            ..incoming
        };
        *slot = mode;
        drop(slot);
        if let Some(output) = self
            .inner
            .output
            .lock()
            .expect("output is not poisoned")
            .clone()
        {
            // The display is only touched when the size or the rate changed.
            let mut resized = self.inner.resized.lock().expect("resized is not poisoned");
            let applied = (mode.width, mode.height, mode.fps);
            if *resized != Some(applied) {
                resize_output(&output, mode);
                *resized = Some(applied);
            }
        }
    }

    fn status(&self) -> String {
        let stored = self
            .inner
            .status
            .lock()
            .expect("status is not poisoned")
            .clone();
        if !stored.starts_with("Virtual screen") {
            return stored;
        }
        // The compositor only sends a picture when something on the screen
        // changes. The last one is repeated, so the phone always has a
        // picture, but the first one has to arrive before that.
        let ready = self
            .inner
            .last_frame
            .lock()
            .expect("last frame is not poisoned")
            .is_some();
        if ready {
            "The virtual screen is streaming.".to_string()
        } else {
            "Waiting for the first picture of the virtual screen.".to_string()
        }
    }

    fn restart(&self) {
        self.inner.stop.store(false, Ordering::Relaxed);
        self.inner.spawn();
    }

    fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        let pid = self.inner.gst_pid.load(Ordering::Acquire);
        if pid != 0 {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
    }
}

impl Clone for ScreenOutput {
    fn clone(&self) -> Self {
        ScreenOutput {
            id: self.id,
            name: self.name.clone(),
        }
    }
}

impl Inner {
    fn spawn(self: &Arc<Self>) {
        let mut slot = self.thread.lock().expect("capture thread is not poisoned");
        if let Some(handle) = slot.as_ref() {
            if !handle.is_finished() {
                return;
            }
        }
        if let Some(handle) = slot.take() {
            let _ = handle.join();
        }
        let this = Arc::clone(self);
        *slot = thread::Builder::new()
            .name("kvm-virtual".into())
            .spawn(move || this.run())
            .ok();
    }

    fn run(self: &Arc<Self>) {
        if let Err(error) = self.capture() {
            self.set_status(format!("Virtual screen was not created: {error}"));
            tracing::error!(%error, "virtual screen failed");
        }
    }

    fn set_status(&self, text: impl Into<String>) {
        *self.status.lock().expect("status is not poisoned") = text.into();
    }

    fn next_frame(&self) -> Option<EncodedFrame> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
            let delivered = self.delivered.load(Ordering::Relaxed);
            let guard = self.frame.lock().expect("frame slot is not poisoned");
            if let Some((seq, frame)) = guard.as_ref() {
                let mut last_sent = self.last_sent.lock().expect("last frame is not poisoned");
                if *seq != delivered || last_sent.elapsed() >= KEEPALIVE {
                    self.delivered.store(*seq, Ordering::Relaxed);
                    *last_sent = Instant::now();
                    *self.last_frame.lock().expect("last frame is not poisoned") =
                        Some(Instant::now());
                    let frame = frame.clone();
                    return Some(frame);
                }
            }
            let (_guard, _) = self
                .cv
                .wait_timeout(guard, Duration::from_millis(100))
                .expect("frame slot is not poisoned");
        }
    }

    fn capture(self: &Arc<Self>) -> Result<(), String> {
        self.set_status("Accept the prompt. It adds a new screen.");
        let before = list_outputs();
        let connection = Connection::session().map_err(|error| error.to_string())?;
        let permission_path = crate::config::portal_permission_path();
        let permission = PortalPermission::load(&permission_path);
        let opened = open_virtual_stream(&connection, permission.restore_token.as_deref())?;
        if let Some(token) = &opened.restore_token {
            PortalPermission {
                restore_token: Some(token.clone()),
            }
            .save(&permission_path);
        }
        let output = find_output(opened.output_name.as_deref(), &before);
        if let Some(output) = &output {
            let mode = *self.mode.lock().expect("stream mode is not poisoned");
            resize_output(output, mode);
            place_output(output, &list_outputs());
            self.set_status(format!("Virtual screen {}", output.name));
            *self.output.lock().expect("output is not poisoned") = Some(output.clone());
            // The portal gives the new screen a size of its own, so it has to
            // be set to the size of the stream.
            *self.resized.lock().expect("resized is not poisoned") = None;
        } else {
            self.set_status("Virtual screen is streaming, but it is not in the display list yet");
        }
        let result = stream_node(self, opened.node);
        // The session ends here, so the virtual screen goes away with it.
        close_session(&connection, &opened.session);
        result
    }
}

struct OpenedStream {
    node: u32,
    /// The portal session, so it can be closed when the capture ends.
    session: OwnedObjectPath,
    /// The name of the output the portal created, when the portal tells it.
    output_name: Option<String>,
    /// The permission the portal gave us, so the next run does not ask again.
    restore_token: Option<String>,
}

fn open_virtual_stream(
    connection: &Connection,
    restore_token: Option<&str>,
) -> Result<OpenedStream, String> {
    let remote = RemoteDesktopProxyBlocking::new(connection).map_err(|error| error.to_string())?;
    let screencast = ScreenCastProxyBlocking::new(connection).map_err(|error| error.to_string())?;
    let handle = token();
    let session_token = token();
    let mut create = HashMap::new();
    create.insert("handle_token", Value::from(handle.as_str()));
    create.insert("session_handle_token", Value::from(session_token.as_str()));
    let created = portal_request(connection, &handle, || remote.create_session(create))?;
    let session = object_path(created.get("session_handle"))?;

    let devices_token = token();
    let mut devices = HashMap::new();
    devices.insert("handle_token", Value::from(devices_token.as_str()));
    // 1 keyboard, 2 pointer, 4 touch. A session with none is rejected.
    devices.insert("types", Value::from(7u32));
    // 2 keeps the permission until the user revokes it, so the next run starts
    // without a prompt. The token is the way back in.
    devices.insert("persist_mode", Value::from(2u32));
    if let Some(token) = restore_token {
        devices.insert("restore_token", Value::from(token));
    }
    portal_request(connection, &devices_token, || {
        remote.select_devices(&session, devices)
    })?;

    let select_token = token();
    let mut select = HashMap::new();
    select.insert("handle_token", Value::from(select_token.as_str()));
    // 4 is a new virtual monitor, not the existing desktop. 2 embeds the pointer.
    select.insert("types", Value::from(4u32));
    select.insert("cursor_mode", Value::from(2u32));
    select.insert("multiple", Value::from(false));
    portal_request(connection, &select_token, || {
        screencast.select_sources(&session, select)
    })?;

    let start_token = token();
    let mut start = HashMap::new();
    start.insert("handle_token", Value::from(start_token.as_str()));
    // RemoteDesktop.Start with type 4 adds a monitor. ScreenCast.Start opens the share picker.
    let started = portal_request(connection, &start_token, || {
        remote.start(&session, "", start)
    })?;
    let (node, output_name) = virtual_node(&started)?;
    let restore_token = started.get("restore_token").and_then(as_string);
    Ok(OpenedStream {
        node,
        session,
        output_name,
        restore_token,
    })
}

/// Ends the portal session, so the virtual screen goes away with it.
fn close_session(connection: &Connection, session: &OwnedObjectPath) {
    let Ok(proxy) = SessionProxyBlocking::builder(connection)
        .path(session.as_ref())
        .and_then(|builder| builder.build())
    else {
        return;
    };
    if let Err(error) = proxy.close() {
        tracing::warn!(%error, "the portal session did not close");
    }
}

fn portal_request(
    connection: &Connection,
    handle: &str,
    call: impl FnOnce() -> zbus::Result<OwnedObjectPath>,
) -> Result<HashMap<String, OwnedValue>, String> {
    let sender = connection
        .unique_name()
        .ok_or("the session bus has no unique name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{handle}");
    let request = RequestProxyBlocking::builder(connection)
        .path(path)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    let mut incoming = request
        .receive_response()
        .map_err(|error| error.to_string())?;
    let _returned = call().map_err(|error| error.to_string())?;
    let message = incoming.next().ok_or("the portal closed the request")?;
    let args = message.args().map_err(|error| error.to_string())?;
    match args.response {
        0 => Ok(args.results),
        1 => Err("the share dialog was cancelled".to_string()),
        other => Err(format!("the portal rejected the request ({other})")),
    }
}

fn object_path(value: Option<&OwnedValue>) -> Result<OwnedObjectPath, String> {
    let value = value.ok_or("the portal did not return a session")?;
    if let Ok(path) = OwnedObjectPath::try_from(value.clone()) {
        return Ok(path);
    }
    // KDE sends the session path as a string variant, not an object path.
    let text = String::try_from(value.clone()).map_err(|error| error.to_string())?;
    OwnedObjectPath::try_from(text).map_err(|error| error.to_string())
}

/// The stream the portal opened, and the output it comes from.
fn virtual_node(results: &HashMap<String, OwnedValue>) -> Result<(u32, Option<String>), String> {
    let value = results
        .get("streams")
        .ok_or_else(|| format!("the portal did not return a stream: {results:?}"))?;
    let streams = Vec::<(u32, HashMap<String, OwnedValue>)>::try_from(value.clone())
        .map_err(|error| format!("cannot read the portal stream: {error}"))?;
    let (node, meta) = streams
        .first()
        .ok_or_else(|| "the portal returned no stream".to_string())?;
    // 1 is a real monitor, 2 a window, 4 the new virtual screen.
    match meta.get("source_type").and_then(as_u32) {
        Some(4) | None => {}
        Some(kind) => {
            return Err(format!(
                "the portal shared an existing screen ({kind}). Accept the dialog that creates a new virtual screen."
            ))
        }
    }
    // The id of the stream is the name of the output. Some portals leave it
    // empty, so it is only a hint.
    let output_name = meta.get("id").and_then(as_string);
    tracing::info!(
        node,
        output = output_name.as_deref().unwrap_or(""),
        "the portal opened the virtual screen"
    );
    Ok((*node, output_name))
}

fn as_string(value: &OwnedValue) -> Option<String> {
    String::try_from(value.clone()).ok()
}

fn as_u32(value: &OwnedValue) -> Option<u32> {
    u32::try_from(value.clone()).ok().or_else(|| {
        i32::try_from(value.clone())
            .ok()
            .and_then(|number| u32::try_from(number).ok())
    })
}

fn token() -> String {
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn list_outputs() -> Vec<ScreenOutput> {
    let output = Command::new("kscreen-doctor").arg("-j").output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    doc.get("outputs")
        .and_then(|value| value.as_array())
        .map(|outputs| {
            outputs
                .iter()
                .filter_map(|output| {
                    let name = output.get("name")?.as_str()?.to_string();
                    let id = output.get("id")?.as_i64()?;
                    Some(ScreenOutput { id, name })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn output_edges() -> Vec<(i64, i32, i32)> {
    let output = Command::new("kscreen-doctor").arg("-j").output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let Some(outputs) = doc.get("outputs").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    outputs
        .iter()
        .filter_map(|output| {
            let id = output.get("id")?.as_i64()?;
            let x = output.get("pos")?.get("x")?.as_i64()? as i32;
            let width = output.get("size")?.get("width")?.as_i64()? as i32;
            Some((id, x, width))
        })
        .collect()
}

/// The output the portal created.
///
/// The output ids change when the layout changes, so the name is what tells
/// the new screen apart from the ones that were already there. The screen of
/// the user is never returned, so it is never resized.
fn find_output(wanted: Option<&str>, before: &[ScreenOutput]) -> Option<ScreenOutput> {
    let known: std::collections::HashSet<&str> =
        before.iter().map(|output| output.name.as_str()).collect();
    for _ in 0..40 {
        let outputs = list_outputs();
        if let Some(name) = wanted {
            if let Some(output) = outputs.iter().find(|output| output.name == name) {
                return Some(output.clone());
            }
        }
        if let Some(output) = outputs
            .into_iter()
            .find(|output| !known.contains(output.name.as_str()))
        {
            return Some(output);
        }
        thread::sleep(Duration::from_millis(200));
    }
    None
}

fn resize_output(output: &ScreenOutput, mode: StreamMode) {
    let id = output.id;
    let refresh = u32::from(mode.fps) * 1000;
    let _ = Command::new("kscreen-doctor")
        .arg(format!(
            "output.{id}.addCustomMode.{}.{}.{refresh}.full",
            mode.width, mode.height
        ))
        .status();
    let status = Command::new("kscreen-doctor")
        .arg(format!(
            "output.{id}.mode.{}x{}@{}",
            mode.width, mode.height, mode.fps
        ))
        .arg(format!("output.{id}.scale.1"))
        .arg(format!("output.{id}.enable"))
        .status();
    if status.as_ref().map(|code| !code.success()).unwrap_or(true) {
        tracing::warn!(
            id,
            width = mode.width,
            height = mode.height,
            "could not resize the virtual screen"
        );
    }
}

fn place_output(output: &ScreenOutput, _current: &[ScreenOutput]) {
    let right = output_edges()
        .into_iter()
        .filter(|(id, _, _)| *id != output.id)
        .map(|(_, x, width)| x.saturating_add(width))
        .max()
        .unwrap_or(0);
    let _ = Command::new("kscreen-doctor")
        .arg(format!("output.{}.position.{right},0", output.id))
        .status();
}

#[proxy(
    interface = "org.freedesktop.portal.ScreenCast",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait ScreenCast {
    fn create_session(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<OwnedObjectPath>;

    fn select_sources(
        &self,
        session_handle: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn start(
        &self,
        session_handle: &ObjectPath<'_>,
        parent_window: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.portal.RemoteDesktop",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait RemoteDesktop {
    fn create_session(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<OwnedObjectPath>;

    fn select_devices(
        &self,
        session_handle: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn start(
        &self,
        session_handle: &ObjectPath<'_>,
        parent_window: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
trait Request {
    #[zbus(signal)]
    fn response(&self, response: u32, results: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.portal.Session",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Session {
    fn close(&self) -> zbus::Result<()>;
}

const GST_CAPTURE: &str = r#####"
import struct, sys, threading
import gi
gi.require_version("Gst", "1.0")
gi.require_version("GLib", "2.0")
from gi.repository import GLib, Gst

Gst.init(None)
node = int(sys.argv[1])
width = int(sys.argv[2])
height = int(sys.argv[3])
fps = max(1, int(sys.argv[4]))
quality = max(1, min(100, int(sys.argv[5])))
codec = sys.argv[6]
if codec == "h264":
    qp = max(18, min(40, 51 - quality * 33 // 100))
    enc = (
        f"x264enc tune=zerolatency speed-preset=ultrafast "
        f"key-int-max={fps} bframes=0 byte-stream=true quantizer={qp} "
        f"rc-lookahead=0 sync-lookahead=0 sliced-threads=true ! "
        f"h264parse config-interval=-1 ! "
        f"video/x-h264,stream-format=byte-stream,alignment=au ! "
    )
else:
    enc = f"jpegenc quality={quality} idct-method=ifast ! "
desc = (
    f"pipewiresrc path={node} ! "
    f"videoconvert ! videoscale method=0 ! "
    f"videorate drop-only=true skip-to-first=true ! "
    f"video/x-raw,width={width},height={height},framerate={fps}/1,format=I420 ! "
    f"{enc}"
    f"appsink name=sink emit-signals=true max-buffers=1 drop=true sync=false"
)
pipeline = Gst.parse_launch(desc)
sink = pipeline.get_by_name("sink")
out = sys.stdout.buffer
lock = threading.Lock()

def on_sample(element):
    sample = element.emit("pull-sample")
    if sample is None:
        return Gst.FlowReturn.OK
    buf = sample.get_buffer()
    ok, info = buf.map(Gst.MapFlags.READ)
    if not ok:
        return Gst.FlowReturn.OK
    try:
        data = bytes(info.data)
    finally:
        buf.unmap(info)
    key = 0 if buf.has_flags(Gst.BufferFlags.DELTA_UNIT) else 1
    header = struct.pack("<4sIBHH", b"KVMF", len(data), key, width, height)
    with lock:
        out.write(header + data)
        out.flush()
    return Gst.FlowReturn.OK

sink.connect("new-sample", on_sample)
loop = GLib.MainLoop()
bus = pipeline.get_bus()
bus.add_signal_watch()

def on_bus(_bus, message):
    if message.type == Gst.MessageType.ERROR:
        err, debug = message.parse_error()
        print(f"gstreamer: {err} {debug}", file=sys.stderr, flush=True)
        loop.quit()
    elif message.type == Gst.MessageType.EOS:
        loop.quit()

bus.connect("message", on_bus)
if pipeline.set_state(Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE:
    print("gstreamer: pipeline did not start", file=sys.stderr, flush=True)
    sys.exit(1)
try:
    loop.run()
finally:
    pipeline.set_state(Gst.State.NULL)
"#####;

fn gstreamer_h264() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        Command::new("python3")
            .args([
                "-c",
                "import gi\n\
                 gi.require_version('Gst','1.0')\n\
                 from gi.repository import Gst\n\
                 Gst.init(None)\n\
                 raise SystemExit(0 if Gst.ElementFactory.find('x264enc') else 1)",
            ])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

fn stream_node(inner: &Arc<Inner>, node: u32) -> Result<(), String> {
    while !inner.stop.load(Ordering::Relaxed) {
        let mode = *inner.mode.lock().expect("stream mode is not poisoned");
        let mut child = spawn_capture(node, mode)?;
        inner.gst_pid.store(child.id(), Ordering::Release);
        let stdout = child.stdout.take().ok_or("capture has no picture output")?;
        let stderr = child.stderr.take();
        let logged = thread::spawn(move || stderr.map(drain_stderr).unwrap_or_default());
        let reading = Arc::clone(inner);
        let reader = thread::spawn(move || read_frames(stdout, &reading));
        let mut exited = false;
        loop {
            if inner.stop.load(Ordering::Relaxed) {
                break;
            }
            let now = *inner.mode.lock().expect("stream mode is not poisoned");
            if capture_changed(mode, now) {
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                Ok(None) => {}
                Err(_) => break,
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        inner.gst_pid.store(0, Ordering::Release);
        let _ = reader.join();
        let error = logged.join().unwrap_or_default();
        if inner.stop.load(Ordering::Relaxed) || capture_changed(mode, inner.current_mode()) {
            continue;
        }
        if exited {
            if error.is_empty() {
                return Err("gstreamer stopped".to_string());
            }
            return Err(error);
        }
    }
    Ok(())
}

fn capture_changed(started: StreamMode, now: StreamMode) -> bool {
    started.width != now.width
        || started.height != now.height
        || started.fps != now.fps
        || started.quality != now.quality
        || started.codec != now.codec
}

impl Inner {
    fn current_mode(&self) -> StreamMode {
        *self.mode.lock().expect("stream mode is not poisoned")
    }
}

fn spawn_capture(node: u32, mode: StreamMode) -> Result<std::process::Child, String> {
    let codec = if mode.codec == codec::H264 {
        "h264"
    } else {
        "jpeg"
    };
    let node = node.to_string();
    let width = mode.width.to_string();
    let height = mode.height.to_string();
    let fps = mode.fps.to_string();
    let quality = mode.quality.to_string();
    let mut child = Command::new("python3")
        .args(["-", &node, &width, &height, &fps, &quality, codec])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("python3 is not available: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("capture has no input")?;
    stdin
        .write_all(GST_CAPTURE.as_bytes())
        .map_err(|error| error.to_string())?;
    drop(stdin);
    Ok(child)
}

fn drain_stderr(mut stderr: impl Read) -> String {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                kept.extend_from_slice(&chunk[..n]);
                if kept.len() > 4000 {
                    let extra = kept.len() - 4000;
                    kept.drain(..extra);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept).trim().to_string()
}

fn read_frames(mut stdout: impl Read, inner: &Inner) {
    loop {
        let mut header = [0u8; 13];
        if stdout.read_exact(&mut header).is_err() {
            return;
        }
        if &header[0..4] != b"KVMF" {
            return;
        }
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap_or([0; 4])) as usize;
        if len == 0 || len > 8 * 1024 * 1024 {
            return;
        }
        let keyframe = header[8] != 0;
        let width = u32::from(u16::from_le_bytes(
            header[9..11].try_into().unwrap_or([0; 2]),
        ));
        let height = u32::from(u16::from_le_bytes(
            header[11..13].try_into().unwrap_or([0; 2]),
        ));
        let mut payload = vec![0u8; len];
        if stdout.read_exact(&mut payload).is_err() {
            return;
        }
        let codec_bit = if payload_is_jpeg(&payload) {
            codec::JPEG
        } else {
            codec::H264
        };
        publish(inner, width, height, codec_bit, keyframe, payload);
    }
}

fn payload_is_jpeg(payload: &[u8]) -> bool {
    payload.starts_with(&[0xff, 0xd8])
}

fn publish(
    inner: &Inner,
    width: u32,
    height: u32,
    codec_bit: u16,
    keyframe: bool,
    payload: Vec<u8>,
) {
    let seq = inner.seq.fetch_add(1, Ordering::Relaxed) + 1;
    *inner.last_frame.lock().expect("last frame is not poisoned") = Some(Instant::now());
    let frame = EncodedFrame {
        header: FrameHeader {
            pts_ms: inner.started.elapsed().as_millis() as u64,
            keyframe,
            width: width as u16,
            height: height as u16,
            codec: codec::wire_value(codec_bit),
        },
        payload: std::sync::Arc::new(payload),
    };
    *inner.frame.lock().expect("frame slot is not poisoned") = Some((seq, frame));
    inner.cv.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> std::path::PathBuf {
        if let Ok(path) = std::env::var("KVM_TMP") {
            let path = std::path::PathBuf::from(path);
            let _ = std::fs::create_dir_all(&path);
            return path;
        }
        let compiled = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.tmp");
        if std::fs::create_dir_all(&compiled).is_ok() {
            return compiled;
        }
        let local = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(".tmp");
        let _ = std::fs::create_dir_all(&local);
        local
    }

    #[test]
    fn the_portal_asks_for_a_virtual_screen() {
        use zbus::zvariant::serialized::Context;
        use zbus::zvariant::{to_bytes, Endian};

        let mut options = HashMap::new();
        options.insert("types", Value::from(4u32));
        let encoded = to_bytes(Context::new_dbus(Endian::Little, 0), &options).expect("encode");
        let (decoded, _): (HashMap<String, Value>, _) = encoded.deserialize().expect("decode");
        let types = decoded.get("types").expect("types");
        // KDE does options.value("types").toUInt() and treats 0 as "share the real monitor".
        // A nested variant would decode as signature "v" and that call returns 0.
        assert_eq!(types.value_signature().to_string(), "u");
        assert_eq!(u32::try_from(types.clone()).unwrap(), 4);

        let session =
            ObjectPath::try_from("/org/freedesktop/portal/desktop/session/probe").unwrap();
        let message =
            zbus::message::Message::method_call("/org/freedesktop/portal/desktop", "SelectSources")
                .unwrap()
                .destination("org.freedesktop.portal.Desktop")
                .unwrap()
                .interface("org.freedesktop.portal.ScreenCast")
                .unwrap()
                .build(&(session, &options))
                .unwrap();
        let (_path, sent): (OwnedObjectPath, HashMap<String, OwnedValue>) =
            message.body().deserialize().unwrap();
        let sent_types = sent.get("types").expect("types");
        assert_eq!(sent_types.value_signature().to_string(), "u");
        assert_eq!(u32::try_from(sent_types.clone()).unwrap(), 4);
    }

    /// Grabs the virtual screen the portal creates and checks it against a
    /// second screenshot of that same output. Skipped in CI.
    #[test]
    #[ignore = "needs the live desktop and the portal prompt"]
    fn the_live_virtual_screen_is_a_picture() {
        let mode = StreamMode {
            width: 1280,
            height: 720,
            fps: 5,
            quality: 90,
            codec: codec::JPEG,
        };
        let before = list_outputs();
        let laptop = output_layouts();
        let source = std::sync::Arc::new(VirtualSource::start(mode));
        let reading = std::sync::Arc::clone(&source);
        let reader = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            let mut frames = Vec::new();
            while frames.len() < 8 && std::time::Instant::now() < deadline {
                match reading.next_frame() {
                    Some(frame) => frames.push(frame),
                    None => break,
                }
            }
            frames
        });
        let started = std::time::Instant::now();
        let mut ready = false;
        while started.elapsed() < std::time::Duration::from_secs(90) {
            let status = source.status();
            eprintln!("capture: {status}");
            if status.contains("Waiting for the first picture") || status.contains("streaming") {
                ready = true;
                break;
            }
            if status.starts_with("Virtual screen was not created") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        if ready {
            paint_marker_on_new_screen();
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        let status = source.status();
        let dir = scratch_dir().join("frames");
        std::fs::create_dir_all(&dir).expect("frames");
        let geometry = new_output_geometry(&before);
        if let Some(geometry) = geometry {
            let reference = dir.join("live-reference.png");
            screenshot_output(&reference, geometry);
        }
        source.stop();
        let frames = reader.join().expect("capture thread");
        let frame = frames.last().unwrap_or_else(|| {
            panic!("no frame from the virtual screen ({status})");
        });
        assert_eq!(frame.header.codec, codec::wire_value(codec::JPEG));
        assert!(frame.payload.starts_with(&[0xff, 0xd8]));
        let live = dir.join("live-virtual.jpg");
        std::fs::write(&live, frame.payload.as_slice()).expect("live jpeg");
        let decoded = image::load_from_memory(&frame.payload)
            .expect("the live frame is not a jpeg")
            .to_rgb8();
        assert_eq!(
            decoded.dimensions(),
            (
                u32::from(frame.header.width),
                u32::from(frame.header.height)
            )
        );
        geometry.expect("the new output has no geometry");
        let reference = dir.join("live-reference.png");
        let diff = mean_abs_diff(&live, &reference, &dir);
        eprintln!("live frame differs from the desktop screenshot by {diff:.1}");
        assert!(
            diff < 30.0,
            "the captured frame does not match the screen (mean abs {diff:.1})"
        );
        let after = output_layouts();
        for (name, x, y, width, height) in &laptop {
            let same = after
                .iter()
                .find(|(after_name, _, _, _, _)| after_name == name);
            assert_eq!(
                same.map(|(_, ax, ay, aw, ah)| (*ax, *ay, *aw, *ah)),
                Some((*x, *y, *width, *height)),
                "{name} was resized"
            );
        }
    }

    fn output_layouts() -> Vec<(String, i64, i64, u64, u64)> {
        let output = std::process::Command::new("kscreen-doctor")
            .arg("-j")
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        let Ok(doc) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
            return Vec::new();
        };
        doc.get("outputs")
            .and_then(|value| value.as_array())
            .map(|outputs| {
                outputs
                    .iter()
                    .filter_map(|output| {
                        let name = output.get("name")?.as_str()?.to_string();
                        let x = output.get("pos")?.get("x")?.as_i64()?;
                        let y = output.get("pos")?.get("y")?.as_i64()?;
                        let width = output.get("size")?.get("width")?.as_u64()?;
                        let height = output.get("size")?.get("height")?.as_u64()?;
                        Some((name, x, y, width, height))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn paint_marker_on_new_screen() {
        let dir = scratch_dir().join("frames");
        let _ = std::fs::create_dir_all(&dir);
        let marker = marker_jpeg(1280, 720);
        let path = dir.join("marker.jpg");
        if std::fs::write(&path, &marker).is_err() {
            return;
        }
        let file = path
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('\'', "");
        let script = format!(
            "var desks = desktops();\n\
             for (var i = 0; i < desks.length; i++) {{\n\
             if (desks[i].screen === 0) continue;\n\
             desks[i].wallpaperPlugin = 'org.kde.image';\n\
             desks[i].currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General'];\n\
             desks[i].writeConfig('Image', 'file://{file}');\n\
             }}\n\
             'painted ' + desks.length;"
        );
        let output = std::process::Command::new("qdbus6")
            .args([
                "org.kde.plasmashell",
                "/PlasmaShell",
                "org.kde.PlasmaShell.evaluateScript",
                &script,
            ])
            .output();
        eprintln!(
            "wallpaper: {}",
            output
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_else(|error| error.to_string())
        );
    }

    fn marker_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut rgb = vec![20u8; (width * height * 3) as usize];
        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 3) as usize;
                if x < 280 && y < 180 {
                    rgb[index] = 230;
                    rgb[index + 1] = 40;
                    rgb[index + 2] = 40;
                } else if x > width - 300 && y > height - 200 {
                    rgb[index] = 30;
                    rgb[index + 1] = 90;
                    rgb[index + 2] = 220;
                } else if (x as i32 - y as i32).unsigned_abs() < 8 {
                    rgb[index] = 240;
                    rgb[index + 1] = 220;
                    rgb[index + 2] = 40;
                }
            }
        }
        let mut payload = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut payload, 95);
        encoder
            .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .expect("marker");
        payload
    }

    fn new_output_geometry(before: &[ScreenOutput]) -> Option<(i32, i32, u32, u32)> {
        let known: std::collections::HashSet<&str> =
            before.iter().map(|output| output.name.as_str()).collect();
        let output = std::process::Command::new("kscreen-doctor")
            .arg("-j")
            .output()
            .ok()?;
        let doc: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        for output in doc.get("outputs")?.as_array()? {
            let name = output.get("name")?.as_str()?;
            if known.contains(name) {
                continue;
            }
            let x = output.get("pos")?.get("x")?.as_i64()? as i32;
            let y = output.get("pos")?.get("y")?.as_i64()? as i32;
            let width = output.get("size")?.get("width")?.as_u64()? as u32;
            let height = output.get("size")?.get("height")?.as_u64()? as u32;
            return Some((x, y, width, height));
        }
        None
    }

    fn screenshot_output(path: &std::path::Path, geometry: (i32, i32, u32, u32)) {
        let full = path.with_file_name("live-desktop.png");
        let status = std::process::Command::new("spectacle")
            .args(["-b", "-n", "-f", "-o"])
            .arg(&full)
            .status()
            .expect("spectacle");
        assert!(status.success(), "spectacle did not grab the desktop");
        let (x, y, width, height) = geometry;
        let crop = format!("crop={width}:{height}:{x}:{y}");
        let status = std::process::Command::new("ffmpeg")
            .env("TMPDIR", scratch_dir())
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&full)
            .args(["-vf", &crop])
            .arg(path)
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "could not crop the virtual screen");
    }

    fn mean_abs_diff(
        left: &std::path::Path,
        right: &std::path::Path,
        dir: &std::path::Path,
    ) -> f32 {
        let left_raw = dir.join("live-left.rgb");
        let right_raw = dir.join("live-right.rgb");
        decode_rgb(left, &left_raw);
        decode_rgb(right, &right_raw);
        let left = std::fs::read(&left_raw).expect("left rgb");
        let right = std::fs::read(&right_raw).expect("right rgb");
        assert_eq!(left.len(), right.len(), "the two pictures differ in size");
        let mut sum = 0u64;
        for (a, b) in left.iter().zip(right.iter()) {
            sum += u64::from(a.abs_diff(*b));
        }
        sum as f32 / left.len() as f32
    }

    fn decode_rgb(path: &std::path::Path, raw: &std::path::Path) {
        let status = std::process::Command::new("ffmpeg")
            .env("TMPDIR", scratch_dir())
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .arg(raw)
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "could not decode {path:?}");
    }
}
