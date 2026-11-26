use std::{borrow::Cow, ffi::c_void, ptr::NonNull, sync::Mutex};

use goodwin_error::{Context, Result, ensure};

pub(crate) const FRAME_SAMPLES: usize = 1280;
const MODEL_ALIGNMENT: usize = 16;

unsafe extern "C" {
    fn goodwin_tflite_open(bytes: *const u8, len: usize) -> *mut c_void;
    fn goodwin_tflite_close(handle: *mut c_void);
    fn goodwin_tflite_reset(handle: *mut c_void);
    fn goodwin_tflite_step(handle: *mut c_void, pcm: *const i16) -> f32;
    #[cfg(all(target_os = "linux", feature = "wake-bench", feature = "hotpath-alloc"))]
    fn goodwin_tflite_buffers(handle: *mut c_void, sizes: *mut usize);
}

static OPEN: Mutex<()> = Mutex::new(());

pub(crate) struct TfliteModel {
    handle: NonNull<c_void>,
    _bytes: ModelBytes,
}

struct ModelBytes {
    storage: Cow<'static, [u8]>,
    start: usize,
}

impl ModelBytes {
    fn new(storage: Cow<'static, [u8]>) -> Result<Self> {
        if storage.as_ptr().addr().is_multiple_of(MODEL_ALIGNMENT) {
            return Ok(Self { storage, start: 0 });
        }
        let capacity = storage
            .len()
            .checked_add(MODEL_ALIGNMENT - 1)
            .context("TFLite model size overflows")?;
        let mut aligned = Vec::<u8>::new();
        aligned
            .try_reserve_exact(capacity)
            .context("Allocate aligned TFLite model")?;
        let start = aligned.as_ptr().align_offset(MODEL_ALIGNMENT);
        aligned.resize(start, 0);
        aligned.extend_from_slice(&storage);
        Ok(Self {
            storage: Cow::Owned(aligned),
            start,
        })
    }

    fn as_slice(&self) -> &[u8] {
        &self.storage[self.start..]
    }
}

// SAFETY: Each interpreter and its arena are exclusively owned and independent of
// the calling thread. OPEN serializes setup of the shared resolver and FFT tables.
// Inference and reset use them read-only.
unsafe impl Send for TfliteModel {}

impl TfliteModel {
    #[cfg(target_os = "espidf")]
    pub(crate) fn bytes(&self) -> &[u8] {
        self._bytes.as_slice()
    }

    pub(crate) fn load(bytes: impl Into<Cow<'static, [u8]>>) -> Result<Self> {
        let bytes = ModelBytes::new(bytes.into())?;
        let flatbuffer = bytes.as_slice();
        let _open = OPEN.lock().expect("TFLite loader lock");
        // SAFETY: The aligned FlatBuffer is immutable and stays allocated until close.
        let handle =
            NonNull::new(unsafe { goodwin_tflite_open(flatbuffer.as_ptr(), flatbuffer.len()) })
                .context("TFLite Micro could not initialize the wake model")?;
        Ok(Self {
            handle,
            _bytes: bytes,
        })
    }

    pub(crate) fn predict(&mut self, frame: &[i16; FRAME_SAMPLES]) -> Result<f32> {
        // SAFETY: This is a live, exclusively borrowed detector. Process reads one
        // full frame synchronously and does not retain the PCM pointer.
        let score = unsafe { goodwin_tflite_step(self.handle.as_ptr(), frame.as_ptr()) };
        ensure!((0.0..=1.0).contains(&score), "TFLite wake inference failed");
        Ok(score)
    }

    pub(crate) fn reset(&mut self) {
        // SAFETY: This is a live detector and the exclusive borrow permits mutation.
        unsafe { goodwin_tflite_reset(self.handle.as_ptr()) };
    }

    #[cfg(all(target_os = "linux", feature = "wake-bench", feature = "hotpath-alloc"))]
    pub(crate) fn buffers(&mut self) -> [usize; 3] {
        let mut sizes = [0; 3];
        // SAFETY: self owns the interpreter and sizes provides three writable size_t slots.
        unsafe { goodwin_tflite_buffers(self.handle.as_ptr(), sizes.as_mut_ptr()) };
        sizes
    }
}

impl Drop for TfliteModel {
    fn drop(&mut self) {
        // SAFETY: This is the sole owner. Its model bytes remain live through close.
        unsafe { goodwin_tflite_close(self.handle.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_storage_copies_only_misaligned_inputs() {
        #[repr(align(16))]
        struct Aligned([u8; 16]);
        static BUFFER: Aligned = Aligned(*b"abcdef0123456789");

        let aligned = ModelBytes::new(Cow::Borrowed(&BUFFER.0)).unwrap();
        assert_eq!(aligned.as_slice().as_ptr(), BUFFER.0.as_ptr());

        let input = &BUFFER.0[1..];
        let realigned = ModelBytes::new(Cow::Borrowed(input)).unwrap();
        assert_ne!(realigned.as_slice().as_ptr(), input.as_ptr());
        assert!(
            realigned
                .as_slice()
                .as_ptr()
                .addr()
                .is_multiple_of(MODEL_ALIGNMENT)
        );
        assert_eq!(realigned.as_slice(), input);
    }
}
