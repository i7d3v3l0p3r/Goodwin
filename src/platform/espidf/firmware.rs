use std::{mem::size_of, ptr};

use esp_idf_svc::{
    ota::{EspOta, SlotState},
    sys,
};
use goodwin_error::{Result, bail};
use serde::Serialize;

use crate::{diagnostics::flash_write, model::FIRMWARE_TAG};

#[derive(Serialize)]
pub(super) struct Info {
    version: &'static str,
    running_slot: String,
    max_image_bytes: usize,
    verified: bool,
}

pub(super) fn info() -> Result<Info> {
    let ota = EspOta::new()?;
    let running = ota.get_running_slot()?;
    Ok(Info {
        version: env!("CARGO_PKG_VERSION"),
        running_slot: running.label.to_string(),
        max_image_bytes: capacity(),
        verified: matches!(running.state, SlotState::Valid),
    })
}

pub(super) fn capacity() -> usize {
    // SAFETY: ESP-IDF returns a pointer into its static partition table, or null.
    unsafe { sys::esp_ota_get_next_update_partition(ptr::null()).as_ref() }
        .map_or(0, |partition| partition.size as usize)
}

pub(super) fn verify_boot(healthy: bool) -> Result<()> {
    let mut ota = EspOta::new()?;
    let running = ota.get_running_slot()?;
    if matches!(running.state, SlotState::Unverified) {
        if !healthy {
            log::error!("Firmware self-test failed; rolling back");
            bail!(
                "Firmware rollback failed: {}",
                ota.mark_running_slot_invalid_and_reboot()
            );
        }
        ota.mark_running_slot_valid()?;
        log::info!("Firmware slot {} verified", running.label);
    }
    Ok(())
}

pub(super) fn install(
    mut read: impl FnMut(&mut [u8]) -> Result<usize>,
    size: usize,
) -> Result<bool> {
    const BLOCK: usize = 4096;
    const OVERLAP: usize = FIRMWARE_TAG.len() - 1;
    const DESCRIPTOR: usize =
        size_of::<sys::esp_image_header_t>() + size_of::<sys::esp_image_segment_header_t>();
    let mut buffer = [0_u8; BLOCK + OVERLAP];
    let mut count = 0;
    while count < DESCRIPTOR + 4 {
        let received = read(&mut buffer[count..BLOCK])?;
        if received == 0 {
            return Ok(false);
        }
        count += received;
    }
    if buffer[0] != 0xe9
        || buffer[DESCRIPTOR..DESCRIPTOR + 4] != sys::ESP_APP_DESC_MAGIC_WORD.to_le_bytes()
    {
        return Ok(false);
    }
    // Stream to the inactive slot. Dropping an incomplete update leaves the boot selection intact.
    let mut ota = EspOta::new()?;
    let owner = &mut ota;
    let mut update = flash_write(move || owner.initiate_update_with_known_size(size))?;
    let mut tag_found = false;
    let mut previous = 0;
    let mut remaining = size;
    loop {
        let end = previous + count;
        tag_found |= buffer[..end]
            .windows(FIRMWARE_TAG.len())
            .any(|window| window == FIRMWARE_TAG);
        flash_write(|| update.write(&buffer[previous..end]))?;
        remaining -= count;
        if remaining == 0 {
            break;
        }
        previous = end.min(OVERLAP);
        buffer.copy_within(end - previous..end, 0);
        count = read(&mut buffer[previous..previous + remaining.min(BLOCK)])?;
        if count == 0 {
            return Ok(false);
        }
    }
    if !tag_found {
        return Ok(false);
    }
    flash_write(|| update.complete())?;
    log::info!("Staged firmware image with {size} bytes");
    Ok(true)
}
