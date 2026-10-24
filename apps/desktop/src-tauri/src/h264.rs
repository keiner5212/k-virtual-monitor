//! H.264 encoder.
//!
//! ffmpeg's libx264 writes Annex B. Each access unit starts with an AUD.
//! The pipe goes quiet after a frame, which is how a still screen still
//! produces a picture: the unit is complete when a VCL NAL has sat idle.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct AccessUnit {
    pub bytes: Vec<u8>,
    pub keyframe: bool,
}

struct Buffer {
    bytes: Mutex<Vec<u8>>,
    cv: Condvar,
    closed: AtomicBool,
}

pub struct H264Session {
    child: Child,
    stdin: ChildStdin,
    buffer: Arc<Buffer>,
    width: u32,
    height: u32,
    fps: u16,
    quality: u8,
}

pub fn available() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| {
        let scratch = scratch_dir();
        Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .env("TMPDIR", &scratch)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains("libx264"))
            .unwrap_or(false)
    })
}

/// Repo-local scratch. ffmpeg inherits it so it does not use /tmp.
pub fn scratch_dir() -> PathBuf {
    if let Ok(path) = std::env::var("KVM_TMP") {
        let path = PathBuf::from(path);
        let _ = std::fs::create_dir_all(&path);
        return path;
    }
    let compiled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.tmp");
    if std::fs::create_dir_all(&compiled).is_ok() {
        return compiled;
    }
    let local = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".tmp");
    let _ = std::fs::create_dir_all(&local);
    local
}

impl H264Session {
    pub fn open(width: u32, height: u32, fps: u16, quality: u8) -> Result<Self, String> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err("h264 needs an even frame size".to_string());
        }
        let fps = fps.clamp(1, 60);
        let scratch = scratch_dir();
        let size = format!("{width}x{height}");
        let rate = fps.to_string();
        let crf_value = crf(quality).to_string();
        let args = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            size.as_str(),
            "-r",
            rate.as_str(),
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-profile:v",
            "baseline",
            "-pix_fmt",
            "yuv420p",
            "-g",
            rate.as_str(),
            "-keyint_min",
            rate.as_str(),
            "-bf",
            "0",
            "-crf",
            crf_value.as_str(),
            "-x264-params",
            "repeat-headers=1:aud=1",
            "-flush_packets",
            "1",
            "-f",
            "h264",
            "pipe:1",
        ];
        let mut child = Command::new("ffmpeg")
            .env("TMPDIR", &scratch)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start ffmpeg: {error}"))?;
        let stdin = child.stdin.take().ok_or("ffmpeg stdin is missing")?;
        let stdout = child.stdout.take().ok_or("ffmpeg stdout is missing")?;
        let buffer = Arc::new(Buffer {
            bytes: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            closed: AtomicBool::new(false),
        });
        let reader_buffer = Arc::clone(&buffer);
        thread::Builder::new()
            .name("kvm-h264".into())
            .spawn(move || read_stdout(stdout, reader_buffer))
            .map_err(|error| error.to_string())?;
        Ok(H264Session {
            child,
            stdin,
            buffer,
            width,
            height,
            fps,
            quality,
        })
    }

    pub fn matches(&self, width: u32, height: u32, fps: u16, quality: u8) -> bool {
        self.width == width && self.height == height && self.fps == fps && self.quality == quality
    }

    /// Writes one RGB frame and returns its access unit once the encoder goes idle.
    pub fn push(&mut self, rgb: &[u8]) -> Result<Option<AccessUnit>, String> {
        let expected = self.width as usize * self.height as usize * 3;
        if rgb.len() != expected {
            return Err("frame size does not match the h264 encoder".to_string());
        }
        self.stdin
            .write_all(rgb)
            .map_err(|error| error.to_string())?;
        self.stdin.flush().map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut last_len = 0usize;
        let mut changed_at: Option<Instant> = None;
        loop {
            let mut bytes = self
                .buffer
                .bytes
                .lock()
                .expect("h264 buffer is not poisoned");
            if bytes.len() != last_len {
                last_len = bytes.len();
                changed_at = Some(Instant::now());
            }
            let idle = changed_at
                .map(|instant| instant.elapsed() >= Duration::from_millis(8))
                .unwrap_or(false);
            if let Some(unit) = take_ready(&mut bytes, idle) {
                return Ok(Some(unit));
            }
            if self.buffer.closed.load(Ordering::Relaxed) {
                return Err("h264 encoder stopped".to_string());
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(8));
            let (next, _) = self
                .buffer
                .cv
                .wait_timeout(bytes, wait)
                .expect("h264 buffer is not poisoned");
            drop(next);
        }
    }
}

impl Drop for H264Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.buffer.closed.store(true, Ordering::Relaxed);
        self.buffer.cv.notify_all();
    }
}

fn read_stdout(mut stdout: std::process::ChildStdout, buffer: Arc<Buffer>) {
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(size) => {
                buffer
                    .bytes
                    .lock()
                    .expect("h264 buffer is not poisoned")
                    .extend_from_slice(&chunk[..size]);
                buffer.cv.notify_all();
            }
            Err(_) => break,
        }
    }
    buffer.closed.store(true, Ordering::Relaxed);
    buffer.cv.notify_all();
}

fn crf(quality: u8) -> u8 {
    let quality = i16::from(quality.clamp(1, 100));
    (40 - quality / 4).clamp(18, 36) as u8
}

fn take_ready(bytes: &mut Vec<u8>, idle: bool) -> Option<AccessUnit> {
    let auds = aud_positions(bytes);
    if auds.len() >= 2 {
        let end = auds[1];
        let unit: Vec<u8> = bytes.drain(..end).collect();
        return Some(access_unit(unit));
    }
    if idle && has_vcl(bytes) {
        let unit = std::mem::take(bytes);
        return Some(access_unit(unit));
    }
    None
}

fn access_unit(bytes: Vec<u8>) -> AccessUnit {
    AccessUnit {
        keyframe: nal_types(&bytes).contains(&5),
        bytes,
    }
}

fn has_vcl(bytes: &[u8]) -> bool {
    nal_types(bytes).iter().any(|kind| *kind == 1 || *kind == 5)
}

fn aud_positions(bytes: &[u8]) -> Vec<usize> {
    nal_spans(bytes)
        .into_iter()
        .filter(|(_, kind)| *kind == 9)
        .map(|(start, _)| start)
        .collect()
}

fn nal_types(bytes: &[u8]) -> Vec<u8> {
    nal_spans(bytes).into_iter().map(|(_, kind)| kind).collect()
}

fn nal_spans(bytes: &[u8]) -> Vec<(usize, u8)> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index + 3 < bytes.len() {
        let three_byte = bytes[index] == 0 && bytes[index + 1] == 0 && bytes[index + 2] == 1;
        let four_byte = index + 4 < bytes.len()
            && bytes[index] == 0
            && bytes[index + 1] == 0
            && bytes[index + 2] == 0
            && bytes[index + 3] == 1;
        if !three_byte && !four_byte {
            index += 1;
            continue;
        }
        let start = index;
        let header = if bytes[start + 2] == 1 {
            start + 3
        } else {
            start + 4
        };
        if header < bytes.len() {
            spans.push((start, bytes[header] & 0x1f));
        }
        index = header.saturating_add(1);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_smaller_than_the_raw_picture() {
        if !available() {
            return;
        }
        let mut session = H264Session::open(64, 64, 10, 70).expect("encoder");
        let rgb = vec![40u8; 64 * 64 * 3];
        let unit = session.push(&rgb).expect("push").expect("access unit");
        assert!(unit.keyframe);
        assert!(unit.bytes.len() > 20);
        assert!(unit.bytes.len() < rgb.len());
    }
}
