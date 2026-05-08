use std::{cell::RefCell, mem::MaybeUninit, rc::Rc, task::Poll};

use slab::Slab;

use crate::vppinfra::tw_timer::StartTimerError;

#[derive(Copy, Clone, Debug)]
struct TimerHandle(usize);

#[derive(Debug)]
struct TimerState {
    is_ready: bool,
    wheel_handle: Option<crate::vppinfra::tw_timer::TimerHandle>,
}

#[derive(Debug)]
pub struct TimerWheel {
    /// Timer entry pool
    entries: Slab<TimerState>,
    previous_ticks: u64,
    wheel: crate::vppinfra::tw_timer::TimerWheel<TimerHandle, 3, 2048>,
}

impl TimerWheel {
    pub fn init(uninit_self: &mut MaybeUninit<Self>) -> &mut Self {
        // SAFETY: `uninit_self` points to valid writable memory and we initialize each field before returning.
        unsafe {
            let ptr = uninit_self.as_mut_ptr();
            std::ptr::addr_of_mut!((*ptr).entries).write(Slab::new());
            std::ptr::addr_of_mut!((*ptr).previous_ticks).write(0);
            crate::vppinfra::tw_timer::TimerWheel::init(
                &mut *(std::ptr::addr_of_mut!((*ptr).wheel)
                    as *mut MaybeUninit<
                        crate::vppinfra::tw_timer::TimerWheel<TimerHandle, 3, 2048>,
                    >),
            );

            uninit_self.assume_init_mut()
        }
    }

    fn start_timer(&mut self, expire_time: u64) -> TimerHandle {
        let entry = TimerState {
            is_ready: false,
            wheel_handle: None,
        };
        let slab_entry = self.entries.vacant_entry();
        let handle = TimerHandle(slab_entry.key());
        let entry = slab_entry.insert(entry);

        match self.wheel.start_timer_absolute(expire_time, handle) {
            Ok(wheel_handle) => {
                entry.wheel_handle = Some(wheel_handle);
            }
            Err(StartTimerError::Expired(_)) => {
                entry.is_ready = true;
            }
        };

        handle
    }

    fn stop_timer(&mut self, handle: TimerHandle) {
        let entry = self.entries.remove(handle.0);
        if let Some(wheel_handle) = entry.wheel_handle {
            self.wheel.stop_timer(wheel_handle);
        }
    }

    fn state(&self, handle: TimerHandle) -> Option<&TimerState> {
        self.entries.get(handle.0)
    }

    pub fn expire_timers(&mut self, now: u64) {
        let ticks = now.saturating_sub(self.previous_ticks);
        let timer_handles = self.wheel.expire_timers(ticks);
        self.previous_ticks = now;
        for handle in timer_handles {
            let timer_entry = self.entries.get_mut(handle.0);
            if let Some(timer_entry) = timer_entry {
                // Note: no waker.wake_by_ref() here since this is designed to be called from the
                // same event loop as the future is polled from
                timer_entry.is_ready = true;
                timer_entry.wheel_handle = None;
            } else {
                // TODO: this shouldn't happen, so panic?
            }
        }
    }

    /// Get the duration until the next expiration in ticks
    ///
    /// Returns the duration until the next expiration given in ticks since the last time [`Self::expire_timers()`] was called.
    ///
    /// If there are no timers pending, [`None`] is returned.
    pub fn next_expiration(&self) -> Option<u64> {
        self.wheel
            .next_expiration()
            .map(|ticks| ticks - self.previous_ticks)
    }
}

#[derive(Debug)]
pub struct Timer {
    handle: TimerHandle,
    wheel: Rc<RefCell<Box<TimerWheel>>>,
}

impl Timer {
    pub(crate) fn new(wheel: Rc<RefCell<Box<TimerWheel>>>, expire_time: u64) -> Self {
        let handle = wheel.borrow_mut().start_timer(expire_time);
        Self { handle, wheel }
    }

    pub(crate) fn is_ready(&self) -> bool {
        match self.wheel.borrow().state(self.handle) {
            Some(state) => state.is_ready,
            None => panic!(
                "Handle {:?} unexpectedly invalid in timer wheel {:?}",
                self.handle, self.wheel
            ),
        }
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.is_ready() {
            Poll::Ready(())
        } else {
            // Note: no storing of cx.waker() for later use since this is designed to be called
            // from the same event loop as expires timers
            Poll::Pending
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.wheel.borrow_mut().stop_timer(self.handle);
    }
}
