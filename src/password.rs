use core::num::NonZeroU32;

use goodwin_error::{Context, Result, ensure, report};
use phc::{Output, PasswordHash, Salt};

use crate::panel_core::{random_bytes, valid_password_length};

const ALGORITHM: &str = "pbkdf2-sha256";
const ITERATIONS: NonZeroU32 = NonZeroU32::new(600_000).unwrap();
const SALT_LENGTH: usize = 16;
const DIGEST_LENGTH: usize = 32;

pub(crate) fn uses_current_algorithm(hash: &str) -> bool {
    hash.strip_prefix('$')
        .and_then(|hash| hash.split_once('$'))
        .is_some_and(|(algorithm, _)| algorithm == ALGORITHM)
}

#[cfg_attr(
    all(not(target_os = "espidf"), feature = "hotpath-alloc"),
    hotpath::measure
)]
pub(crate) fn hash(password: &str) -> Result<String> {
    ensure!(
        valid_password_length(password),
        "Use a password between 8 and 256 bytes"
    );
    let bytes = random_bytes::<SALT_LENGTH>()?;
    let salt = Salt::new(&bytes).map_err(|error| report!("{error}"))?;
    let digest = derive(password, &bytes, ITERATIONS)?;
    Ok(format!("${ALGORITHM}$i={ITERATIONS}${salt}${digest}"))
}

#[cfg_attr(
    all(not(target_os = "espidf"), feature = "hotpath-alloc"),
    hotpath::measure
)]
pub(crate) fn verify(hash: &str, password: &str) -> Result<bool> {
    if !valid_password_length(password) {
        return Ok(false);
    }
    ensure!(
        uses_current_algorithm(hash),
        "Unsupported panel password hash algorithm"
    );
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
            && (ITERATIONS.get()..=1_000_000).contains(&iterations.get())
            && expected.as_ref().len() == DIGEST_LENGTH,
        "Invalid password hash parameters"
    );
    #[cfg(target_os = "espidf")]
    {
        Ok(&derive(password, salt, iterations)? == expected)
    }
    #[cfg(not(target_os = "espidf"))]
    {
        Ok(ring::pbkdf2::verify(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            iterations,
            salt,
            password.as_bytes(),
            expected.as_ref(),
        )
        .is_ok())
    }
}

fn derive(password: &str, salt: &[u8], iterations: NonZeroU32) -> Result<Output> {
    let mut digest = [0; DIGEST_LENGTH];
    #[cfg(target_os = "espidf")]
    {
        use esp_idf_svc::sys;

        // SAFETY: A null task handle selects the calling FreeRTOS task.
        // The PBKDF2 pointers refer to live slices of the given lengths.
        let result = unsafe {
            let task = core::ptr::null_mut();
            let priority = sys::uxTaskPriorityGet(task);
            // Share CPU time with the idle task so it can feed the watchdog.
            sys::vTaskPrioritySet(task, 0);
            let result = sys::mbedtls_pkcs5_pbkdf2_hmac_ext(
                sys::mbedtls_md_type_t_MBEDTLS_MD_SHA256,
                password.as_ptr(),
                password.len(),
                salt.as_ptr(),
                salt.len(),
                iterations.get(),
                DIGEST_LENGTH as u32,
                digest.as_mut_ptr(),
            );
            sys::vTaskPrioritySet(task, priority);
            result
        };
        ensure!(result == 0, "Derive panel password hash: {result:#x}");
    }
    #[cfg(not(target_os = "espidf"))]
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut digest,
    );
    Output::new(&digest).map_err(|error| report!("{error}"))
}

#[cfg(all(test, not(target_os = "espidf")))]
mod tests {
    use super::*;

    // Independently calculated with Python's hashlib.pbkdf2_hmac.
    const HASH: &str = "$pbkdf2-sha256$i=600000$AAECAwQFBgcICQoLDA0ODw$rxOkXUrNNb6umYJpYjEkgK9K69Au/c8gGb3WKIrArnI";
    const PASSWORD: &str = "пароль Goodwin";

    #[test]
    fn verifies_a_reference_sha256_password_hash() {
        assert!(verify(HASH, PASSWORD).unwrap());
        assert!(!verify(HASH, "another password").unwrap());
        let hash = hash(PASSWORD).unwrap();
        assert!(uses_current_algorithm(&hash));
        assert!(verify(&hash, PASSWORD).unwrap());
    }

    #[test]
    fn rejects_password_hashes_with_an_unsupported_cost() {
        for cost in ["0", "599999", "1000001"] {
            assert!(verify(&HASH.replace("600000", cost), PASSWORD).is_err());
        }
    }
}
