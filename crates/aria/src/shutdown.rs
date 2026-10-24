use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Condvar, Mutex},
    task::{Context, Poll, Waker},
};

use log::info;

pub struct SharedEvent {
    lock: Mutex<bool>,
    cvar: Condvar,
}

impl SharedEvent {
    // const fn allows this to be called in a static context
    pub const fn new() -> Self {
        Self {
            lock: Mutex::new(false),
            cvar: Condvar::new(),
        }
    }

    /// Blocks the current thread until notify() is called.
    pub fn wait(&self) {
        let mut ready = self.lock.lock().unwrap();
        while !*ready {
            ready = self.cvar.wait(ready).unwrap();
        }
    }

    /// Wakes up ALL waiting threads.
    pub fn notify(&self) {
        let mut ready = self.lock.lock().unwrap();
        *ready = true;
        self.cvar.notify_all();
    }
}

// Global static initialization
static SHUTDOWN_WAKER: SharedEvent = SharedEvent::new();

#[allow(dead_code)]
pub fn block_until_shutdown() {
    SHUTDOWN_WAKER.wait();
}

static SHUTDOWN_SENT: AtomicBool = AtomicBool::new(false);
static WAKERS: Mutex<Vec<Waker>> = Mutex::new(Vec::new());

pub fn is_shutdown_sent() -> bool {
    SHUTDOWN_SENT.load(Ordering::Relaxed)
}

#[derive(Clone)]
pub struct ShutdownSignal;

impl Future for ShutdownSignal {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if SHUTDOWN_SENT.load(Ordering::Relaxed) {
            Poll::Ready(())
        } else {
            let waker = cx.waker().clone();
            let mut wakers = WAKERS.lock().unwrap();
            if !wakers.iter().any(|w| w.will_wake(&waker)) {
                wakers.push(waker);
            }
            Poll::Pending
        }
    }
}

pub fn shutdown() {
    SHUTDOWN_SENT.store(true, Ordering::Relaxed);
    let mut wakers = WAKERS.lock().unwrap();
    for waker in wakers.drain(..) {
        waker.wake();
    }
    drop(wakers);
    SHUTDOWN_WAKER.notify();
    info!("Shutdown signal sent, waiting for shutdown");
}
