//! The live heap of the browser app (spec §14 Phase 10 B2, the memory per
//! resident tile): a counting wrapper around the system allocator (wasm's
//! dlmalloc), so the status line, `world.info` and the browser test
//! (`window.ocw.heap()`) can tell the bytes in use from the wasm memory,
//! which only grows. A few relaxed atomic operations per allocation.
//!
//! It is the global allocator of the browser build and of this crate's
//! native tests (the memory report in `world`); the desktop app is not
//! built with it.

use std::alloc::System;

use cap::Cap;

/// The system allocator with its bytes counted (`cap`: the workspace
/// forbids `unsafe`, an allocator needs it). No limit.
#[cfg(any(target_arch = "wasm32", test))]
#[global_allocator]
static COUNTING: Cap<System> = Cap::new(System, usize::MAX);

#[cfg(not(any(target_arch = "wasm32", test)))]
static COUNTING: Cap<System> = Cap::new(System, usize::MAX);

/// Bytes allocated now (0 where this is not the global allocator).
pub fn live() -> u64 {
    COUNTING.allocated() as u64
}

/// The most bytes allocated at once since the start.
pub fn peak() -> u64 {
    COUNTING.max_allocated() as u64
}

/// The status line's part: "heap 312 MB (peak 405 MB)".
pub fn summary() -> String {
    format!(
        "heap {:.0} MB (peak {:.0} MB)",
        live() as f64 / 1e6,
        peak() as f64 / 1e6
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counter follows allocations and frees, the peak keeps the most
    /// (other tests allocate in parallel, so only lower bounds hold).
    #[test]
    fn the_counter_follows_allocations() {
        let v = vec![1u8; 64 << 20];
        assert!(live() >= 64 << 20);
        assert!(peak() >= 64 << 20);
        drop(v);
        assert!(peak() >= 64 << 20);
        assert!(summary().starts_with("heap "));
    }
}
