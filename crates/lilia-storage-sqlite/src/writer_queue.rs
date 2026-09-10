use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use lilia_core::{ErrorCode, LiliaError, Result};

/// Bounded FIFO admission; capacity counts waiting writers, not the active writer.
#[derive(Debug)]
pub(crate) struct WriterQueue {
    capacity: usize,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    active: bool,
    next: u64,
    waiting: VecDeque<u64>,
}

#[derive(Debug)]
pub(crate) struct Permit<'a>(&'a WriterQueue);

impl WriterQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }
    }

    pub(crate) fn acquire(&self, deadline: Option<Instant>) -> Result<Permit<'_>> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if expired(deadline) {
            return Err(timeout());
        }
        if !state.active && state.waiting.is_empty() {
            state.active = true;
            return Ok(Permit(self));
        }
        if state.waiting.len() >= self.capacity {
            return Err(LiliaError::new(
                ErrorCode::Busy,
                "writer queue is saturated",
                true,
            ));
        }
        let ticket = state.next;
        state.next = state.next.wrapping_add(1);
        state.waiting.push_back(ticket);
        self.changed.notify_all();
        loop {
            // Check before granting the writer, even when a permit just became free.
            if expired(deadline) {
                state.waiting.retain(|entry| *entry != ticket);
                self.changed.notify_all();
                return Err(timeout());
            }
            if !state.active && state.waiting.front() == Some(&ticket) {
                state.waiting.pop_front();
                state.active = true;
                return Ok(Permit(self));
            }
            state = if let Some(deadline) = deadline {
                self.changed
                    .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| poisoned())?
                    .0
            } else {
                self.changed.wait(state).map_err(|_| poisoned())?
            };
        }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        // Release on success, SQL error, or unwind. A poisoned admission mutex
        // remains fail-closed for later requests.
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = false;
        self.0.changed.notify_all();
    }
}

fn expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

fn timeout() -> LiliaError {
    LiliaError::new(ErrorCode::Timeout, "request expired in writer queue", true)
}

fn poisoned() -> LiliaError {
    LiliaError::new(ErrorCode::Storage, "writer queue lock poisoned", false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn wait_for(queue: &WriterQueue, count: usize) {
        let until = Instant::now() + Duration::from_secs(5);
        let mut state = queue.state.lock().unwrap();
        while state.waiting.len() != count {
            assert!(Instant::now() < until, "writer was not enqueued");
            state = queue
                .changed
                .wait_timeout(state, until.saturating_duration_since(Instant::now()))
                .unwrap()
                .0;
        }
    }

    #[test]
    fn fifo_capacity_and_release() {
        let queue = WriterQueue::new(2);
        let active = queue.acquire(None).unwrap();
        let (send, receive) = mpsc::channel();
        std::thread::scope(|scope| {
            for index in 0..2 {
                let send = send.clone();
                let queue = &queue;
                scope.spawn(move || {
                    let _permit = queue.acquire(None).unwrap();
                    send.send(index).unwrap();
                });
                wait_for(queue, index + 1);
            }
            let error = queue.acquire(None).unwrap_err();
            assert_eq!(error.code, ErrorCode::Busy);
            assert!(error.retryable);
            drop(active);
            assert_eq!(receive.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
            assert_eq!(receive.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        });
        assert!(queue.acquire(None).is_ok());
    }

    #[test]
    fn expired_waiter_is_removed_without_blocking_followers() {
        let queue = WriterQueue::new(2);
        let active = queue.acquire(None).unwrap();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                queue
                    .acquire(Some(Instant::now() + Duration::from_millis(50)))
                    .unwrap_err()
            });
            let error = waiter.join().unwrap();
            assert_eq!(error.code, ErrorCode::Timeout);
            assert!(error.retryable);
            wait_for(&queue, 0);
            drop(active);
            assert!(queue.acquire(None).is_ok());
        });
    }

    #[test]
    fn zero_capacity_allows_only_the_active_writer() {
        let queue = WriterQueue::new(0);
        let active = queue.acquire(None).unwrap();
        assert_eq!(queue.acquire(None).unwrap_err().code, ErrorCode::Busy);
        drop(active);
        assert_eq!(
            queue.acquire(Some(Instant::now())).unwrap_err().code,
            ErrorCode::Timeout
        );
        assert!(queue.acquire(None).is_ok());
    }
}
