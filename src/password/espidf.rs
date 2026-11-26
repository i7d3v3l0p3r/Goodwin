use core::{mem::MaybeUninit, num::NonZeroU32};

use super::{SHA256_LENGTH, SHA512_LENGTH};
use esp_idf_svc::sys;

const ROUNDS_PER_TICK: u32 = 2_048;

pub(super) fn derive_sha256(
    password: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
    digest: &mut [u8; SHA256_LENGTH],
) -> Result<(), i32> {
    let task = core::ptr::null_mut();
    // SAFETY: A null handle selects the current FreeRTOS task.
    let priority = unsafe {
        let priority = sys::uxTaskPriorityGet(task);
        sys::vTaskPrioritySet(task, 0);
        priority
    };
    let result = derive::<Sha256, 64, SHA256_LENGTH, false>(password, salt, iterations, digest);
    // SAFETY: Restore the recorded priority of the current task.
    unsafe { sys::vTaskPrioritySet(task, priority) };
    result
}

pub(super) fn derive_sha512(
    password: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
    digest: &mut [u8; SHA512_LENGTH],
) -> Result<(), i32> {
    derive::<Sha512, 128, SHA512_LENGTH, true>(password, salt, iterations, digest)
}

fn derive<Hash: Sha<N>, const BLOCK: usize, const N: usize, const PAUSE: bool>(
    password: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
    digest: &mut [u8; N],
) -> Result<(), i32> {
    let mut pad = Sensitive([0; BLOCK]);
    let mut block = Sensitive([0; N]);
    let mut hash = Hash::new();
    if password.len() > BLOCK {
        hash.start()?;
        hash.update(password)?;
        hash.finish(&mut block.0)?;
        pad.0[..N].copy_from_slice(&block.0);
    } else {
        pad.0[..password.len()].copy_from_slice(password);
    }

    // Cache the SHA states after the two HMAC key pads (RFC 2104 §4).
    let mut inner = Hash::new();
    let mut outer = Hash::new();
    for byte in &mut pad.0 {
        *byte ^= 0x36;
    }
    inner.start()?;
    inner.update(&pad.0)?;
    for byte in &mut pad.0 {
        *byte ^= 0x36 ^ 0x5c;
    }
    outer.start()?;
    outer.update(&pad.0)?;
    drop(pad);

    // A key the size of the digest uses only PBKDF2 block 1 (RFC 8018 §5.2).
    hash.copy_from(&inner);
    hash.update(salt)?;
    hash.update(&1_u32.to_be_bytes())?;
    finish_hmac(&mut hash, &outer, &mut block.0)?;
    let mut accumulated = Sensitive(block.0);

    for round in 1..iterations.get() {
        if PAUSE && round % ROUNDS_PER_TICK == 0 {
            // Let the idle task feed the watchdog between batches. Both SHA calls
            // from the preceding round have already released the peripheral.
            // SAFETY: This runs in the panel password worker, outside any lock.
            unsafe { sys::vTaskDelay(1) };
        }
        hash.copy_from(&inner);
        hash.update(&block.0)?;
        finish_hmac(&mut hash, &outer, &mut block.0)?;
        for (output, byte) in accumulated.0.iter_mut().zip(&block.0) {
            *output ^= byte;
        }
    }
    *digest = accumulated.0;
    Ok(())
}

fn finish_hmac<Hash: Sha<N>, const N: usize>(
    hash: &mut Hash,
    outer: &Hash,
    block: &mut [u8; N],
) -> Result<(), i32> {
    hash.finish(block)?;
    hash.copy_from(outer);
    hash.update(block)?;
    hash.finish(block)
}

struct Sensitive<const N: usize>([u8; N]);

impl<const N: usize> Drop for Sensitive<N> {
    fn drop(&mut self) {
        // SAFETY: This array is writable for N bytes.
        unsafe { sys::mbedtls_platform_zeroize(self.0.as_mut_ptr().cast(), N) }
    }
}

trait Sha<const N: usize> {
    fn new() -> Self;
    fn start(&mut self) -> Result<(), i32>;
    fn update(&mut self, input: &[u8]) -> Result<(), i32>;
    fn finish(&mut self, digest: &mut [u8; N]) -> Result<(), i32>;
    fn copy_from(&mut self, source: &Self);
}

macro_rules! sha_context {
    ($name:ident, $length:ident, $context:ident, $init:ident, $start:ident,
     $update:ident, $finish:ident, $clone:ident, $free:ident) => {
        struct $name(sys::$context);

        impl Sha<$length> for $name {
            fn new() -> Self {
                let mut context = MaybeUninit::uninit();
                // SAFETY: The matching Mbed TLS function initializes the context.
                unsafe {
                    sys::$init(context.as_mut_ptr());
                    Self(context.assume_init())
                }
            }

            fn start(&mut self) -> Result<(), i32> {
                // SAFETY: The context is initialized and 0 selects the full digest.
                check(unsafe { sys::$start(&mut self.0, 0) })
            }

            fn update(&mut self, input: &[u8]) -> Result<(), i32> {
                // SAFETY: The context is active and the input is readable for its length.
                check(unsafe { sys::$update(&mut self.0, input.as_ptr(), input.len()) })
            }

            fn finish(&mut self, digest: &mut [u8; $length]) -> Result<(), i32> {
                // SAFETY: The output holds a full digest of the selected hash.
                check(unsafe { sys::$finish(&mut self.0, digest.as_mut_ptr()) })
            }

            fn copy_from(&mut self, source: &Self) {
                // SAFETY: Both contexts are initialized, distinct and of the same type.
                unsafe { sys::$clone(&mut self.0, &source.0) }
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: The context was initialized by the matching Mbed TLS function.
                unsafe { sys::$free(&mut self.0) }
            }
        }
    };
}

sha_context!(
    Sha256,
    SHA256_LENGTH,
    mbedtls_sha256_context,
    mbedtls_sha256_init,
    mbedtls_sha256_starts,
    mbedtls_sha256_update,
    mbedtls_sha256_finish,
    mbedtls_sha256_clone,
    mbedtls_sha256_free
);
sha_context!(
    Sha512,
    SHA512_LENGTH,
    mbedtls_sha512_context,
    mbedtls_sha512_init,
    mbedtls_sha512_starts,
    mbedtls_sha512_update,
    mbedtls_sha512_finish,
    mbedtls_sha512_clone,
    mbedtls_sha512_free
);

fn check(result: i32) -> Result<(), i32> {
    if result == 0 { Ok(()) } else { Err(result) }
}
