//! JSON encoding and decoding for typed Goodwin contracts.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::{string::String, vec::Vec};
use core::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json_core::de;

pub mod string_map;
mod syntax;

pub use serde_json_core::ser::Error as EncodeError;
pub use serde_json_core::to_slice;

#[derive(Debug)]
pub enum DecodeError {
    Syntax,
    Data(de::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => formatter.write_str("Invalid JSON syntax or nesting"),
            Self::Data(error) => error.fmt(formatter),
        }
    }
}

impl core::error::Error for DecodeError {}

impl DecodeError {
    pub fn requires_untyped_values(&self) -> bool {
        matches!(self, Self::Data(de::Error::AnyIsUnsupported))
    }

    pub fn has_generic_message(&self) -> bool {
        matches!(self, Self::Data(de::Error::CustomError))
    }
}

pub fn validate(input: &[u8]) -> Result<(), DecodeError> {
    let text = core::str::from_utf8(input).map_err(|_| DecodeError::Syntax)?;
    syntax::validate(text.as_bytes())
        .map(|_| ())
        .ok_or(DecodeError::Syntax)
}

/// Extracts selected members from a JSON object without materializing their values.
pub fn object_fields<'input, const N: usize>(
    input: &'input [u8],
    keys: [&str; N],
) -> Result<[Option<&'input [u8]>; N], DecodeError> {
    core::str::from_utf8(input).map_err(|_| DecodeError::Syntax)?;
    let mut result = [None; N];
    syntax::fields(input, |key, value| {
        let escaped;
        let key = if key.contains(&b'\\') {
            escaped = from_slice::<String>(key).ok()?;
            escaped.as_bytes()
        } else {
            &key[1..key.len() - 1]
        };
        if let Some(index) = keys.iter().position(|name| name.as_bytes() == key)
            && result[index].replace(value).is_some()
        {
            return None;
        }
        Some(())
    })
    .ok_or(DecodeError::Syntax)?;
    Ok(result)
}

pub fn from_slice<T: DeserializeOwned>(input: &[u8]) -> Result<T, DecodeError> {
    let text = core::str::from_utf8(input).map_err(|_| DecodeError::Syntax)?;
    from_str(text)
}

pub fn from_str<T: DeserializeOwned>(input: &str) -> Result<T, DecodeError> {
    with_decoder(input, |decoder| decoder.decode())?
}

/// Use a validated JSON document for a scoped decode that can borrow strings.
/// The result must own any data it keeps after the callback returns.
pub fn with_decoder<R>(
    input: &str,
    visit: impl for<'input> FnOnce(Decoder<'input>) -> R,
) -> Result<R, DecodeError> {
    // The core decoder skips unknown scalar fields without validating their
    // syntax and has no nesting limit of its own.
    let strings = syntax::validate(input.as_bytes()).ok_or(DecodeError::Syntax)?;
    let normalized;
    let input = if strings.surrogate_bytes_saved != 0 {
        // serde-json-core doesn't join UTF-16 escape pairs. Pass those code
        // points as UTF-8, preserving every other escape in the document.
        normalized = join_surrogate_pairs(input.as_bytes(), strings.surrogate_bytes_saved);
        &normalized
    } else {
        input.as_bytes()
    };
    Ok(visit(Decoder {
        input,
        escaped: strings.escaped,
    }))
}

pub struct Decoder<'input> {
    input: &'input [u8],
    escaped: usize,
}

impl<'input> Decoder<'input> {
    pub fn decode<T: Deserialize<'input>>(self) -> Result<T, DecodeError> {
        if self.escaped == 0 {
            return serde_json_core::from_slice(self.input)
                .map(|(value, _)| value)
                .map_err(DecodeError::Data);
        }
        let mut scratch = [0; 128];
        let mut extended = Vec::new();
        let buffer = if self.escaped <= scratch.len() {
            &mut scratch[..self.escaped]
        } else {
            extended.resize(self.escaped, 0);
            &mut extended
        };
        serde_json_core::from_slice_escaped(self.input, buffer)
            .map(|(value, _)| value)
            .map_err(DecodeError::Data)
    }
}

fn join_surrogate_pairs(mut input: &[u8], bytes_saved: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() - bytes_saved);
    while let Some(begin) = input.iter().position(|&byte| byte == b'\\') {
        output.extend_from_slice(&input[..begin]);
        input = &input[begin..];
        if input[1] == b'u' {
            let high = syntax::hex(&input[2..6]).expect("validated Unicode escape");
            if (0xd800..=0xdbff).contains(&high) {
                let low = syntax::hex(&input[8..12]).expect("validated surrogate pair");
                let code = 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
                let character = char::from_u32(code).expect("validated surrogate pair");
                output.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                input = &input[12..];
                continue;
            }
        }
        output.extend_from_slice(&input[..2]);
        input = &input[2..];
    }
    output.extend_from_slice(input);
    output
}

pub fn to_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, EncodeError> {
    to_vec_with_capacity(value, 128)
}

pub fn to_vec_with_capacity<T: Serialize + ?Sized>(
    value: &T,
    capacity: usize,
) -> Result<Vec<u8>, EncodeError> {
    let mut buffer = Vec::new();
    let capacity = capacity.max(128);
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| EncodeError::BufferFull)?;
    buffer.resize(capacity, 0);
    loop {
        let mut encoder = serde_json_core::ser::Serializer::new(&mut buffer);
        match value.serialize(&mut encoder) {
            Ok(()) => {
                let used = encoder.end();
                buffer.truncate(used);
                return Ok(buffer);
            }
            Err(EncodeError::BufferFull) => {
                let additional = buffer.len();
                let size = additional.checked_mul(2).ok_or(EncodeError::BufferFull)?;
                buffer
                    .try_reserve_exact(additional)
                    .map_err(|_| EncodeError::BufferFull)?;
                buffer.resize(size, 0);
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn to_string<T: Serialize + ?Sized>(value: &T) -> Result<String, EncodeError> {
    // The JSON serializer only emits ASCII syntax and UTF-8 strings.
    Ok(String::from_utf8(to_vec(value)?).expect("JSON encoder emits UTF-8"))
}

#[cfg(feature = "std")]
pub fn from_reader<T: DeserializeOwned>(mut reader: impl std::io::Read) -> std::io::Result<T> {
    let mut input = Vec::new();
    reader.read_to_end(&mut input)?;
    from_slice(&input).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(feature = "std")]
pub fn to_writer<T: Serialize + ?Sized>(
    mut writer: impl std::io::Write,
    value: &T,
) -> std::io::Result<()> {
    let bytes = to_vec(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    writer.write_all(&bytes)
}

#[cfg(feature = "std")]
pub fn to_writer_pretty<T: Serialize + ?Sized>(
    writer: impl std::io::Write,
    value: &T,
) -> std::io::Result<()> {
    use std::io::{BufWriter, Write};

    let bytes = to_vec(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut writer = BufWriter::with_capacity(1024, writer);
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if quoted {
            writer.write_all(&[byte])?;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        if matches!(byte, b'}' | b']') {
            depth -= 1;
            if !matches!(bytes[index - 1], b'{' | b'[') {
                indent(&mut writer, depth)?;
            }
        }
        writer.write_all(&[byte])?;
        match byte {
            b'{' | b'[' => {
                depth += 1;
                if !matches!(bytes.get(index + 1), Some(b'}' | b']')) {
                    indent(&mut writer, depth)?;
                }
            }
            b',' => indent(&mut writer, depth)?,
            b':' => writer.write_all(b" ")?,
            b'"' => quoted = true,
            _ => (),
        }
    }
    writer.flush()
}

#[cfg(feature = "std")]
fn indent(writer: &mut impl std::io::Write, depth: usize) -> std::io::Result<()> {
    writer.write_all(b"\n")?;
    for _ in 0..depth {
        writer.write_all(b"  ")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
