//! Sharing independent pieces of work out among a few threads, where threads can be started
//! (not in the browser), with the results in order: the same whichever thread did what.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Most threads a parallel task runs on.
const MAX_THREADS: usize = 8;

/// `f` of `0..n`, in order, shared out among a few threads when threads can be started (each
/// takes the next index nobody has taken, so a costly one does not hold the others up).
pub(crate) fn each_in_parallel<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |t| t.get())
        .min(MAX_THREADS)
        .min(n);
    if threads < 2 {
        return (0..n).map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let work = || {
        let mut done = Vec::new();
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            if k >= n {
                return done;
            }
            done.push((k, f(k)));
        }
    };
    let mut done = std::thread::scope(|s| {
        // A helper that cannot be started leaves its share to the others and to this thread.
        let helpers: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(s, work).ok())
            .collect();
        let mut done = work();
        for h in helpers {
            done.extend(h.join().expect("a worker thread panicked"));
        }
        done
    });
    done.sort_by_key(|(k, _)| *k);
    done.into_iter().map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_in_order() {
        let squares = each_in_parallel(1000, |k| k * k);
        assert_eq!(squares, (0..1000).map(|k| k * k).collect::<Vec<_>>());
        assert!(each_in_parallel(0, |k| k).is_empty());
    }
}
