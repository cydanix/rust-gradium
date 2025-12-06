//! WaitGroup implementation for Tokio, similar to Go's sync.WaitGroup.
//!
//! # Example
//!
//! ```
//! use rust_gradium::WaitGroup;
//!
//! #[tokio::main]
//! async fn main() {
//!     let wg = WaitGroup::new();
//!
//!     for i in 0..5 {
//!         let guard = wg.add();  // Increment counter and get guard
//!         tokio::spawn(async move {
//!             let _guard = guard;  // Guard calls done() on drop
//!             println!("Task {} running", i);
//!             // work...
//!         });
//!     }
//!
//!     wg.wait().await;  // Wait for all tasks to complete
//!     println!("All tasks completed");
//! }
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// A WaitGroup waits for a collection of async tasks to finish.
///
/// Similar to Go's `sync.WaitGroup`.
#[derive(Clone)]
pub struct WaitGroup {
    inner: Arc<WaitGroupInner>,
}

struct WaitGroupInner {
    counter: AtomicUsize,
    notify: Notify,
}

/// A guard that calls `done()` when dropped.
///
/// This provides Go-like `defer wg.Done()` semantics via RAII.
pub struct WaitGroupGuard {
    inner: Arc<WaitGroupInner>,
}

impl Drop for WaitGroupGuard {
    fn drop(&mut self) {
        self.inner.done();
    }
}

impl WaitGroupInner {
    fn done(&self) {
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Counter reached zero, notify all waiters
            self.notify.notify_waiters();
        }
    }
}

impl WaitGroup {
    /// Creates a new WaitGroup with counter at 0.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(WaitGroupInner {
                counter: AtomicUsize::new(0),
                notify: Notify::new(),
            }),
        }
    }

    /// Increments the counter by 1 and returns a guard that will
    /// decrement the counter when dropped.
    ///
    /// This is the preferred way to use WaitGroup as it provides
    /// automatic cleanup similar to Go's `defer wg.Done()`.
    ///
    /// # Example
    ///
    /// ```
    /// use rust_gradium::WaitGroup;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let wg = WaitGroup::new();
    ///     let guard = wg.add();
    ///     
    ///     tokio::spawn(async move {
    ///         let _guard = guard;  // Will call done() when dropped
    ///         // do work...
    ///     });
    ///     
    ///     wg.wait().await;
    /// }
    /// ```
    pub fn add(&self) -> WaitGroupGuard {
        self.inner.counter.fetch_add(1, Ordering::SeqCst);
        WaitGroupGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Increments the counter by `n`.
    ///
    /// Use this when you need to add multiple counts at once and will
    /// call `done()` manually for each.
    pub fn add_n(&self, n: usize) {
        self.inner.counter.fetch_add(n, Ordering::SeqCst);
    }

    /// Decrements the counter by 1.
    ///
    /// When using `add()` with guards, you typically don't need to call
    /// this directly as the guard handles it automatically.
    pub fn done(&self) {
        self.inner.done();
    }

    /// Returns the current counter value.
    pub fn count(&self) -> usize {
        self.inner.counter.load(Ordering::SeqCst)
    }

    /// Waits until the counter becomes 0.
    ///
    /// If the counter is already 0, returns immediately.
    pub async fn wait(&self) {
        loop {
            let count = self.inner.counter.load(Ordering::SeqCst);
            if count == 0 {
                return;
            }
            self.inner.notify.notified().await;
            // Re-check after notification in case of spurious wakeups
            // or if someone added more work
            if self.inner.counter.load(Ordering::SeqCst) == 0 {
                return;
            }
        }
    }
}

impl Default for WaitGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[tokio::test]
    async fn test_wait_group_basic() {
        let wg = WaitGroup::new();
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..5 {
            let guard = wg.add();
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let _guard = guard;
                tokio::time::sleep(Duration::from_millis(10)).await;
                counter.fetch_add(1, Ordering::SeqCst);
            });
        }

        wg.wait().await;
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn test_wait_group_empty() {
        let wg = WaitGroup::new();
        // Should return immediately when counter is 0
        wg.wait().await;
    }

    #[tokio::test]
    async fn test_wait_group_manual_done() {
        let wg = WaitGroup::new();
        wg.add_n(3);

        assert_eq!(wg.count(), 3);

        wg.done();
        assert_eq!(wg.count(), 2);

        wg.done();
        wg.done();
        assert_eq!(wg.count(), 0);

        wg.wait().await;
    }

    #[tokio::test]
    async fn test_wait_group_clone() {
        let wg = WaitGroup::new();
        let wg2 = wg.clone();

        let guard = wg.add();
        tokio::spawn(async move {
            let _guard = guard;
            tokio::time::sleep(Duration::from_millis(10)).await;
        });

        // Both clones should see the same state
        assert_eq!(wg.count(), 1);
        assert_eq!(wg2.count(), 1);

        wg2.wait().await;
        assert_eq!(wg.count(), 0);
    }

    #[tokio::test]
    async fn test_guard_drop_on_panic() {
        let wg = WaitGroup::new();
        let guard = wg.add();

        let handle = tokio::spawn(async move {
            let _guard = guard;
            panic!("intentional panic");
        });

        // Wait for task to complete (it will panic)
        let _ = handle.await;

        // Guard should still have called done() even on panic
        assert_eq!(wg.count(), 0);
    }
}
