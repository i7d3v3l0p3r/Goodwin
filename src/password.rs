use core::num::NonZeroU32;

use goodwin_error::{Context, Result, ensure, report};
use phc::{Output, PasswordHash, Salt};

use crate::panel_core::{random_bytes, valid_password_length};

#[cfg(target_os = "espidf")]
mod espidf;

const SALT_LENGTH: usize = 16;
const SHA256_LENGTH: usize = 32;
const SHA512_LENGTH: usize = 64;
const SHA512_ITERATIONS: NonZeroU32 = NonZeroU32::new(220_000).unwrap();

#[derive(Clone, Copy)]
enum Algorithm {
    Sha256,
    Sha512,
}

impl Algorithm {
    fn from_hash(hash: &str) -> Option<Self> {
        match hash.strip_prefix('$')?.split_once('$')?.0 {
            "pbkdf2-sha256" => Some(Self::Sha256),
            "pbkdf2-sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    fn parameters(self) -> (usize, u32) {
        match self {
            Self::Sha256 => (SHA256_LENGTH, 600_000),
            Self::Sha512 => (SHA512_LENGTH, SHA512_ITERATIONS.get()),
        }
    }

    #[cfg(not(target_os = "espidf"))]
    fn ring(self) -> ring::pbkdf2::Algorithm {
        match self {
            Self::Sha256 => ring::pbkdf2::PBKDF2_HMAC_SHA256,
            Self::Sha512 => ring::pbkdf2::PBKDF2_HMAC_SHA512,
        }
    }
}

pub(crate) fn supports_hash(hash: &str) -> bool {
    Algorithm::from_hash(hash).is_some()
}

#[cfg_attr(
    all(not(target_os = "espidf"), feature = "hotpath-alloc"),
    hotpath::measure
)]
pub(crate) fn hash(password: &str) -> Result<String> {
    ensure!(
        valid_password_length(password),
        "Use a password between 8 and 256 characters"
    );
    let bytes = random_bytes::<SALT_LENGTH>()?;
    let salt = Salt::new(&bytes).map_err(|error| report!("{error}"))?;
    let digest = derive(Algorithm::Sha512, password, &bytes, SHA512_ITERATIONS)?;
    Ok(format!(
        "$pbkdf2-sha512$i={SHA512_ITERATIONS}${salt}${digest}"
    ))
}

#[cfg_attr(
    all(not(target_os = "espidf"), feature = "hotpath-alloc"),
    hotpath::measure
)]
pub(crate) fn verify(hash: &str, password: &str) -> Result<bool> {
    // Older passwords were validated by byte length and may have fewer than eight characters.
    if !(8..=1024).contains(&password.len()) {
        return Ok(false);
    }
    let algorithm =
        Algorithm::from_hash(hash).context("Unsupported panel password hash algorithm")?;
    let (digest_length, minimum_iterations) = algorithm.parameters();
    let hash = PasswordHash::new(hash).map_err(|error| report!("{error}"))?;
    let salt = hash.salt.as_ref().context("Missing password salt")?;
    let iterations = hash
        .params
        .get_decimal("i")
        .and_then(NonZeroU32::new)
        .context("Missing or invalid password cost")?;
    let expected = hash.hash.as_ref().context("Missing password digest")?;
    ensure!(
        salt.len() >= SALT_LENGTH
            && (minimum_iterations..=1_000_000).contains(&iterations.get())
            && expected.as_ref().len() == digest_length,
        "Invalid password hash parameters"
    );
    #[cfg(target_os = "espidf")]
    {
        Ok(&derive(algorithm, password, salt, iterations)? == expected)
    }
    #[cfg(not(target_os = "espidf"))]
    {
        Ok(ring::pbkdf2::verify(
            algorithm.ring(),
            iterations,
            salt,
            password.as_bytes(),
            expected.as_ref(),
        )
        .is_ok())
    }
}

fn derive(
    algorithm: Algorithm,
    password: &str,
    salt: &[u8],
    iterations: NonZeroU32,
) -> Result<Output> {
    let mut digest = [0; SHA512_LENGTH];
    let (digest_length, _) = algorithm.parameters();
    #[cfg(target_os = "espidf")]
    {
        let result = match algorithm {
            Algorithm::Sha256 => {
                let mut legacy = [0; SHA256_LENGTH];
                let result =
                    espidf::derive_sha256(password.as_bytes(), salt, iterations, &mut legacy);
                digest[..SHA256_LENGTH].copy_from_slice(&legacy);
                result
            }
            Algorithm::Sha512 => {
                espidf::derive_sha512(password.as_bytes(), salt, iterations, &mut digest)
            }
        };
        result.map_err(|error| report!("Derive panel password hash: {error:#x}"))?;
    }
    #[cfg(not(target_os = "espidf"))]
    ring::pbkdf2::derive(
        algorithm.ring(),
        iterations,
        salt,
        password.as_bytes(),
        &mut digest[..digest_length],
    );
    Output::new(&digest[..digest_length]).map_err(|error| report!("{error}"))
}

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::*;

    // Independently calculated with Python's hashlib.pbkdf2_hmac.
    const SHA256_HASH: &str = "$pbkdf2-sha256$i=600000$AAECAwQFBgcICQoLDA0ODw$rxOkXUrNNb6umYJpYjEkgK9K69Au/c8gGb3WKIrArnI";
    const SHA512_HASH: &str = "$pbkdf2-sha512$i=220000$AAECAwQFBgcICQoLDA0ODw$IuRYD2bPNZecS5FyOX+tYXMm56zoVBeMs/h2WUwIk1qXPNZn8EDGKp8XX/sfBTgfckGcEqBPlFuGWKYizXGemA";
    const PASSWORD: &str = "пароль Goodwin";

    #[test]
    fn verifies_existing_sha256_and_sha512_hashes_and_creates_sha512_hashes() {
        for saved in [SHA256_HASH, SHA512_HASH] {
            assert!(verify(saved, PASSWORD).unwrap());
            assert!(!verify(saved, "another password").unwrap());
        }
        let new_hash = hash(PASSWORD).unwrap();
        assert!(new_hash.starts_with("$pbkdf2-sha512$i=220000$"));
        assert!(verify(&new_hash, PASSWORD).unwrap());
    }

    #[test]
    fn new_passwords_count_characters_and_saved_passwords_keep_working() {
        const OLD: &str = "$pbkdf2-sha256$i=600000$AAECAwQFBgcICQoLDA0ODw$QXePZF7udJ2hNuUq2Pr81PMU5R5Ch2JZzwaTshyjRw8";
        assert!(hash("ёжик").is_err());
        assert!(verify(OLD, "ёжик").unwrap());
        assert!(valid_password_length(&"🍏".repeat(256)));
        assert!(!valid_password_length(&"🍏".repeat(257)));
    }

    #[test]
    fn rejects_password_hashes_with_an_unsupported_cost() {
        for (hash, valid, too_low) in [
            (SHA256_HASH, "600000", "599999"),
            (SHA512_HASH, "220000", "219999"),
        ] {
            for cost in ["0", too_low, "1000001"] {
                assert!(verify(&hash.replace(valid, cost), PASSWORD).is_err());
            }
        }
    }
}
