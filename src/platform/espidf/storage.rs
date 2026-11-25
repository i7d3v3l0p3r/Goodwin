use anyhow::{Result, ensure};
use esp_idf_svc::sys;

pub(super) const MOUNT: &str = "/addons";
const MOUNT_C: &std::ffi::CStr = c"/addons";

pub(super) fn mount() -> Result<()> {
    let path = MOUNT_C;
    let label = c"plugins";
    let mut config = sys::esp_vfs_fat_mount_config_t {
        max_files: 8,
        allocation_unit_size: 4096,
        ..Default::default()
    };
    let mut handle = 0;
    // SAFETY: Static NUL-terminated paths and valid out pointers. The mount lasts until reset.
    let first = unsafe {
        sys::esp_vfs_fat_spiflash_mount_rw_wl(path.as_ptr(), label.as_ptr(), &config, &mut handle)
    };
    if first != sys::ESP_OK {
        // SAFETY: The partition descriptor is shared and remains valid for the boot lifetime.
        let partition = partition();
        let mut sample = [0_u8; 256];
        // An erased partition is initialized once. Mount errors on an existing volume preserve data.
        ensure!(!partition.is_null(), "Add-on partition is missing");
        // SAFETY: Read at most one small block from the valid partition to a writable buffer.
        let result = unsafe {
            sys::esp_partition_read(partition, 0, sample.as_mut_ptr().cast(), sample.len())
        };
        ensure!(
            result == sys::ESP_OK && sample.iter().all(|byte| *byte == 0xff),
            "Add-on volume could not be mounted ({first})"
        );
        config.format_if_mount_failed = true;
        // SAFETY: Same arguments and lifetime as the first mount attempt.
        let result = unsafe {
            sys::esp_vfs_fat_spiflash_mount_rw_wl(
                path.as_ptr(),
                label.as_ptr(),
                &config,
                &mut handle,
            )
        };
        ensure!(result == sys::ESP_OK, "Initialize add-on volume ({result})");
    }
    Ok(())
}

#[cfg(feature = "wasm-plugins")]
pub(super) fn has_partition() -> bool {
    !partition().is_null()
}

pub(super) fn usage() -> Option<serde_json::Value> {
    let (mut total, mut free) = (0, 0);
    // SAFETY: The path is static and both byte counters are valid out pointers.
    let result = unsafe { sys::esp_vfs_fat_info(MOUNT_C.as_ptr(), &mut total, &mut free) };
    (result == sys::ESP_OK).then(
        || serde_json::json!({"total_bytes": total, "used_bytes": total.saturating_sub(free)}),
    )
}

fn partition() -> *const sys::esp_partition_t {
    // SAFETY: ESP-IDF owns the descriptor and the partition label is static.
    unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
            sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_FAT,
            c"plugins".as_ptr(),
        )
    }
}
