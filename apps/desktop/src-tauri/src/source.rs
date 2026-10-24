//! The frame source.
//!
//! The server does not know how the picture is produced. It asks a
//! [`FrameSource`] for the next encoded frame and ships it as is. The real
//! implementation captures the screen through the XDG portal; the synthetic
//! one draws a test pattern, so the whole pipeline can be exercised without
//! a desktop session.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use kvm_protocol::messages::codec;
use kvm_protocol::messages::FrameHeader;

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
    /// Keeps the mode inside what a virtual output and a JPEG frame can carry.
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
        codec::JPEG
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

/// A test pattern, encoded as JPEG.
///
/// It exists so the server, the protocol and the phone can be tested end to
/// end without a compositor.
pub struct SyntheticSource {
    mode: std::sync::Mutex<StreamMode>,
    counter: AtomicU64,
}

impl SyntheticSource {
    pub fn new(width: u16, height: u16, quality: u8) -> Self {
        SyntheticSource {
            mode: std::sync::Mutex::new(StreamMode {
                width,
                height,
                fps: 20,
                quality,
                codec: codec::JPEG,
            }),
            counter: AtomicU64::new(0),
        }
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
        let index = self.counter.fetch_add(1, Ordering::Relaxed);
        let mode = self.mode();
        let width = mode.width as u32;
        let height = mode.height as u32;
        let mut rgb = vec![0u8; (width * height * 3) as usize];
        let bar = (index % 120) as u32;
        for y in 0..height {
            for x in 0..width {
                let offset = ((y * width + x) * 3) as usize;
                let in_bar =
                    (y / 20) % 2 == 0 && x >= bar * width / 120 && x < (bar + 20) * width / 120;
                let shade = (y * 255 / height.max(1)) as u8;
                rgb[offset] = if in_bar { 255 } else { shade };
                rgb[offset + 1] = if in_bar { 96 } else { shade / 2 };
                rgb[offset + 2] = if in_bar { 0 } else { 255 - shade };
            }
        }
        let mut payload = Vec::new();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut payload, mode.quality);
        if encoder
            .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .is_err()
        {
            return None;
        }
        Some(EncodedFrame {
            header: FrameHeader {
                pts_ms: index * 50,
                keyframe: true,
                width: mode.width,
                height: mode.height,
                codec: codec::wire_value(codec::JPEG),
            },
            payload: Arc::new(payload),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthetic_source_produces_jpeg_frames() {
        let source = SyntheticSource::new(64, 48, 70);
        let frame = source.next_frame().expect("a frame");
        assert!(frame.payload.len() > 100);
        assert_eq!(&frame.payload[0..2], &[0xff, 0xd8]);
        assert_eq!(frame.header.codec, codec::wire_value(codec::JPEG));
    }

    #[test]
    fn consecutive_frames_differ() {
        let source = SyntheticSource::new(64, 48, 70);
        let first = source.next_frame().expect("a frame");
        let second = source.next_frame().expect("a frame");
        assert_ne!(first.payload, second.payload);
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
