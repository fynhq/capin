//! Streaming animated WebP encoder backed by libwebp.
//!
//! Frames arrive as top-down BGRA buffers from the platform capture backends.
//! libwebp compresses each frame incrementally; only the muxed animation is
//! held until [`WebpEncoder::finish`], so memory use stays bounded.

use std::ffi::{CStr, c_void};
use std::path::Path;
use std::ptr::NonNull;

use core_types::PhysicalSize;
use libwebp_sys::{
    WEBP_ENCODER_ABI_VERSION, WebPAnimEncoder, WebPAnimEncoderAdd, WebPAnimEncoderAssemble,
    WebPAnimEncoderDelete, WebPAnimEncoderGetError, WebPAnimEncoderNewInternal,
    WebPAnimEncoderOptions, WebPAnimEncoderOptionsInitInternal, WebPConfig, WebPConfigInitInternal,
    WebPData, WebPFree, WebPGetMuxABIVersion, WebPPicture, WebPPictureFree, WebPPictureImportBGRA,
    WebPPictureInitInternal, WebPPictureRescale, WebPPreset, WebPValidateConfig,
};

use crate::RecordError;

const METHOD: i32 = 2;

/// Encodes one animated WebP recording.
///
/// The encoder is bound to a single recording thread. It is safe to move
/// between threads, but not to use concurrently.
pub struct WebpEncoder {
    encoder: Option<NonNull<WebPAnimEncoder>>,
    config: WebPConfig,
    input: PhysicalSize,
    output: PhysicalSize,
    frames: u64,
    last_timestamp_ms: i32,
}

// SAFETY: libwebp's animation encoder is a heap allocation without thread
// affinity. The recorder owns it exclusively and drops it on one thread.
unsafe impl Send for WebpEncoder {}

impl WebpEncoder {
    /// Creates an encoder for `input` frames scaled to `output`.
    pub fn new(
        input: PhysicalSize,
        output: PhysicalSize,
        _fps: u32,
        quality: f32,
    ) -> Result<Self, RecordError> {
        if input.is_empty() || output.is_empty() {
            return Err(RecordError::EmptyDimensions);
        }
        if input.width > i32::MAX as u32 || input.height > i32::MAX as u32 {
            return Err(RecordError::Webp(
                "the recording dimensions are too large".to_owned(),
            ));
        }

        // SAFETY: every libwebp struct is initialized through its own init
        // function before use.
        let mut config = unsafe { std::mem::zeroed::<WebPConfig>() };
        let ok = unsafe {
            WebPConfigInitInternal(
                &mut config,
                WebPPreset::WEBP_PRESET_PICTURE,
                quality.clamp(0.0, 100.0),
                WEBP_ENCODER_ABI_VERSION as i32,
            )
        };
        if ok == 0 {
            return Err(RecordError::Webp(
                "could not initialize the WebP configuration".to_owned(),
            ));
        }
        config.quality = quality.clamp(0.0, 100.0);
        config.method = METHOD;
        config.thread_level = 1;
        config.alpha_quality = 100;
        if unsafe { WebPValidateConfig(&config) } == 0 {
            return Err(RecordError::Webp(
                "the WebP configuration is invalid".to_owned(),
            ));
        }

        let mut options = unsafe { std::mem::zeroed::<WebPAnimEncoderOptions>() };
        let mux_abi = WebPGetMuxABIVersion();
        if unsafe { WebPAnimEncoderOptionsInitInternal(&mut options, mux_abi) } == 0 {
            return Err(RecordError::Webp(
                "could not initialize the WebP animation options".to_owned(),
            ));
        }
        // Streaming mode: minimizing size would buffer every frame.
        options.minimize_size = 0;
        options.allow_mixed = 0;
        options.anim_params.loop_count = 1;

        let encoder = unsafe {
            WebPAnimEncoderNewInternal(output.width as i32, output.height as i32, &options, mux_abi)
        };
        let encoder = NonNull::new(encoder).ok_or_else(|| {
            RecordError::Webp("could not create the WebP animation encoder".to_owned())
        })?;

        Ok(Self {
            encoder: Some(encoder),
            config,
            input,
            output,
            frames: 0,
            last_timestamp_ms: -1,
        })
    }

    /// Adds one top-down BGRA frame.
    pub fn push_bgra(&mut self, bgra: &[u8], timestamp_ms: i64) -> Result<(), RecordError> {
        let Some(encoder) = self.encoder else {
            return Err(RecordError::Webp(
                "the WebP encoder is already finished".to_owned(),
            ));
        };
        let expected = self.input.width as usize * self.input.height as usize * 4;
        if bgra.len() < expected {
            return Err(RecordError::Webp(format!(
                "expected {expected} bytes per frame, got {}",
                bgra.len()
            )));
        }

        let mut picture = unsafe { std::mem::zeroed::<WebPPicture>() };
        if unsafe { WebPPictureInitInternal(&mut picture, WEBP_ENCODER_ABI_VERSION as i32) } == 0 {
            return Err(RecordError::Webp(
                "could not initialize the WebP picture".to_owned(),
            ));
        }
        picture.width = self.input.width as i32;
        picture.height = self.input.height as i32;
        picture.use_argb = 1;

        let stride = (self.input.width * 4) as i32;
        if unsafe { WebPPictureImportBGRA(&mut picture, bgra.as_ptr(), stride) } == 0 {
            unsafe { WebPPictureFree(&mut picture) };
            return Err(RecordError::Webp(
                "could not import the captured frame".to_owned(),
            ));
        }
        if self.output != self.input
            && unsafe {
                WebPPictureRescale(
                    &mut picture,
                    self.output.width as i32,
                    self.output.height as i32,
                )
            } == 0
        {
            unsafe { WebPPictureFree(&mut picture) };
            return Err(RecordError::Webp(
                "could not scale the captured frame".to_owned(),
            ));
        }

        let timestamp = (timestamp_ms.clamp(0, i32::MAX as i64) as i32)
            .max(self.last_timestamp_ms.saturating_add(1));
        let ok =
            unsafe { WebPAnimEncoderAdd(encoder.as_ptr(), &mut picture, timestamp, &self.config) };
        if ok == 0 {
            let error = encoder_error(encoder);
            unsafe { WebPPictureFree(&mut picture) };
            return Err(RecordError::Webp(error));
        }
        unsafe { WebPPictureFree(&mut picture) };

        self.frames += 1;
        self.last_timestamp_ms = timestamp;
        Ok(())
    }

    /// Finalizes the animation and writes it to `path`.
    ///
    /// `end_ms` extends the last frame to the real recording duration.
    pub fn finish(mut self, end_ms: i64, path: &Path) -> Result<(), RecordError> {
        if self.frames == 0 {
            return Err(RecordError::Webp(
                "no frames were captured before the recording stopped".to_owned(),
            ));
        }
        let encoder = self
            .encoder
            .take()
            .ok_or_else(|| RecordError::Webp("the WebP encoder is already finished".to_owned()))?;
        let end_timestamp =
            (end_ms.clamp(0, i32::MAX as i64) as i32).max(self.last_timestamp_ms.saturating_add(1));

        if unsafe {
            WebPAnimEncoderAdd(
                encoder.as_ptr(),
                std::ptr::null_mut(),
                end_timestamp,
                std::ptr::null(),
            )
        } == 0
        {
            let error = encoder_error(encoder);
            unsafe { WebPAnimEncoderDelete(encoder.as_ptr()) };
            return Err(RecordError::Webp(error));
        }

        let mut data = unsafe { std::mem::zeroed::<WebPData>() };
        if unsafe { WebPAnimEncoderAssemble(encoder.as_ptr(), &mut data) } == 0 {
            let error = encoder_error(encoder);
            unsafe { WebPAnimEncoderDelete(encoder.as_ptr()) };
            return Err(RecordError::Webp(error));
        }

        let bytes = if data.bytes.is_null() {
            &[][..]
        } else {
            // SAFETY: libwebp owns a buffer of `data.size` bytes until it is
            // released with WebPFree below.
            unsafe { std::slice::from_raw_parts(data.bytes, data.size) }
        };
        let result = std::fs::write(path, bytes)
            .map_err(|error| RecordError::Webp(format!("could not write the recording: {error}")));

        unsafe {
            WebPFree(data.bytes as *mut c_void);
            WebPAnimEncoderDelete(encoder.as_ptr());
        }
        result
    }
}

impl Drop for WebpEncoder {
    fn drop(&mut self) {
        if let Some(encoder) = self.encoder.take() {
            // SAFETY: the encoder is owned by this value and is not used after.
            unsafe { WebPAnimEncoderDelete(encoder.as_ptr()) };
        }
    }
}

fn encoder_error(encoder: NonNull<WebPAnimEncoder>) -> String {
    let pointer = unsafe { WebPAnimEncoderGetError(encoder.as_ptr()) };
    if pointer.is_null() {
        return "unknown WebP encoder error".to_owned();
    }
    // SAFETY: libwebp returns a NUL-terminated string owned by the encoder.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libwebp_sys::{
        WebPAnimDecoderDelete, WebPAnimDecoderGetInfo, WebPAnimDecoderNewInternal,
        WebPAnimDecoderOptions, WebPAnimDecoderOptionsInitInternal, WebPAnimInfo,
        WebPGetDemuxABIVersion,
    };

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("capin-{name}-{}.webp", std::process::id()))
    }

    fn bgra(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        color.repeat((width * height) as usize)
    }

    fn decode_info(bytes: &[u8]) -> (u32, u32, u32) {
        let mut options = unsafe { std::mem::zeroed::<WebPAnimDecoderOptions>() };
        let demux_abi = WebPGetDemuxABIVersion();
        assert_ne!(
            unsafe { WebPAnimDecoderOptionsInitInternal(&mut options, demux_abi) },
            0
        );
        let data = WebPData {
            bytes: bytes.as_ptr(),
            size: bytes.len(),
        };
        let decoder = unsafe { WebPAnimDecoderNewInternal(&data, &options, demux_abi) };
        let decoder = NonNull::new(decoder).expect("decoder");
        let mut info = unsafe { std::mem::zeroed::<WebPAnimInfo>() };
        assert_ne!(
            unsafe { WebPAnimDecoderGetInfo(decoder.as_ptr(), &mut info) },
            0
        );
        let result = (info.canvas_width, info.canvas_height, info.frame_count);
        unsafe { WebPAnimDecoderDelete(decoder.as_ptr()) };
        result
    }

    #[test]
    fn writes_a_playable_animation() {
        let path = path("animation");
        let mut encoder = WebpEncoder::new(
            PhysicalSize::new(64, 32),
            PhysicalSize::new(64, 32),
            30,
            72.0,
        )
        .unwrap();
        encoder
            .push_bgra(&bgra(64, 32, [0, 0, 255, 255]), 0)
            .unwrap();
        encoder
            .push_bgra(&bgra(64, 32, [0, 255, 0, 255]), 33)
            .unwrap();
        encoder
            .push_bgra(&bgra(64, 32, [255, 0, 0, 255]), 66)
            .unwrap();
        encoder.finish(99, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        assert_eq!(decode_info(&bytes), (64, 32, 3));
    }

    #[test]
    fn scales_frames_to_the_output_canvas() {
        let path = path("scaled");
        let mut encoder = WebpEncoder::new(
            PhysicalSize::new(128, 64),
            PhysicalSize::new(64, 32),
            30,
            72.0,
        )
        .unwrap();
        encoder
            .push_bgra(&bgra(128, 64, [10, 20, 30, 255]), 0)
            .unwrap();
        encoder
            .push_bgra(&bgra(128, 64, [40, 50, 60, 255]), 33)
            .unwrap();
        encoder.finish(66, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(decode_info(&bytes), (64, 32, 2));
    }

    #[test]
    fn rejects_an_empty_recording() {
        let encoder = WebpEncoder::new(
            PhysicalSize::new(64, 32),
            PhysicalSize::new(64, 32),
            30,
            72.0,
        )
        .unwrap();
        let error = encoder.finish(0, &path("empty")).unwrap_err();
        assert!(matches!(error, RecordError::Webp(_)));
    }
}
