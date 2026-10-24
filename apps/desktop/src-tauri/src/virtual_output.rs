//! The virtual screen.
//!
//! A RemoteDesktop session with a ScreenCast source of type virtual makes KWin
//! add a new monitor. It does not mirror an existing one. The first allow
//! prompt is the remote-control prompt. The stream comes back as a PipeWire
//! node. The development headers are not installed, so the library is loaded
//! by name and the structs below follow the PipeWire 1.0-1.6 ABI.

use rand::RngCore;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::fd::IntoRawFd;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use kvm_protocol::messages::codec;
use kvm_protocol::messages::FrameHeader;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{blocking::Connection, proxy};

use crate::h264::{self, H264Session};
use crate::source::{EncodedFrame, FrameSource, StreamMode};

const SPA_TYPE_ID: u32 = 3;
const SPA_TYPE_RECTANGLE: u32 = 10;
const SPA_TYPE_FRACTION: u32 = 11;
const SPA_TYPE_OBJECT: u32 = 15;
const SPA_TYPE_CHOICE: u32 = 19;
const SPA_TYPE_OBJECT_FORMAT: u32 = 0x40003;
const SPA_PARAM_FORMAT: u32 = 4;
const SPA_CHOICE_RANGE: u32 = 1;
const SPA_CHOICE_ENUM: u32 = 3;
const SPA_FORMAT_VIDEO_FORMAT: u32 = 0x20001;
const SPA_FORMAT_VIDEO_SIZE: u32 = 0x20003;
const SPA_VIDEO_RGBX: u32 = 7;
const SPA_VIDEO_BGRX: u32 = 8;
const SPA_VIDEO_RGBA: u32 = 11;
const SPA_VIDEO_BGRA: u32 = 12;
const SPA_VIDEO_RGB: u32 = 15;
const SPA_VIDEO_BGR: u32 = 16;

const RTLD_LAZY: i32 = 1;
const PROT_READ: i32 = 1;
const MAP_SHARED: i32 = 1;

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
    loop_ptr: AtomicPtr<std::ffi::c_void>,
    quit: Mutex<Option<QuitFn>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    started: Instant,
    seq: AtomicU64,
    delivered: AtomicU64,
    encoder: Mutex<Option<H264Session>>,
    h264_retry: Mutex<Option<Instant>>,
    h264_down: AtomicBool,
}

type QuitFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;

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
            loop_ptr: AtomicPtr::new(std::ptr::null_mut()),
            quit: Mutex::new(None),
            thread: Mutex::new(None),
            started: Instant::now(),
            seq: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            encoder: Mutex::new(None),
            h264_retry: Mutex::new(None),
            h264_down: AtomicBool::new(false),
        });
        inner.spawn();
        VirtualSource { inner }
    }
}

impl Drop for VirtualSource {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        let loop_ptr = self.inner.loop_ptr.load(Ordering::Acquire);
        if let Some(quit) = *self.inner.quit.lock().expect("quit fn is not poisoned") {
            if !loop_ptr.is_null() {
                unsafe { quit(loop_ptr) };
            }
        }
    }
}

impl FrameSource for VirtualSource {
    fn next_frame(&self) -> Option<EncodedFrame> {
        self.inner.next_frame()
    }

    fn codecs(&self) -> u16 {
        if h264::available() {
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
        if codec_bit == codec::H264 {
            self.inner.h264_down.store(false, Ordering::Relaxed);
            *self
                .inner
                .h264_retry
                .lock()
                .expect("h264 retry is not poisoned") = None;
        }
        if let Some(output) = self
            .inner
            .output
            .lock()
            .expect("output is not poisoned")
            .clone()
        {
            resize_output(&output, mode);
        }
    }

    fn status(&self) -> String {
        self.inner
            .status
            .lock()
            .expect("status is not poisoned")
            .clone()
    }

    fn restart(&self) {
        self.inner.stop.store(false, Ordering::Relaxed);
        self.inner.spawn();
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
                if *seq != delivered {
                    self.delivered.store(*seq, Ordering::Relaxed);
                    let frame = frame.clone();
                    return Some(frame);
                }
            }
            let (_guard, _) = self
                .cv
                .wait_timeout(guard, Duration::from_millis(250))
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
        } else {
            self.set_status("Virtual screen is streaming, but it is not in the display list yet");
        }
        // The connection has to stay alive: dropping it removes the monitor.
        let _connection = connection;
        stream_node(self, opened.fd, opened.node)
    }
}

struct OpenedStream {
    fd: std::os::fd::OwnedFd,
    node: u32,
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
    let fd = screencast
        .open_pipe_wire_remote(&session, HashMap::new())
        .map_err(|error| error.to_string())?;
    Ok(OpenedStream {
        fd: fd.into(),
        node,
        output_name,
        restore_token,
    })
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
    let _ = Command::new("kscreen-doctor")
        .arg(format!(
            "output.{id}.addCustomMode.{}.{}.{}.full",
            mode.width, mode.height, 60_000
        ))
        .status();
    let status = Command::new("kscreen-doctor")
        .arg(format!(
            "output.{id}.mode.{}x{}@60",
            mode.width, mode.height
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

    fn open_pipe_wire_remote(
        &self,
        session_handle: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
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

struct Pw {
    init: unsafe extern "C" fn(*mut i32, *mut *mut *mut i8),
    main_loop_new: unsafe extern "C" fn(*const std::ffi::c_void) -> *mut std::ffi::c_void,
    main_loop_get_loop: unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void,
    main_loop_run: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
    main_loop_quit: QuitFn,
    context_new: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        usize,
    ) -> *mut std::ffi::c_void,
    context_connect_fd: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        i32,
        *mut std::ffi::c_void,
        usize,
    ) -> *mut std::ffi::c_void,
    properties_new_string: unsafe extern "C" fn(*const i8) -> *mut std::ffi::c_void,
    stream_new_simple: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *const i8,
        *mut std::ffi::c_void,
        *const PwStreamEvents,
        *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void,
    stream_connect:
        unsafe extern "C" fn(*mut std::ffi::c_void, i32, u32, u32, *const *const u8, u32) -> i32,
    stream_dequeue_buffer: unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut PwBuffer,
    stream_queue_buffer: unsafe extern "C" fn(*mut std::ffi::c_void, *mut PwBuffer) -> i32,
}

#[repr(C)]
struct PwStreamEvents {
    version: u32,
    destroy: Option<extern "C" fn(*mut std::ffi::c_void)>,
    state_changed: Option<extern "C" fn(*mut std::ffi::c_void, i32, i32, *const i8)>,
    control_info: Option<extern "C" fn(*mut std::ffi::c_void, u32, *mut std::ffi::c_void)>,
    io_changed: Option<extern "C" fn(*mut std::ffi::c_void, u32, *mut std::ffi::c_void, u32)>,
    param_changed: Option<extern "C" fn(*mut std::ffi::c_void, u32, *const u8)>,
    add_buffer: Option<extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void)>,
    remove_buffer: Option<extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void)>,
    process: Option<extern "C" fn(*mut std::ffi::c_void)>,
    drained: Option<extern "C" fn(*mut std::ffi::c_void)>,
    command: Option<extern "C" fn(*mut std::ffi::c_void, *const u8)>,
    trigger_done: Option<extern "C" fn(*mut std::ffi::c_void)>,
}

#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut std::ffi::c_void,
    datas: *mut SpaData,
}

#[repr(C)]
struct SpaData {
    type_: u32,
    flags: u32,
    fd: i32,
    mapoffset: u32,
    maxsize: u32,
    data: *mut u8,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

struct CallbackState {
    inner: Arc<Inner>,
    pw: Arc<Pw>,
    stream: *mut std::ffi::c_void,
    format: u32,
    width: u32,
    height: u32,
    last_encode: Instant,
}

impl Pw {
    fn load() -> Result<Arc<Self>, String> {
        let handle = open_pipewire()?;
        unsafe {
            Ok(Arc::new(Pw {
                init: sym(handle, b"pw_init\0")?,
                main_loop_new: sym(handle, b"pw_main_loop_new\0")?,
                main_loop_get_loop: sym(handle, b"pw_main_loop_get_loop\0")?,
                main_loop_run: sym(handle, b"pw_main_loop_run\0")?,
                main_loop_quit: sym(handle, b"pw_main_loop_quit\0")?,
                context_new: sym(handle, b"pw_context_new\0")?,
                context_connect_fd: sym(handle, b"pw_context_connect_fd\0")?,
                properties_new_string: sym(handle, b"pw_properties_new_string\0")?,
                stream_new_simple: sym(handle, b"pw_stream_new_simple\0")?,
                stream_connect: sym(handle, b"pw_stream_connect\0")?,
                stream_dequeue_buffer: sym(handle, b"pw_stream_dequeue_buffer\0")?,
                stream_queue_buffer: sym(handle, b"pw_stream_queue_buffer\0")?,
            }))
        }
    }
}

fn open_pipewire() -> Result<*mut std::ffi::c_void, String> {
    const NAMES: [&CStr; 3] = [
        c"libpipewire-0.3.so.0",
        c"/usr/lib/x86_64-linux-gnu/libpipewire-0.3.so.0",
        c"/usr/lib/libpipewire-0.3.so.0",
    ];
    for name in NAMES {
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_LAZY) };
        if !handle.is_null() {
            return Ok(handle);
        }
    }
    Err("libpipewire-0.3.so.0 is not installed".to_string())
}

unsafe fn sym<T>(handle: *mut std::ffi::c_void, name: &[u8]) -> Result<T, String> {
    let pointer = dlsym(handle, name.as_ptr() as *const i8);
    if pointer.is_null() {
        let label = CStr::from_bytes_until_nul(name)
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "symbol".to_string());
        return Err(format!("pipewire is missing {label}"));
    }
    Ok(std::mem::transmute_copy(&pointer))
}

unsafe extern "C" {
    fn dlopen(filename: *const i8, flags: i32) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const i8) -> *mut std::ffi::c_void;
    fn mmap(
        addr: *mut std::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut std::ffi::c_void;
    fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
}

fn stream_node(inner: &Arc<Inner>, fd: std::os::fd::OwnedFd, node: u32) -> Result<(), String> {
    let pw = Pw::load()?;
    unsafe { (pw.init)(std::ptr::null_mut(), std::ptr::null_mut()) };
    let main_loop = unsafe { (pw.main_loop_new)(std::ptr::null()) };
    if main_loop.is_null() {
        return Err("pipewire did not create a loop".to_string());
    }
    inner.loop_ptr.store(main_loop, Ordering::Release);
    *inner.quit.lock().expect("quit fn is not poisoned") = Some(pw.main_loop_quit);
    let loop_ = unsafe { (pw.main_loop_get_loop)(main_loop) };
    let context = unsafe { (pw.context_new)(loop_, std::ptr::null_mut(), 0) };
    if context.is_null() {
        return Err("pipewire did not create a context".to_string());
    }
    let raw_fd = fd.into_raw_fd();
    let core = unsafe { (pw.context_connect_fd)(context, raw_fd, std::ptr::null_mut(), 0) };
    if core.is_null() {
        return Err("pipewire rejected the portal socket".to_string());
    }
    let props = CString::new("media.type=Video media.category=Capture media.role=Screen")
        .map_err(|error| error.to_string())?;
    let properties = unsafe { (pw.properties_new_string)(props.as_ptr()) };
    let name = CString::new("kvm-virtual").map_err(|error| error.to_string())?;
    let mut callback = CallbackState {
        inner: Arc::clone(inner),
        pw: Arc::clone(&pw),
        stream: std::ptr::null_mut(),
        format: SPA_VIDEO_BGRX,
        width: 0,
        height: 0,
        last_encode: Instant::now() - Duration::from_secs(1),
    };
    let events = PwStreamEvents {
        version: 2,
        destroy: None,
        state_changed: Some(on_state),
        control_info: None,
        io_changed: None,
        param_changed: Some(on_param),
        add_buffer: None,
        remove_buffer: None,
        process: Some(on_process),
        drained: None,
        command: None,
        trigger_done: None,
    };
    let stream = unsafe {
        (pw.stream_new_simple)(
            loop_,
            name.as_ptr(),
            properties,
            &events,
            &mut callback as *mut CallbackState as *mut std::ffi::c_void,
        )
    };
    if stream.is_null() {
        return Err("pipewire did not create a stream".to_string());
    }
    callback.stream = stream;
    let pod = format_pod();
    let ptrs = [pod.as_ptr()];
    let connected = unsafe { (pw.stream_connect)(stream, 0, node, 1 | 4, ptrs.as_ptr(), 1) };
    if connected < 0 {
        return Err(format!(
            "pipewire did not connect to the virtual screen ({connected})"
        ));
    }
    unsafe { (pw.main_loop_run)(main_loop) };
    Ok(())
}

extern "C" fn on_state(data: *mut std::ffi::c_void, _old: i32, state: i32, error: *const i8) {
    if state != -1 {
        return;
    }
    let callback = unsafe { &*(data as *const CallbackState) };
    let text = if error.is_null() {
        "pipewire stopped the stream".to_string()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    };
    callback
        .inner
        .set_status(format!("Virtual screen was not created: {text}"));
}

extern "C" fn on_param(data: *mut std::ffi::c_void, id: u32, param: *const u8) {
    if id != SPA_PARAM_FORMAT || param.is_null() {
        return;
    }
    let callback = unsafe { &mut *(data as *mut CallbackState) };
    let bytes = unsafe {
        let size = u32::from_ne_bytes(
            std::slice::from_raw_parts(param, 4)
                .try_into()
                .unwrap_or([0; 4]),
        ) as usize;
        if size > 1_000_000 {
            return;
        }
        std::slice::from_raw_parts(param, 8 + size)
    };
    if let Some((format, width, height)) = parse_video(bytes) {
        callback.format = format;
        callback.width = width;
        callback.height = height;
        tracing::info!(format, width, height, "virtual screen format");
    }
}

extern "C" fn on_process(data: *mut std::ffi::c_void) {
    let callback = unsafe { &mut *(data as *mut CallbackState) };
    let buffer = unsafe { (callback.pw.stream_dequeue_buffer)(callback.stream) };
    if buffer.is_null() {
        return;
    }
    let copied = copy_frame(callback, buffer);
    unsafe { (callback.pw.stream_queue_buffer)(callback.stream, buffer) };
    let Some((width, height, rgb)) = copied else {
        return;
    };
    let mode = *callback
        .inner
        .mode
        .lock()
        .expect("stream mode is not poisoned");
    let interval = Duration::from_millis(1000 / u64::from(mode.fps.max(1)));
    if callback.last_encode.elapsed() < interval {
        return;
    }
    callback.last_encode = Instant::now();
    if mode.codec == codec::H264 && !callback.inner.h264_down.load(Ordering::Relaxed) {
        match encode_h264(callback, &rgb, width, height, mode) {
            H264Outcome::Frame(unit) => {
                publish(
                    callback.inner.as_ref(),
                    width,
                    height,
                    codec::H264,
                    unit.keyframe,
                    unit.bytes,
                );
                return;
            }
            H264Outcome::Wait => return,
            H264Outcome::Failed => {
                callback.inner.h264_down.store(true, Ordering::Relaxed);
            }
        }
    }
    let Some(payload) = encode_jpeg(&rgb, width, height, mode.quality) else {
        return;
    };
    publish(
        callback.inner.as_ref(),
        width,
        height,
        codec::JPEG,
        true,
        payload,
    );
}

enum H264Outcome {
    Frame(h264::AccessUnit),
    Wait,
    Failed,
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
    let frame = EncodedFrame {
        header: FrameHeader {
            pts_ms: inner.started.elapsed().as_millis() as u64,
            keyframe,
            width: width as u16,
            height: height as u16,
            codec: codec::wire_value(codec_bit),
        },
        payload,
    };
    *inner.frame.lock().expect("frame slot is not poisoned") = Some((seq, frame));
    inner.cv.notify_one();
}

fn encode_h264(
    callback: &mut CallbackState,
    rgb: &[u8],
    width: u32,
    height: u32,
    mode: StreamMode,
) -> H264Outcome {
    let retry_at = *callback
        .inner
        .h264_retry
        .lock()
        .expect("h264 retry is not poisoned");
    if let Some(failed_at) = retry_at {
        if failed_at.elapsed() < Duration::from_secs(1) {
            return H264Outcome::Failed;
        }
    }
    let mut slot = callback
        .inner
        .encoder
        .lock()
        .expect("h264 encoder is not poisoned");
    let reopen = slot
        .as_ref()
        .map(|session| !session.matches(width, height, mode.fps, mode.quality))
        .unwrap_or(true);
    if reopen {
        match H264Session::open(width, height, mode.fps, mode.quality) {
            Ok(session) => *slot = Some(session),
            Err(error) => {
                *slot = None;
                drop(slot);
                *callback
                    .inner
                    .h264_retry
                    .lock()
                    .expect("h264 retry is not poisoned") = Some(Instant::now());
                callback.inner.set_status(format!(
                    "H.264 encoder failed, sending JPEG instead: {error}"
                ));
                return H264Outcome::Failed;
            }
        }
    }
    let Some(session) = slot.as_mut() else {
        return H264Outcome::Failed;
    };
    match session.push(rgb) {
        Ok(Some(unit)) => H264Outcome::Frame(unit),
        Ok(None) => H264Outcome::Wait,
        Err(error) => {
            *slot = None;
            drop(slot);
            *callback
                .inner
                .h264_retry
                .lock()
                .expect("h264 retry is not poisoned") = Some(Instant::now());
            callback.inner.set_status(format!(
                "H.264 encoder failed, sending JPEG instead: {error}"
            ));
            H264Outcome::Failed
        }
    }
}

fn copy_frame(callback: &CallbackState, buffer: *mut PwBuffer) -> Option<(u32, u32, Vec<u8>)> {
    let width = callback.width;
    let height = callback.height;
    if width == 0 || height == 0 {
        return None;
    }
    let spa = unsafe { &*(*buffer).buffer };
    if spa.n_datas == 0 || spa.datas.is_null() {
        return None;
    }
    let data = unsafe { &*spa.datas };
    if data.chunk.is_null() {
        return None;
    }
    let chunk = unsafe { &*data.chunk };
    let stride = if chunk.stride == 0 {
        (width * bytes_per_pixel(callback.format)) as i32
    } else {
        chunk.stride
    };
    let bpp = bytes_per_pixel(callback.format) as usize;
    let row = stride.unsigned_abs() as usize;
    if row < width as usize * bpp {
        return None;
    }
    let need = row.saturating_mul(height as usize);
    if need == 0 || need > 64 * 1024 * 1024 {
        return None;
    }
    let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
    let base = if !data.data.is_null() {
        data.data
    } else if data.fd >= 0 {
        mapped = unsafe {
            mmap(
                std::ptr::null_mut(),
                data.maxsize as usize,
                PROT_READ,
                MAP_SHARED,
                data.fd,
                data.mapoffset as i64,
            )
        };
        if mapped.is_null() || mapped == usize::MAX as *mut std::ffi::c_void {
            return None;
        }
        mapped as *mut u8
    } else {
        return None;
    };
    let offset = chunk.offset as usize;
    let available = if chunk.size == 0 {
        need
    } else {
        chunk.size as usize
    };
    if offset.saturating_add(available) < need && chunk.size != 0 {
        if !mapped.is_null() {
            unsafe { munmap(mapped, data.maxsize as usize) };
        }
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(base.add(offset), need.max(available)) };
    let rgb = pixels_to_rgb(callback.format, bytes, width, height, stride);
    if !mapped.is_null() {
        unsafe { munmap(mapped, data.maxsize as usize) };
    }
    rgb.map(|rgb| (width, height, rgb))
}

fn bytes_per_pixel(format: u32) -> u32 {
    match format {
        SPA_VIDEO_RGB | SPA_VIDEO_BGR => 3,
        _ => 4,
    }
}

fn pixels_to_rgb(format: u32, src: &[u8], width: u32, height: u32, stride: i32) -> Option<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let row = stride.unsigned_abs() as usize;
    let bpp = bytes_per_pixel(format) as usize;
    if row < width * bpp || src.len() < row * height {
        return None;
    }
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let line = &src[y * row..];
        for x in 0..width {
            let (r, g, b) = match format {
                SPA_VIDEO_RGBX | SPA_VIDEO_RGBA => (line[x * 4], line[x * 4 + 1], line[x * 4 + 2]),
                SPA_VIDEO_RGB => (line[x * 3], line[x * 3 + 1], line[x * 3 + 2]),
                SPA_VIDEO_BGR => (line[x * 3 + 2], line[x * 3 + 1], line[x * 3]),
                _ => (line[x * 4 + 2], line[x * 4 + 1], line[x * 4]),
            };
            let offset = (y * width + x) * 3;
            rgb[offset] = r;
            rgb[offset + 1] = g;
            rgb[offset + 2] = b;
        }
    }
    Some(rgb)
}

fn encode_jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    let mut payload = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut payload, quality);
    encoder
        .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(payload)
}

fn format_pod() -> Vec<u8> {
    let mut props = Vec::new();
    push_prop(&mut props, 1, &pod_id(2));
    push_prop(&mut props, 2, &pod_id(1));
    push_prop(
        &mut props,
        SPA_FORMAT_VIDEO_FORMAT,
        &choice_enum_id(&[
            SPA_VIDEO_BGRX,
            SPA_VIDEO_BGRX,
            SPA_VIDEO_BGRA,
            SPA_VIDEO_RGBX,
            SPA_VIDEO_RGBA,
        ]),
    );
    push_prop(
        &mut props,
        SPA_FORMAT_VIDEO_SIZE,
        &choice_range_rectangle(1920, 1080, 1, 1, 8192, 8192),
    );
    push_prop(
        &mut props,
        0x20004,
        &choice_range_fraction(30, 1, 0, 1, 240, 1),
    );
    let mut pod = Vec::new();
    let body = 8 + props.len();
    pod.extend_from_slice(&(body as u32).to_ne_bytes());
    pod.extend_from_slice(&SPA_TYPE_OBJECT.to_ne_bytes());
    pod.extend_from_slice(&SPA_TYPE_OBJECT_FORMAT.to_ne_bytes());
    pod.extend_from_slice(&SPA_PARAM_FORMAT.to_ne_bytes());
    pod.extend_from_slice(&props);
    pad(&mut pod);
    pod
}

fn push_prop(buf: &mut Vec<u8>, key: u32, value: &[u8]) {
    buf.extend_from_slice(&key.to_ne_bytes());
    buf.extend_from_slice(&0u32.to_ne_bytes());
    buf.extend_from_slice(value);
}

fn pad(buf: &mut Vec<u8>) {
    while buf.len() % 8 != 0 {
        buf.push(0);
    }
}

fn pod_id(id: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&4u32.to_ne_bytes());
    buf.extend_from_slice(&SPA_TYPE_ID.to_ne_bytes());
    buf.extend_from_slice(&id.to_ne_bytes());
    pad(&mut buf);
    buf
}

fn pod_rectangle(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&8u32.to_ne_bytes());
    buf.extend_from_slice(&SPA_TYPE_RECTANGLE.to_ne_bytes());
    buf.extend_from_slice(&width.to_ne_bytes());
    buf.extend_from_slice(&height.to_ne_bytes());
    buf
}

fn pod_fraction(num: u32, denom: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&8u32.to_ne_bytes());
    buf.extend_from_slice(&SPA_TYPE_FRACTION.to_ne_bytes());
    buf.extend_from_slice(&num.to_ne_bytes());
    buf.extend_from_slice(&denom.to_ne_bytes());
    buf
}

fn choice(kind: u32, children: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    let body = 8 + children.len();
    buf.extend_from_slice(&(body as u32).to_ne_bytes());
    buf.extend_from_slice(&SPA_TYPE_CHOICE.to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(&0u32.to_ne_bytes());
    buf.extend_from_slice(children);
    pad(&mut buf);
    buf
}

fn choice_enum_id(ids: &[u32]) -> Vec<u8> {
    let mut children = Vec::new();
    for id in ids {
        children.extend(pod_id(*id));
    }
    choice(SPA_CHOICE_ENUM, &children)
}

fn choice_range_rectangle(
    dw: u32,
    dh: u32,
    min_w: u32,
    min_h: u32,
    max_w: u32,
    max_h: u32,
) -> Vec<u8> {
    let mut children = Vec::new();
    children.extend(pod_rectangle(dw, dh));
    children.extend(pod_rectangle(min_w, min_h));
    children.extend(pod_rectangle(max_w, max_h));
    choice(SPA_CHOICE_RANGE, &children)
}

fn choice_range_fraction(
    dn: u32,
    dd: u32,
    min_n: u32,
    min_d: u32,
    max_n: u32,
    max_d: u32,
) -> Vec<u8> {
    let mut children = Vec::new();
    children.extend(pod_fraction(dn, dd));
    children.extend(pod_fraction(min_n, min_d));
    children.extend(pod_fraction(max_n, max_d));
    choice(SPA_CHOICE_RANGE, &children)
}

struct Pod<'a> {
    kind: u32,
    body: &'a [u8],
}

fn read_pod(bytes: &[u8]) -> Option<Pod<'_>> {
    if bytes.len() < 8 {
        return None;
    }
    let size = u32::from_ne_bytes(bytes[0..4].try_into().ok()?) as usize;
    let kind = u32::from_ne_bytes(bytes[4..8].try_into().ok()?);
    if bytes.len() < 8 + size {
        return None;
    }
    Some(Pod {
        kind,
        body: &bytes[8..8 + size],
    })
}

fn object_props(body: &[u8]) -> Vec<(u32, Pod<'_>)> {
    if body.len() < 8 {
        return Vec::new();
    }
    let mut offset = 8;
    let mut props = Vec::new();
    while offset + 8 <= body.len() {
        let key = u32::from_ne_bytes(body[offset..offset + 4].try_into().unwrap_or([0; 4]));
        offset += 8;
        let Some(rest) = body.get(offset..) else {
            break;
        };
        let Some(pod) = read_pod(rest) else {
            break;
        };
        let size = u32::from_ne_bytes(rest[0..4].try_into().unwrap_or([0; 4])) as usize;
        props.push((key, pod));
        offset += (8 + size + 7) & !7;
    }
    props
}

fn choice_default(body: &[u8]) -> Option<Pod<'_>> {
    let rest = body.get(8..)?;
    read_pod(rest)
}

fn parse_video(bytes: &[u8]) -> Option<(u32, u32, u32)> {
    let pod = read_pod(bytes)?;
    if pod.kind != SPA_TYPE_OBJECT {
        return None;
    }
    let mut format = None;
    let mut size = None;
    for (key, value) in object_props(pod.body) {
        let value = if value.kind == SPA_TYPE_CHOICE {
            choice_default(value.body)?
        } else {
            value
        };
        if key == SPA_FORMAT_VIDEO_FORMAT && value.body.len() >= 4 {
            format = Some(u32::from_ne_bytes(value.body[0..4].try_into().ok()?));
        }
        if key == SPA_FORMAT_VIDEO_SIZE && value.body.len() >= 8 {
            let width = u32::from_ne_bytes(value.body[0..4].try_into().ok()?);
            let height = u32::from_ne_bytes(value.body[4..8].try_into().ok()?);
            size = Some((width, height));
        }
    }
    Some((format?, size?.0, size?.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_pod_offers_a_raw_video_size() {
        let (format, width, height) = parse_video(&format_pod()).expect("a video format");
        assert_eq!(format, SPA_VIDEO_BGRX);
        assert_eq!((width, height), (1920, 1080));
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

    #[test]
    fn pipewire_loads_when_the_library_is_installed() {
        let present = [
            "/usr/lib/x86_64-linux-gnu/libpipewire-0.3.so.0",
            "/usr/lib/libpipewire-0.3.so.0",
        ]
        .into_iter()
        .any(|path| std::path::Path::new(path).exists());
        if !present {
            return;
        }
        Pw::load().expect("pipewire symbols");
    }
}
