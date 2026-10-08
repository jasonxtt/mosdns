//! Keeping the resident size near the live heap across large rule builds.
//!
//! glibc keeps memory freed by a finished rule build (the previous matcher
//! generation and the build's temporaries) resident: it shrinks a heap only
//! from its top and never shrinks a thread arena below its peak. Three
//! settings fix that for this host (measurements in
//! `docs/rust/validation-records/10-08-mem-1/`):
//!
//! - a fixed mmap threshold, so every allocation of 128 KiB or more (matcher
//!   tables, build vectors) is its own mapping and is unmapped when freed. By
//!   default glibc raises the threshold after such a free, and later large
//!   blocks then come from, and fragment, the heap;
//! - `malloc_trim(0)` after a build is published, which releases every whole
//!   free page of every arena;
//! - no transparent huge pages for this process: with the common `always`
//!   setting, `khugepaged` refills the released pages of any 2 MiB range that
//!   still holds one live page, and the resident size climbs back.
//!
//! Other C libraries and systems: the glibc settings are no-ops.
#![allow(unsafe_code)]

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
    fn malloc_trim(pad: usize) -> std::ffi::c_int;
}

/// Fixes glibc's mmap threshold at 128 KiB (its initial value), which also
/// stops its dynamic adjustment, and turns transparent huge pages off for
/// this process. Call once at process start, before the configuration is
/// compiled.
pub fn configure_process_memory() {
    #[cfg(target_os = "linux")]
    let _ = rustix::thread::disable_transparent_huge_pages(true);
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        /// `M_MMAP_THRESHOLD` in glibc's `malloc.h`.
        const M_MMAP_THRESHOLD: std::ffi::c_int = -3;
        // SAFETY: `mallopt` takes no pointers; it only changes a glibc
        // allocator parameter and is thread-safe.
        unsafe {
            mallopt(M_MMAP_THRESHOLD, 128 * 1024);
        }
    }
}

/// Releases the free pages of every malloc arena. Call it off the DNS thread
/// (it takes each arena's lock in turn; a few milliseconds on the production
/// rule set) after a build's results are published and its temporaries and
/// the replaced generation have been dropped.
pub(crate) fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: `malloc_trim` takes no pointers; it only returns free pages
        // of glibc's own arenas to the kernel and is thread-safe.
        unsafe {
            malloc_trim(0);
        }
    }
}
