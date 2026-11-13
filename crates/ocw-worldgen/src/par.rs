//! Parallel map with dynamic scheduling (spec §4.7, "Build profile").
//!
//! The world build's parallel stages used to split their work into one
//! contiguous chunk per thread. Work items differ a lot in cost (a DTM file
//! outside the part is skipped after its header, one inside is decoded and
//! resampled; a tile in a town has many more roads near its vertices than
//! one in a forest), so the thread with the expensive chunk finished long
//! after the others. Here each thread takes the next item when it is done;
//! the results come back in the order of the items, so the output is the
//! same as a sequential map.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The number of threads the world build uses (all cores).
pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZero::get)
}

/// `items.iter().map(f).collect()`, on `threads` threads (at least one),
/// each taking the next unclaimed item. Results are in item order.
pub fn map<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = threads.clamp(1, items.len().max(1));
    if n == 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut parts: Vec<(usize, R)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..n)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else {
                            return done;
                        };
                        done.push((i, f(item)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread"))
            .collect()
    });
    parts.sort_unstable_by_key(|p| p.0);
    parts.into_iter().map(|p| p.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_the_item_order_and_every_item_runs_once() {
        let items: Vec<u64> = (0..1000).collect();
        let calls = AtomicUsize::new(0);
        // Uneven work: some items take much longer.
        let out = map(&items, 8, |&i| {
            calls.fetch_add(1, Ordering::Relaxed);
            if i % 97 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            i * i
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1000);
        assert_eq!(out, items.iter().map(|i| i * i).collect::<Vec<_>>());
    }

    #[test]
    fn small_inputs_and_one_thread() {
        assert!(map(&[] as &[u8], 4, |&x| x).is_empty());
        assert_eq!(map(&[3], 16, |&x| x + 1), [4]);
        assert_eq!(map(&[1, 2, 3], 0, |&x| x * 2), [2, 4, 6]);
        assert!(threads() >= 1);
    }

    #[test]
    fn work_is_shared_when_items_are_uneven() {
        // One slow item first: with one contiguous chunk per thread its
        // chunk would hold a quarter of the items; here the other threads
        // take all the rest while it runs.
        let items: Vec<usize> = (0..64).collect();
        let by: Vec<std::thread::ThreadId> = map(&items, 4, |&i| {
            if i == 0 {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            std::thread::current().id()
        });
        let first = by[0];
        let with_first = by.iter().filter(|t| **t == first).count();
        assert!(
            with_first < 16,
            "{with_first} of 64 on the slow item's thread"
        );
    }
}
