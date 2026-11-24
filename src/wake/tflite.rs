use std::{ffi::c_void, ptr::NonNull, sync::Mutex};

use anyhow::{Context, Result, ensure};

use super::FRAME_SAMPLES;

unsafe extern "C" {
    fn goodwin_tflite_open(bytes: *const u8, len: usize) -> *mut c_void;
    fn goodwin_tflite_close(handle: *mut c_void);
    fn goodwin_tflite_reset(handle: *mut c_void);
    fn goodwin_tflite_step(handle: *mut c_void, pcm: *const i16) -> f32;
}

static OPEN: Mutex<()> = Mutex::new(());

pub(super) struct TfliteModel {
    handle: NonNull<c_void>,
    _bytes: Vec<u8>,
}

// SAFETY: Each interpreter is exclusively owned. The shared operation resolver is
// initialized under OPEN and is read-only during inference.
unsafe impl Send for TfliteModel {}

impl TfliteModel {
    pub(super) fn load(bytes: Vec<u8>) -> Result<Self> {
        let _open = OPEN.lock().expect("TFLite loader lock");
        // The interpreter borrows the model data until it is destroyed.
        let handle = NonNull::new(unsafe { goodwin_tflite_open(bytes.as_ptr(), bytes.len()) })
            .context("TFLite Micro could not initialize the wake model")?;
        Ok(Self {
            handle,
            _bytes: bytes,
        })
    }

    pub(super) fn predict(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        let score = unsafe { goodwin_tflite_step(self.handle.as_ptr(), frame.as_ptr()) };
        ensure!((0.0..=1.0).contains(&score), "TFLite wake inference failed");
        Ok(score)
    }

    pub(super) fn reset(&mut self) {
        unsafe { goodwin_tflite_reset(self.handle.as_ptr()) };
    }
}

impl Drop for TfliteModel {
    fn drop(&mut self) {
        unsafe { goodwin_tflite_close(self.handle.as_ptr()) };
    }
}
