//! The frame source.
//!
//! The server does not know how the picture is produced. It asks a
//! [`FrameSource`] for the next encoded frame and ships it as is. The real
//! implementation captures the screen through the XDG portal; the synthetic
//! one draws a test pattern, so the whole pipeline can be exercised without
//! a desktop session.

use std::sync::atomic::{AtomicU64, Ordering};

use kvm_protocol::messages::codec;
use kvm_protocol::messages::FrameHeader;

/// An encoded frame, ready to be sealed and sent.
pub struct EncodedFrame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

/// Something that produces frames.
pub trait FrameSource: Send + Sync {
    /// Blocks until the next frame is ready. `None` ends the stream.
    fn next_frame(&self) -> Option<EncodedFrame>;
}

/// A test pattern, encoded as JPEG.
///
/// It exists so the server, the protocol and the phone can be tested end to
/// end without a compositor.
pub struct SyntheticSource {
    width: u16,
    height: u16,
    quality: u8,
    counter: AtomicU64,
}

impl SyntheticSource {
    pub fn new(width: u16, height: u16, quality: u8) -> Self {
        SyntheticSource {
            width,
            height,
            quality,
            counter: AtomicU64::new(0),
        }
    }
}

impl FrameSource for SyntheticSource {
    fn next_frame(&self) -> Option<EncodedFrame> {
        let index = self.counter.fetch_add(1, Ordering::Relaxed);
        let width = self.width as u32;
        let height = self.height as u32;
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
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut payload, self.quality);
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
                width: self.width,
                height: self.height,
                codec: codec::JPEG as u8,
            },
            payload,
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
        assert_eq!(frame.payload.len() > 100, true);
        assert_eq!(&frame.payload[0..2], &[0xff, 0xd8]);
        assert_eq!(frame.header.codec, codec::JPEG as u8);
    }

    #[test]
    fn consecutive_frames_differ() {
        let source = SyntheticSource::new(64, 48, 70);
        let first = source.next_frame().expect("a frame");
        let second = source.next_frame().expect("a frame");
        assert_ne!(first.payload, second.payload);
    }
}
