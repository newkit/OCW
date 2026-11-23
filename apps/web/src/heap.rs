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

/// The heap's steps (v5.42, the iPhone's memory): each point of a frame
/// where the heap or its peak moved by 4 MB or more since the last mark,
/// as (frame × 10 + stage, live, peak) – where a ride's start peak comes
/// from. Stages: 1 First, 2 PreUpdate, 3 Update, 4 PostUpdate, 5 Last;
/// the render world's 6 PrepareAssets, 7 Queue, 8 Render, 9 Cleanup. At
/// most [`TRACE_MAX`].
static TRACE: std::sync::Mutex<Vec<(u32, u64, u64)>> = std::sync::Mutex::new(Vec::new());
/// The marks kept.
pub const TRACE_MAX: usize = 400;
const TRACE_STEP: u64 = 4 << 20;

/// The frame the main world is in (set in `First`).
pub static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Marks `stage` of the current frame in the trace.
pub fn mark(stage: u32) {
    trace(FRAME.load(std::sync::atomic::Ordering::Relaxed) * 10 + stage);
}

/// Marks point `frame` in the trace if the heap moved enough.
pub fn trace(frame: u32) {
    let (live, peak) = (live(), peak());
    let Ok(mut t) = TRACE.lock() else {
        return;
    };
    let moved = t
        .last()
        .is_none_or(|&(_, l, p)| live.abs_diff(l) >= TRACE_STEP || peak.abs_diff(p) >= TRACE_STEP);
    if moved && t.len() < TRACE_MAX {
        t.push((frame, live, peak));
    }
}

/// The trace so far.
pub fn trace_marks() -> Vec<(u32, u64, u64)> {
    TRACE.lock().map(|t| t.clone()).unwrap_or_default()
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
        trace(1);
        let w = vec![1u8; 8 << 20];
        trace(2);
        drop(w);
        let marks = trace_marks();
        assert!(marks.iter().any(|m| m.0 == 1) && marks.iter().any(|m| m.0 == 2));
    }
}
