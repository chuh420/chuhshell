use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
    closed: bool,
}

pub struct Sender<T>(Arc<Mutex<Slot<T>>>);

pub struct Receiver<T>(Arc<Mutex<Slot<T>>>);

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let slot = Arc::new(Mutex::new(Slot {
        value: None,
        waker: None,
        closed: false,
    }));
    (Sender(slot.clone()), Receiver(slot))
}

impl<T> Sender<T> {
    pub fn send(self, value: T) {
        if let Ok(mut slot) = self.0.lock() {
            slot.value = Some(value);
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.0.lock() {
            slot.closed = true;
            if let Some(waker) = slot.waker.take() {
                waker.wake();
            }
        }
    }
}

impl<T> Future for Receiver<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let Ok(mut slot) = self.0.lock() else {
            return Poll::Ready(None);
        };
        if let Some(value) = slot.value.take() {
            return Poll::Ready(Some(value));
        }
        if slot.closed {
            return Poll::Ready(None);
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}
