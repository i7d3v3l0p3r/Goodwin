use std::{
    alloc::{GlobalAlloc, Layout, System},
    ffi::c_void,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use esp_idf_svc::sys;

static EXTERNAL_TASKS: [AtomicPtr<c_void>; 4] = [const { AtomicPtr::new(ptr::null_mut()) }; 4];

#[global_allocator]
static ALLOCATOR: PlatformAllocator = PlatformAllocator;

struct PlatformAllocator;

// SAFETY: Both allocation paths use the ESP-IDF heap. System frees and resizes
// allocations from that heap, including the PSRAM regions.
unsafe impl GlobalAlloc for PlatformAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if prefers_external() {
            // SAFETY: Layout provides a positive size and power-of-two alignment.
            let external = unsafe {
                sys::heap_caps_aligned_alloc(
                    layout.align().max(size_of::<usize>()),
                    layout.pad_to_align().size(),
                    sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT,
                )
            };
            if !external.is_null() {
                return external.cast();
            }
        }
        // SAFETY: Forward the caller's allocation layout unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, address: *mut u8, layout: Layout) {
        // SAFETY: The ESP-IDF allocator can free pointers regardless of heap capabilities.
        unsafe { System.dealloc(address, layout) }
    }

    unsafe fn realloc(&self, address: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if !prefers_external() {
            // SAFETY: Forward the caller's allocation and size unchanged.
            return unsafe { System.realloc(address, layout, size) };
        }
        // SAFETY: GlobalAlloc guarantees the new size fits a layout with the old alignment.
        let next = unsafe { Layout::from_size_align_unchecked(size, layout.align()) };
        // SAFETY: Allocate a new block before moving data so failure preserves the old block.
        let output = unsafe { self.alloc(next) };
        if !output.is_null() {
            // SAFETY: Both blocks contain at least this many bytes and cannot overlap.
            unsafe {
                ptr::copy_nonoverlapping(address, output, layout.size().min(size));
                self.dealloc(address, layout);
            }
        }
        output
    }
}

fn prefers_external() -> bool {
    let tasks = EXTERNAL_TASKS
        .each_ref()
        .map(|slot| slot.load(Ordering::Acquire));
    if tasks.iter().all(|task| task.is_null()) {
        return false;
    }
    // SAFETY: FreeRTOS returns the current task handle without allocating.
    let task: *mut c_void = unsafe { sys::xTaskGetCurrentTaskHandle() }.cast();
    !task.is_null() && tasks.contains(&task)
}

pub(super) fn external<T>(work: impl FnOnce() -> T) -> T {
    // SAFETY: Plugin operations run on a task after the scheduler has started.
    let task: *mut c_void = unsafe { sys::xTaskGetCurrentTaskHandle() }.cast();
    if task.is_null() || prefers_external() {
        return work();
    }
    let slot = EXTERNAL_TASKS.iter().find(|slot| {
        slot.compare_exchange(ptr::null_mut(), task, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    });
    struct Restore(Option<&'static AtomicPtr<c_void>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(slot) = self.0 {
                slot.store(ptr::null_mut(), Ordering::Release);
            }
        }
    }
    let _restore = Restore(slot);
    work()
}
