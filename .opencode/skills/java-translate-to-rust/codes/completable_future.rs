use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::oneshot;

type CompletableResult<T, E> = Result<T, FutureError<E>>;

type CompletableSender<T, E> = oneshot::Sender<CompletableResult<T, E>>;

type CompletableReceiver<T, E> = oneshot::Receiver<CompletableResult<T, E>>;

#[derive(Debug)]
pub enum FutureError<E> {
    Raw(E),
    Inner(&'static str),
}

/// CompletableFuture 完成操作的错误
#[derive(Debug)]
pub enum CompleteError {
    /// future 已经完成，无法再次完成
    AlreadyCompleted,
    /// 发送者被 dropped，无法发送结果
    SenderDropped,
    /// 锁中毒
    LockPoisoned,
}

pub struct CompletableFuture<T, E> {
    inner: Arc<Inner<T, E>>,
}

impl<T, E> Clone for CompletableFuture<T, E> {
    fn clone(&self) -> Self {
        CompletableFuture {
            inner: self.inner.clone(),
        }
    }
}

struct Inner<T, E> {
    sender: Mutex<Option<CompletableSender<T, E>>>,
    receiver: Mutex<Option<CompletableReceiver<T, E>>>,
    completed: AtomicBool,
}

impl<T, E> Default for CompletableFuture<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, E> CompletableFuture<T, E> {
    pub fn new() -> Self {
        let (tx, rx) = oneshot::channel();

        Self {
            inner: Arc::new(Inner {
                sender: Mutex::new(Some(tx)),
                receiver: Mutex::new(Some(rx)),
                completed: AtomicBool::new(false),
            }),
        }
    }

    pub fn complete(&self, value: T) -> Result<(), CompleteError> {
        if self.inner.completed.swap(true, Ordering::SeqCst) {
            return Err(CompleteError::AlreadyCompleted);
        }

        let mut sender_guard = match self.inner.sender.lock() {
            Ok(guard) => guard,
            Err(_) => return Err(CompleteError::LockPoisoned),
        };

        if let Some(sender) = sender_guard.take() {
            sender
                .send(Ok(value))
                .map_err(|_| CompleteError::SenderDropped)
        } else {
            Err(CompleteError::SenderDropped)
        }
    }

    pub fn complete_exceptionally(&self, error: E) -> Result<(), CompleteError> {
        if self.inner.completed.swap(true, Ordering::SeqCst) {
            return Err(CompleteError::AlreadyCompleted);
        }

        let mut sender_guard = match self.inner.sender.lock() {
            Ok(guard) => guard,
            Err(_) => return Err(CompleteError::LockPoisoned),
        };

        if let Some(sender) = sender_guard.take() {
            sender
                .send(Err(FutureError::Raw(error)))
                .map_err(|_| CompleteError::SenderDropped)
        } else {
            Err(CompleteError::SenderDropped)
        }
    }

    pub fn is_done(&self) -> bool {
        self.inner.completed.load(Ordering::Relaxed)
    }

    /// Chain a transformation on success.
    ///
    /// Returns a new `CompletableFuture<U, E>` that will be completed with `f(value)`
    /// when this future completes successfully. If this future fails, the new
    /// future also fails with the same error.
    ///
    /// Corresponds to Java: `CompletableFuture.thenApply(Function<T, U>)`
    #[allow(dead_code)]
    pub fn then_apply<U, F>(self, f: F) -> CompletableFuture<U, E>
    where
        T: Send + 'static,
        E: Send + 'static,
        U: Send + 'static,
        F: FnOnce(T) -> U + Send + 'static,
    {
        let next = CompletableFuture::<U, E>::new();
        let next_clone = next.clone();

        tokio::spawn(async move {
            match self.await {
                Ok(value) => {
                    let _ = next_clone.complete(f(value));
                }
                Err(FutureError::Raw(error)) => {
                    let _ = next_clone.complete_exceptionally(error);
                }
                Err(FutureError::Inner(_)) => {
                    // Internal error (lock poisoned / channel dropped) — drop next, leaving it pending
                }
            }
        });

        next
    }

    /// Chain an error recovery handler.
    ///
    /// Returns a new `CompletableFuture<T, E>` that will be completed with `f(error)`
    /// when this future fails. If this future succeeds, the new future also
    /// succeeds with the same value.
    ///
    /// Corresponds to Java: `CompletableFuture.exceptionally(Function<Throwable, T>)`
    #[allow(dead_code)]
    pub fn exceptionally<F>(self, f: F) -> CompletableFuture<T, E>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: FnOnce(E) -> T + Send + 'static,
    {
        let next = CompletableFuture::<T, E>::new();
        let next_clone = next.clone();

        tokio::spawn(async move {
            match self.await {
                Ok(value) => {
                    let _ = next_clone.complete(value);
                }
                Err(FutureError::Raw(error)) => {
                    let _ = next_clone.complete(f(error));
                }
                Err(FutureError::Inner(_)) => {
                    // Internal error — drop next, leaving it pending
                }
            }
        });

        next
    }

    /// Compose this future with a handler that processes both success and failure.
    ///
    /// Spawns a task that awaits this future and calls the handler with the result.
    /// The handler receives a `Result<T, E>` directly — no need to return a new future.
    ///
    /// This matches Java's `RequestFuture.compose(RequestFutureAdapter)` pattern where
    /// the adapter's `onSuccess`/`onFailure` processes the response and updates state.
    ///
    /// # Example
    /// ```ignore
    /// client.send_to_coordinator(request_bytes)
    ///     .then_compose(HeartbeatResponseHandler::new(generation));
    /// ```
    ///
    /// Corresponds to Java: `RequestFuture.compose(RequestFutureAdapter)`
    pub fn then_compose<H>(self, mut handler: H)
    where
        T: Send + 'static,
        E: Send + 'static,
        H: FutureHandler<T, E>,
    {
        tokio::spawn(async move {
            match self.await {
                Ok(value) => handler.on_success(value),
                Err(FutureError::Raw(error)) => handler.on_failure(error),
                Err(FutureError::Inner(_)) => {
                    // Internal error — nothing to do
                }
            }
        });
    }
}

/// Handler trait for processing both success and failure cases in a future chain.
///
/// This trait is used with `CompletableFuture::then_compose()` to handle both
/// successful completion and errors in a single handler.
///
/// # Type Parameters
/// - `T`: The success type (response from the previous future)
/// - `E`: The error type
pub trait FutureHandler<T, E>: Send + 'static {
    /// Called when the previous future completes successfully.
    fn on_success(&mut self, response: T);

    /// Called when the previous future completes with an error.
    fn on_failure(&mut self, error: E);
}

impl<T, E> fmt::Debug for CompletableFuture<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompletableFuture")
            .field("is_done", &self.is_done())
            .finish()
    }
}

impl<T, E> Future for CompletableFuture<T, E> {
    type Output = Result<T, FutureError<E>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut receiver_guard = match self.inner.receiver.lock() {
            Ok(guard) => guard,
            Err(_) => {
                // 锁中毒
                return Poll::Ready(Err(FutureError::Inner("Future lock poisoned")));
            }
        };

        if let Some(ref mut rx) = *receiver_guard {
            // 轮询接收端
            match Pin::new(rx).poll(cx) {
                Poll::Ready(Ok(result)) => {
                    // 清空 receiver
                    receiver_guard.take();
                    Poll::Ready(result)
                }
                Poll::Ready(Err(_)) => {
                    // channel 关闭
                    receiver_guard.take();
                    Poll::Ready(Err(FutureError::Inner("Future was cancelled or dropped")))
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            // receiver 已经被取走，说明已经完成
            // 这种情况不应该发生，因为完成时我们已经清空了 receiver
            Poll::Ready(Err(FutureError::Inner("Future already consumed")))
        }
    }
}
