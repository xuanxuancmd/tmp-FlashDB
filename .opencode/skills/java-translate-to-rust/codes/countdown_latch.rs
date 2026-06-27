use futures::task::AtomicWaker;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

struct CountDownLatchInner {
    count: AtomicUsize,
    waker: AtomicWaker,
}

#[derive(Clone)]
pub struct CountDownLatch {
    inner: Arc<CountDownLatchInner>,
}

impl std::fmt::Debug for CountDownLatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountDownLatch")
            .field("count", &self.inner.count.load(Ordering::Acquire))
            .finish()
    }
}

impl CountDownLatch {
    pub fn new(count: usize) -> Self {
        Self {
            inner: Arc::new(CountDownLatchInner {
                count: AtomicUsize::new(count),
                waker: AtomicWaker::new(),
            }),
        }
    }

    pub fn count_down(&self) {
        loop {
            let current = self.inner.count.load(Ordering::Acquire);
            if current == 0 {
                return;
            }

            if self
                .inner
                .count
                .compare_exchange(current, current - 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // 成功减少计数
                // 如果减少后计数为 0，唤醒所有等待者
                if current == 1 {
                    self.inner.waker.wake();
                }
                break;
            }
            // 如果 compare_exchange 失败（计数被其他线程修改），重试
        }
    }

    /// Returns true if the count has reached zero.
    ///
    /// Corresponds to Java's implicit check via `getCount() == 0`.
    pub fn is_done(&self) -> bool {
        self.inner.count.load(Ordering::Acquire) == 0
    }

    /// Returns the current count value.
    ///
    /// Corresponds to Java's `CountDownLatch.getCount()`.
    pub fn get_count(&self) -> usize {
        self.inner.count.load(Ordering::Acquire)
    }

    /// Blocks the current thread until the latch reaches zero.
    ///
    /// This is a synchronous blocking wait, suitable for non-async contexts.
    /// In Java, this is `CountDownLatch.await()`.
    pub fn await_sync(&self) {
        while self.inner.count.load(Ordering::Acquire) > 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Blocks the current thread until the latch reaches zero, with timeout.
    ///
    /// Returns `true` if the count reached zero within the timeout,
    /// `false` if the timeout elapsed.
    ///
    /// This is a synchronous blocking wait with timeout, suitable for non-async contexts.
    /// In Java, this is `CountDownLatch.await(timeout, TimeUnit)`.
    ///
    /// # Arguments
    /// * `timeout_ms` - Maximum time to wait in milliseconds
    ///
    /// # Returns
    /// * `true` - Latch reached zero before timeout
    /// * `false` - Timeout elapsed before latch reached zero
    pub fn await_timeout(&self, timeout_ms: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        while self.inner.count.load(Ordering::Acquire) > 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        true
    }
}

impl Future for CountDownLatch {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 先检查计数是否为0
        let count = self.inner.count.load(Ordering::Acquire);
        if count == 0 {
            return Poll::Ready(());
        }

        // 注册 waker，以便被唤醒
        self.inner.waker.register(cx.waker());

        // 再次检查，防止在注册 waker 后计数变为0
        let count = self.inner.count.load(Ordering::Acquire);
        if count == 0 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_new() {
        let latch = CountDownLatch::new(3);
        assert_eq!(latch.get_count(), 3);
        assert!(!latch.is_done());
    }

    #[test]
    fn test_count_down() {
        let latch = CountDownLatch::new(2);
        latch.count_down();
        assert_eq!(latch.get_count(), 1);
        assert!(!latch.is_done());

        latch.count_down();
        assert_eq!(latch.get_count(), 0);
        assert!(latch.is_done());
    }

    #[test]
    fn test_count_down_already_zero() {
        let latch = CountDownLatch::new(1);
        latch.count_down();
        latch.count_down(); // Should not panic, just return
        assert_eq!(latch.get_count(), 0);
    }

    #[test]
    fn test_await_sync() {
        let latch = CountDownLatch::new(1);
        let latch_clone = latch.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            latch_clone.count_down();
        });

        // This should complete after ~50ms
        latch.await_sync();
        assert!(latch.is_done());
    }

    #[test]
    fn test_await_timeout_success() {
        let latch = CountDownLatch::new(1);
        let latch_clone = latch.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            latch_clone.count_down();
        });

        // Wait with timeout of 200ms - should succeed
        let result = latch.await_timeout(200);
        assert!(result);
        assert!(latch.is_done());
    }

    #[test]
    fn test_await_timeout_failure() {
        let latch = CountDownLatch::new(1);

        // Don't count down, just wait with short timeout
        let result = latch.await_timeout(50);
        assert!(!result);
        assert!(!latch.is_done());
    }

    #[test]
    fn test_already_zero_await() {
        let latch = CountDownLatch::new(0);

        // Should return immediately
        assert!(latch.is_done());
        assert!(latch.await_timeout(10));
    }

    #[test]
    fn test_multi_thread_count_down() {
        let latch = CountDownLatch::new(5);

        let threads: Vec<_> = (0..5)
            .map(|_| {
                let latch_clone = latch.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(10));
                    latch_clone.count_down();
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        assert!(latch.is_done());
        assert!(latch.await_timeout(10));
    }
}
