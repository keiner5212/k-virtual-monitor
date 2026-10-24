//! The GStreamer child that encodes the picture.
//!
//! The compositor only paints the screen when something on it changes, so the
//! picture arrives late and sometimes not at all. The child repeats the last
//! picture to the encoder, so the stream keeps the rate it was asked for and
//! the phone always has something to show.
//!
//! The repetition happens before the encoder and never after it: a picture
//! that is already encoded cannot be sent twice, because the decoder of the
//! phone would apply the second one twice.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use kvm_protocol::messages::codec;

use crate::source::{EncodedFrame, FrameSlot, StreamMode};

/// The program that runs inside the child.
const SCRIPT: &str = r#####"
import struct, sys, threading, time
import gi
gi.require_version("Gst", "1.0")
gi.require_version("GLib", "2.0")
from gi.repository import GLib, Gst

Gst.init(None)

source = sys.argv[1]
width = int(sys.argv[2])
height = int(sys.argv[3])
fps = max(1, int(sys.argv[4]))
quality = max(1, min(100, int(sys.argv[5])))
CODEC_H264 = 2

quantizer = max(18, min(40, 51 - quality * 33 // 100))
capture_description = (
    f"{source} ! videoconvert ! videoscale method=0 ! "
    f"videorate drop-only=true ! "
    f"video/x-raw,width={width},height={height},format=I420,framerate={fps}/1 ! "
    f"appsink name=raw emit-signals=true max-buffers=1 drop=true sync=false"
)
encoder_description = (
    f"appsrc name=src is-live=true format=time ! "
    f"video/x-raw,width={width},height={height},format=I420,framerate={fps}/1 ! "
    f"x264enc tune=zerolatency speed-preset=ultrafast key-int-max={fps} bframes=0 "
    f"byte-stream=true quantizer={quantizer} rc-lookahead=0 sync-lookahead=0 "
    f"sliced-threads=true ! "
    f"h264parse config-interval=-1 ! "
    f"video/x-h264,stream-format=byte-stream,alignment=au ! "
    f"appsink name=out emit-signals=true max-buffers=1 drop=true sync=false"
)

capture = Gst.parse_launch(capture_description)
encoder = Gst.parse_launch(encoder_description)
raw_sink = capture.get_by_name("raw")
source_element = encoder.get_by_name("src")
out_sink = encoder.get_by_name("out")
source_element.set_property(
    "caps",
    Gst.Caps.from_string(
        f"video/x-raw,width={width},height={height},format=I420,framerate={fps}/1"
    ),
)

lock = threading.Lock()
state = {"picture": None, "pending": None, "pushed": 0.0}
out = sys.stdout.buffer
interval = 1.0 / fps


def push(picture):
    frame = picture.copy()
    frame.pts = int(Gst.util_get_timestamp())
    frame.duration = int(Gst.SECOND / fps)
    with lock:
        state["pushed"] = time.monotonic()
    source_element.emit("push-buffer", frame)


def tick():
    # The picture goes to the encoder when it is due. A picture that arrives
    # between two ticks waits for the next one, so the encoder keeps the rate
    # of the stream.
    now = time.monotonic()
    with lock:
        picture = state["pending"] or state["picture"]
        state["pending"] = None
        due = now - state["pushed"] >= interval
    if picture is not None and due:
        push(picture)


def on_picture(element):
    sample = element.emit("pull-sample")
    if sample is None:
        return Gst.FlowReturn.OK
    picture = sample.get_buffer()
    with lock:
        state["picture"] = picture
        state["pending"] = picture
    # A new picture goes to the encoder at once: the phone has to see it now.
    tick()
    return Gst.FlowReturn.OK


def on_frame(element):
    sample = element.emit("pull-sample")
    if sample is None:
        return Gst.FlowReturn.OK
    buffer = sample.get_buffer()
    ok, info = buffer.map(Gst.MapFlags.READ)
    if not ok:
        return Gst.FlowReturn.OK
    try:
        data = bytes(info.data)
    finally:
        buffer.unmap(info)
    key = 0 if buffer.has_flags(Gst.BufferFlags.DELTA_UNIT) else 1
    header = struct.pack(
        "<4sIBHHB", b"KVMF", len(data), key, width, height, CODEC_H264
    )
    with lock:
        out.write(header + data)
        out.flush()
    return Gst.FlowReturn.OK


raw_sink.connect("new-sample", on_picture)
out_sink.connect("new-sample", on_frame)
GLib.timeout_add(max(1, int(1000 / fps)), tick)

loop = GLib.MainLoop()
bus = capture.get_bus()
bus.add_signal_watch()


def on_bus(_bus, message):
    if message.type == Gst.MessageType.ERROR:
        error, debug = message.parse_error()
        print(f"gstreamer: {error} {debug}", file=sys.stderr, flush=True)
        loop.quit()
    elif message.type == Gst.MessageType.EOS:
        loop.quit()


bus.connect("message", on_bus)
for pipeline in (capture, encoder):
    if pipeline.set_state(Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE:
        print("gstreamer: the pipeline did not start", file=sys.stderr, flush=True)
        sys.exit(1)
try:
    loop.run()
finally:
    for pipeline in (capture, encoder):
        pipeline.set_state(Gst.State.NULL)
"#####;

/// The value the child writes in the header of every picture.
const CODEC_H264: u8 = 2;

/// The size of the header of a picture: magic, length, key frame, size, codec.
const HEADER_LEN: usize = 14;

/// The encoder of a source.
///
/// It runs a child process and starts it again when the size, the rate or the
/// quality of the source change, because the encoder is built with those
/// values.
pub struct Encoder {
    mode: Arc<Mutex<StreamMode>>,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    errors: Arc<Mutex<String>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Encoder {
    /// Starts the encoder of a source, for example `pipewiresrc path=42`.
    pub fn start(source: String, mode: Arc<Mutex<StreamMode>>, slot: Arc<FrameSlot>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(Mutex::new(String::new()));
        let encoder = Encoder {
            mode,
            slot,
            stop: Arc::clone(&stop),
            failed: Arc::clone(&failed),
            errors: Arc::clone(&errors),
            thread: Mutex::new(None),
        };
        let handle = std::thread::Builder::new()
            .name("kvm-encoder".into())
            .spawn({
                let encoder_mode = Arc::clone(&encoder.mode);
                let encoder_slot = Arc::clone(&encoder.slot);
                let encoder_stop = Arc::clone(&stop);
                let encoder_failed = Arc::clone(&failed);
                let encoder_errors = Arc::clone(&errors);
                move || {
                    run(
                        source,
                        encoder_mode,
                        encoder_slot,
                        encoder_stop,
                        encoder_failed,
                        encoder_errors,
                    );
                }
            })
            .ok();
        *encoder
            .thread
            .lock()
            .expect("encoder thread is not poisoned") = handle;
        encoder
    }

    /// The next picture of the stream.
    pub fn next_frame(&self) -> Option<EncodedFrame> {
        self.slot.next_frame()
    }

    /// True when the child died and no more pictures will come.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// What the child wrote on its error output.
    pub fn error(&self) -> String {
        self.errors
            .lock()
            .expect("error text is not poisoned")
            .clone()
    }

    /// Stops the child.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self
            .thread
            .lock()
            .expect("encoder thread is not poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs the child until the mode changes or the encoder is stopped.
///
/// The mode is read again on every turn, because the user can change the size
/// or the rate of the stream while the server runs.
fn run(
    source: String,
    mode: Arc<Mutex<StreamMode>>,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    errors: Arc<Mutex<String>>,
) {
    while !stop.load(Ordering::Relaxed) {
        let current = *mode.lock().expect("stream mode is not poisoned");
        let mut child = match spawn(&source, current) {
            Ok(child) => child,
            Err(error) => {
                *errors.lock().expect("error text is not poisoned") = error;
                failed.store(true, Ordering::Relaxed);
                return;
            }
        };
        let reader = match read_frames(&mut child, Arc::clone(&slot)) {
            Ok(reader) => reader,
            Err(error) => {
                *errors.lock().expect("error text is not poisoned") = error;
                failed.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut dead = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let now = *mode.lock().expect("stream mode is not poisoned");
            if now != current {
                break;
            }
            if reader.finished() {
                let error = reader.error();
                if !error.is_empty() {
                    *errors.lock().expect("error text is not poisoned") = error;
                }
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        reader.join();
        if dead {
            failed.store(true, Ordering::Relaxed);
            return;
        }
    }
}

/// Starts the child process.
fn spawn(source: &str, mode: StreamMode) -> Result<Child, String> {
    let mut child = Command::new("python3")
        .args([
            "-",
            source,
            &mode.width.to_string(),
            &mode.height.to_string(),
            &mode.fps.to_string(),
            &mode.quality.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("python3 is not available: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("the encoder has no input")?;
    use std::io::Write;
    stdin
        .write_all(SCRIPT.as_bytes())
        .map_err(|error| error.to_string())?;
    drop(stdin);
    Ok(child)
}

/// Reads the pictures of the child and hands them to the slot.
struct Reader {
    finished: Arc<AtomicBool>,
    errors: Arc<Mutex<String>>,
    thread: JoinHandle<()>,
}

impl Reader {
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    fn error(&self) -> String {
        self.errors
            .lock()
            .expect("error text is not poisoned")
            .clone()
    }

    fn join(self) {
        let _ = self.thread.join();
    }
}

/// Reads the pictures of the child and publishes them in the slot.
fn read_frames(child: &mut Child, slot: Arc<FrameSlot>) -> Result<Reader, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or("the encoder has no picture output")?;
    let stderr = child.stderr.take();
    let finished = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&errors);
    let done = Arc::clone(&finished);
    let thread = std::thread::spawn(move || {
        let mut stdout = stdout;
        read_pictures(&mut stdout, &slot);
        done.store(true, Ordering::Relaxed);
    });
    if let Some(stderr) = stderr {
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || {
            // The child writes the reason of a failure here. The user cannot
            // see it, so it goes to the log of the server.
            report_errors(stderr, sink);
        });
    }
    Ok(Reader {
        finished,
        errors,
        thread,
    })
}

/// Shows what the child writes on its error output.
fn report_errors(mut stderr: impl Read, sink: Arc<Mutex<String>>) {
    let mut kept = String::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let text = String::from_utf8_lossy(&chunk[..n]).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                kept.push_str(&text);
                if kept.len() > 4000 {
                    kept = kept[kept.len() - 4000..].to_string();
                }
                tracing::warn!(%text, "the encoder wrote an error");
                *sink.lock().expect("error text is not poisoned") = kept.clone();
            }
        }
    }
}

/// Reads the pictures of the child until it ends.
fn read_pictures(stdout: &mut impl Read, slot: &Arc<FrameSlot>) {
    loop {
        let mut header = [0u8; HEADER_LEN];
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
        if header[13] != CODEC_H264 {
            return;
        }
        let mut payload = vec![0u8; len];
        if stdout.read_exact(&mut payload).is_err() {
            return;
        }
        slot.publish(EncodedFrame {
            header: kvm_protocol::messages::FrameHeader {
                pts_ms: 0,
                keyframe,
                width: width as u16,
                height: height as u16,
                codec: codec::wire_value(codec::H264),
            },
            payload: Arc::new(payload),
        });
    }
}
