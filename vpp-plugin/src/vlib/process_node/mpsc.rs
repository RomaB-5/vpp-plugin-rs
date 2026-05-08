//! Multi-producer, single-consumer channel for sending data into asynchronous tasks

use futures::task::AtomicWaker;
use std::{
    cell::Cell,
    pin::Pin,
    sync::{Arc, mpsc::TryRecvError},
    task::{Context, Poll},
};

/// A multiple-producer, single-consumer channel for async process events.
///
/// This channel is unbounded.
pub struct Sender<T> {
    inner: std::sync::mpsc::Sender<T>,
    shared_state: Arc<MpscSharedState>,
}

/// Receiver for a single consumer side of an MPSC channel.
pub struct Receiver<T> {
    inner: std::sync::mpsc::Receiver<T>,
    shared_state: Arc<MpscSharedState>,
    _not_sync: std::marker::PhantomData<Cell<()>>,
}

struct MpscSharedState {
    rx_waker: AtomicWaker,
}

// SAFETY: `MpscSender` uses a mutex/condvar-protected buffer and only stores `T` when `T: Send`, so it is safe to send across threads.
unsafe impl<T: Send> Send for Sender<T> {}

// SAFETY: `MpscSender` can be shared across threads because internal synchronization ensures correctness.
unsafe impl<T: Send> Sync for Sender<T> {}

// Receiver is not Sync by design (single consumer), but it may be moved between threads safely.
// SAFETY: `MpscReceiver` guarantees single-consumer semantics while preserving `T: Send`.
unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Sender<T> {
    /// Send a value into the channel.
    pub fn send(&self, value: T) -> Result<(), T> {
        self.inner
            .send(value)
            .map_err(|std::sync::mpsc::SendError(value)| value)?;

        // Notify the process/executor to resume via VPP event mechanism.
        self.shared_state.rx_waker.wake();

        Ok(())
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared_state: self.shared_state.clone(),
        }
    }
}

impl<T> Receiver<T> {
    /// Try to receive a value without blocking.
    pub fn try_recv(&self) -> Option<T> {
        self.inner.try_recv().ok()
    }

    /// Returns a future that waits for the next value or channel close.
    pub fn recv(&self) -> ReceiverFuture<'_, T> {
        ReceiverFuture { receiver: self }
    }

    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        match self.inner.try_recv() {
            Ok(value) => Poll::Ready(Some(value)),
            Err(TryRecvError::Disconnected) => Poll::Ready(None),
            Err(TryRecvError::Empty) => {
                self.shared_state.rx_waker.register(cx.waker());

                Poll::Pending
            }
        }
    }
}

/// Future for receiver for a single consumer side of an MPSC channel.
pub struct ReceiverFuture<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<'a, T> Future for ReceiverFuture<'a, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.receiver.poll_recv(cx)
    }
}

/// Create an unbounded multi-producer single-consumer channel.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (sender, receiver) = std::sync::mpsc::channel();

    let shared_state = Arc::new(MpscSharedState {
        rx_waker: AtomicWaker::new(),
    });

    (
        Sender {
            inner: sender,
            shared_state: shared_state.clone(),
        },
        Receiver {
            inner: receiver,
            shared_state,
            _not_sync: std::marker::PhantomData,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::channel;
    use futures_task::noop_waker;

    use std::{
        pin::Pin,
        task::{Context, Poll},
        thread,
    };

    #[test]
    fn mpsc_channel_basic_send_recv() {
        let (tx, rx) = channel();
        assert!(tx.send(10).is_ok());
        assert!(tx.send(20).is_ok());

        assert_eq!(rx.try_recv(), Some(10));
        assert_eq!(rx.try_recv(), Some(20));
        assert_eq!(rx.try_recv(), None);

        drop(tx);
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn mpsc_channel_multithreaded_producers() {
        let (tx, rx) = channel();
        let tx1 = tx.clone();
        let tx2 = tx.clone();

        let t1 = thread::spawn(move || {
            for i in 0..4 {
                assert!(tx1.send(i).is_ok());
            }
        });
        let t2 = thread::spawn(move || {
            for i in 4..8 {
                assert!(tx2.send(i).is_ok());
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        let mut seen = [false; 8];
        for _ in 0..8 {
            let value = rx.try_recv().expect("channel should return value");
            assert!(value < 8);
            seen[value] = true;
        }

        assert!(seen.iter().all(|&v| v));
    }

    #[test]
    fn mpsc_channel_async_poll_wakes() {
        let (tx, rx) = channel();
        let mut rx_future = rx.recv();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(
            Pin::new(&mut rx_future).poll(&mut cx),
            Poll::Pending
        ));

        assert!(tx.send(42).is_ok());

        match Pin::new(&mut rx_future).poll(&mut cx) {
            Poll::Ready(Some(v)) => assert_eq!(v, 42),
            other => panic!("expected ready after send, got {:?}", other),
        }

        drop(tx);
        let mut rx_future2 = rx.recv();
        assert!(matches!(
            Pin::new(&mut rx_future2).poll(&mut cx),
            Poll::Ready(None)
        ));
    }
}
