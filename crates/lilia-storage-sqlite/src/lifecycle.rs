use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use lilia_core::{ErrorCode, LiliaError, Result};

#[derive(Debug, Default)]
pub(crate) struct Lifecycle {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    active: usize,
    closing: bool,
    result: Option<Result<()>>,
}

#[derive(Debug)]
pub(crate) struct Operation<'a>(&'a Lifecycle);

impl Lifecycle {
    pub(crate) fn enter(&self) -> Result<Operation<'_>> {
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if state.closing {
            return Err(LiliaError::new(
                ErrorCode::Closed,
                "database is closing or closed",
                false,
            ));
        }
        state.active += 1;
        Ok(Operation(self))
    }

    pub(crate) fn close(
        self: &Arc<Self>,
        timeout: Duration,
        cleanup: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            LiliaError::new(ErrorCode::InvalidInput, "close timeout is too large", false)
        })?;
        let mut state = self.state.lock().map_err(|_| poisoned())?;
        if !state.closing {
            state.closing = true;
            let shared = Arc::clone(self);
            if let Err(error) = std::thread::Builder::new()
                .name("lilia-close".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut state = shared.state.lock().map_err(|_| poisoned())?;
                        while state.active != 0 {
                            state = shared.changed.wait(state).map_err(|_| poisoned())?;
                        }
                        drop(state);
                        cleanup()
                    }))
                    .unwrap_or_else(|_| Err(poisoned()));
                    let mut state = shared
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.result = Some(result);
                    shared.changed.notify_all();
                })
            {
                state.closing = false;
                return Err(LiliaError::new(ErrorCode::Io, error.to_string(), false));
            }
        }
        loop {
            if let Some(result) = &state.result {
                return result.clone();
            }
            if Instant::now() >= deadline {
                return Err(LiliaError::new(
                    ErrorCode::Timeout,
                    "close is still draining; accepted operations are not cancelled",
                    true,
                ));
            }
            state = self
                .changed
                .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| poisoned())?
                .0;
        }
    }
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

fn poisoned() -> LiliaError {
    LiliaError::new(
        ErrorCode::Storage,
        "database lifecycle worker failed",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_does_not_wait_for_slow_cleanup_or_restart_it() {
        let lifecycle = Arc::new(Lifecycle::default());
        let (started, receive_started) = std::sync::mpsc::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        assert_eq!(
            lifecycle
                .close(Duration::ZERO, move || {
                    started.send(()).unwrap();
                    wait_release.recv().unwrap();
                    Ok(())
                })
                .unwrap_err()
                .code,
            ErrorCode::Timeout
        );
        receive_started
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(lifecycle.enter().unwrap_err().code, ErrorCode::Closed);
        assert_eq!(
            lifecycle
                .close(Duration::from_millis(10), || panic!("cleanup restarted"))
                .unwrap_err()
                .code,
            ErrorCode::Timeout
        );
        release.send(()).unwrap();
        lifecycle
            .close(Duration::from_secs(5), || panic!("cleanup restarted"))
            .unwrap();
    }
}
