//! The frame source.
//!
//! The server does not know how the picture is produced. It asks a
//! [`FrameSource`] for the next encoded frame and ships it as is. The real
//! implementation captures the screen through the XDG portal; the synthetic
//! one draws a test pattern, so the whole pipeline can be exercised without
//! a desktop session.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kvm_protocol::messages::codec;
use kvm_protocol::messages::FrameHeader;

use crate::gstreamer::Encoder;

/// An encoded frame, ready to be sealed and sent.
#[derive(Clone)]
pub struct EncodedFrame {
    pub header: FrameHeader,
    /// The picture. It is shared, so a frame can be handed to the session
    /// without copying the whole picture again.
    pub payload: Arc<Vec<u8>>,
}

/// The size and rate a phone asked for, or the desktop set for that phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamMode {
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub quality: u8,
    /// Codec bit. Zero means "leave the codec the source already uses".
    pub codec: u16,
}

impl StreamMode {
    /// Keeps the mode inside what a virtual output and a frame can carry.
    pub fn normalize(self) -> Self {
        StreamMode {
            width: (self.width.clamp(320, 3840)) & !1,
            height: (self.height.clamp(240, 2160)) & !1,
            fps: self.fps.clamp(1, 120),
            quality: self.quality.clamp(1, 100),
            codec: self.codec,
        }
    }
}

/// Something that produces frames.
pub trait FrameSource: Send + Sync {
    /// Blocks until the next frame is ready. `None` ends the stream.
    fn next_frame(&self) -> Option<EncodedFrame>;

    /// The codecs this source can produce, as a bit mask.
    fn codecs(&self) -> u16 {
        codec::H264
    }

    /// The mode the next frames should follow.
    fn mode(&self) -> StreamMode;

    /// Asks the source for a new size or rate. A source that cannot change ignores it.
    fn set_mode(&self, _mode: StreamMode) {}

    /// What the user should be told about the capture, if anything.
    fn status(&self) -> String {
        String::new()
    }

    /// Starts the capture again after the user cancelled the share dialog.
    fn restart(&self) {}

    /// Stops the capture and gives back everything it holds: the screen it
    /// created, the encoder and the process it started.
    fn stop(&self) {}
}

/// The newest picture of a source.
///
/// Only the newest picture is kept: a picture that is late is worth less than
/// the one that replaced it.
pub struct FrameSlot {
    frame: Mutex<Option<(u64, EncodedFrame)>>,
    signal: Condvar,
    seq: AtomicU64,
    delivered: AtomicU64,
    started: Instant,
}

impl FrameSlot {
    pub fn new() -> Self {
        FrameSlot {
            frame: Mutex::new(None),
            signal: Condvar::new(),
            seq: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    /// Stores a new picture.
    pub fn publish(&self, mut frame: EncodedFrame) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        frame.header.pts_ms = self.started.elapsed().as_millis() as u64;
        *self.frame.lock().expect("frame slot is not poisoned") = Some((seq, frame));
        self.signal.notify_one();
    }

    /// The next picture, waiting for one to arrive.
    pub fn next_frame(&self) -> Option<EncodedFrame> {
        loop {
            let delivered = self.delivered.load(Ordering::Relaxed);
            let guard = self.frame.lock().expect("frame slot is not poisoned");
            if let Some((seq, frame)) = guard.as_ref() {
                if *seq != delivered {
                    self.delivered.store(*seq, Ordering::Relaxed);
                    return Some(frame.clone());
                }
            }
            let (_guard, _) = self
                .signal
                .wait_timeout(guard, Duration::from_millis(100))
                .expect("frame slot is not poisoned");
        }
    }

    /// True when a picture arrived already.
    pub fn has_frame(&self) -> bool {
        self.frame
            .lock()
            .expect("frame slot is not poisoned")
            .is_some()
    }
}

impl Default for FrameSlot {
    fn default() -> Self {
        Self::new()
    }
}

/// A test pattern, encoded as H.264.
///
/// It exists so the server, the protocol and the phone can be tested end to
/// end without a compositor.
pub struct SyntheticSource {
    mode: Arc<Mutex<StreamMode>>,
    encoder: Encoder,
}

impl SyntheticSource {
    pub fn new(width: u16, height: u16, quality: u8) -> Self {
        let mode = Arc::new(Mutex::new(
            StreamMode {
                width,
                height,
                fps: 20,
                quality,
                codec: codec::H264,
            }
            .normalize(),
        ));
        let encoder = Encoder::start(
            "videotestsrc pattern=smpte".to_string(),
            Arc::clone(&mode),
            Arc::new(FrameSlot::new()),
        );
        SyntheticSource { mode, encoder }
    }
}

impl FrameSource for SyntheticSource {
    fn mode(&self) -> StreamMode {
        *self.mode.lock().expect("stream mode is not poisoned")
    }

    fn set_mode(&self, mode: StreamMode) {
        let incoming = mode.normalize();
        let mut slot = self.mode.lock().expect("stream mode is not poisoned");
        let codec = if incoming.codec == 0 {
            slot.codec
        } else {
            incoming.codec
        };
        *slot = StreamMode { codec, ..incoming };
    }

    fn next_frame(&self) -> Option<EncodedFrame> {
        self.encoder.next_frame()
    }

    fn stop(&self) {
        self.encoder.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_slot_keeps_the_newest_picture() {
        let slot = FrameSlot::new();
        let frame = EncodedFrame {
            header: FrameHeader {
                pts_ms: 0,
                keyframe: true,
                width: 64,
                height: 48,
                codec: codec::wire_value(codec::H264),
            },
            payload: Arc::new(vec![1, 2, 3]),
        };
        slot.publish(frame.clone());
        let first = slot.next_frame().expect("a frame");
        assert_eq!(first.payload.len(), 3);
        assert!(first.header.pts_ms < 1000);
    }

    #[test]
    fn a_high_frame_rate_is_kept() {
        let mode = StreamMode {
            width: 1280,
            height: 720,
            fps: 120,
            quality: 70,
            codec: 0,
        }
        .normalize();
        assert_eq!(mode.fps, 120);
        let capped = StreamMode { fps: 240, ..mode }.normalize();
        assert_eq!(capped.fps, 120);
    }
}
