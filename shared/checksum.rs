#[cfg(not(target_os = "espidf"))]
use std::io::{self, Write};

#[cfg(not(target_os = "espidf"))]
use sha2::{Digest, Sha256};

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(not(target_os = "espidf"))]
#[derive(Default)]
pub(crate) struct Sha256Writer(Sha256);

#[cfg(not(target_os = "espidf"))]
impl Sha256Writer {
    pub(crate) fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}

#[cfg(not(target_os = "espidf"))]
impl Write for Sha256Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
