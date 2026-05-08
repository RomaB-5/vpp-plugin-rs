//! Timer wheels
//!
//! # Design Considerations
//!
//! This module implements a hierarchical timer wheel data structure optimised for efficient
//! expiration of large numbers of timers in O(1) amortised time. Key design decisions:
//!
//! - Hierarchical levels: Multiple levels with decreasing resolution enable handling both
//!   near-term (high precision) and far-future (lower precision) timers efficiently without
//!   requiring a massive single-level array.
//!
//! - Doubly-linked lists per slot: Each slot in each level contains a doubly-linked list of
//!   timers, enabling O(1) stop (and removal) of arbitrary timers.
//!
//! - Head entries: Every slot is pre-initialised with a head entry, eliminating the need for
//!   special-case list manipulation.
//!
//! - Cascading: When a timer expires in a coarser level, it is re-added to finer levels (if
//!   available), until it is determined to the caller as having expired. This still allows
//!   efficient processing of many timers far in the future whilst still allowing them to have
//!   the fine expiration resolution.
//!
//! - Generic context: The context type `T` is associated with each timer, allowing arbitrary
//!   application data to be attached without additional allocation.

use std::mem::MaybeUninit;

use super::Vec;
use slab::Slab;

/// Add timer error
#[non_exhaustive]
#[derive(Debug)]
pub enum StartTimerError<T> {
    /// The timer is already expired
    Expired(T),
}

const EMPTY_INDEX: u32 = u32::MAX;

/// Timer handle
///
/// Used for stopping a running timer.
#[derive(Debug)]
// Copying a timer handle would make it easier to double-cancel timers, causing undesirable behaviour
#[allow(missing_copy_implementations)]
pub struct TimerHandle(u32);

/// Timer entry data
#[derive(Debug, Copy, Clone)]
enum TimerEntryData<T> {
    /// Value for the head of a list
    Head,
    /// An actual timer entry
    Timer {
        /// The absolute expiration time in ticks
        expire_time: u64,
        /// Application context for the timer entry
        context: T,
    },
}

/// A timer entry
#[derive(Debug, Copy, Clone)]
struct TimerEntry<T> {
    /// Index of previous entry
    next: u32,
    /// Index of next entry
    previous: u32,
    /// Timer entry value
    data: TimerEntryData<T>,
}

/// A list of timer entries for a slot
#[derive(Debug)]
struct EntryList {
    /// Index to head of list
    head: u32,
}

impl Default for EntryList {
    fn default() -> Self {
        Self { head: EMPTY_INDEX }
    }
}

/// A level in the timer wheel
#[derive(Debug)]
struct Level<const NUM_SLOTS: usize> {
    /// The slots, each containing a list of timers
    slots: [EntryList; NUM_SLOTS],
    /// Current slot index
    position: usize,
}

/// Hierarchical timer wheel
///
/// This is a multi-level timer data structure where each level has lower resolution (is coarser)
/// than the previous. It efficiently handles expiration of many timers by batching them into
/// slots.
///
/// # Parameters
///
/// - `T`: The context type associated with each timer
/// - `NUM_LEVELS`: The number of hierarchical levels (each adds a factor of NUM_SLOTS in range)
/// - `NUM_SLOTS`: The number of slots per level
///
/// # Resolution and Range
///
/// - **Range per level**: Level 0 covers `[0, NUM_SLOTS]` ticks. Level 1 covers
///   `[0, NUM_SLOTS²]` ticks at coarser granularity. In general, level `L` covers
///   `[0, NUM_SLOTS^(L+1)]` ticks.
///
/// - **Total range**: With `NUM_LEVELS` levels, the wheel can represent timers up to
///   approximately `NUM_SLOTS^NUM_LEVELS` ticks in the future (though timers beyond
///   `NUM_SLOTS^NUM_LEVELS` are still supported, see below).
///
/// # Handling Timers Far in the Future
///
/// When a timer is scheduled with an expiration time far beyond what the wheel's levels can
/// directly represent (delta ≥ `NUM_SLOTS^NUM_LEVELS`), it is placed in the **last slot of
/// the highest level**. As time progresses and cascading occurs, such timers are progressively
/// re-inserted at appropriate finer-resolution slots, eventually cascading down to level 0 for
/// precise expiration. This approach trades initial placement overhead for the ability to handle
/// arbitrarily distant future timers without requiring additional data structures.
///
/// # Compatibility with VPP C Timer Wheels
///
/// The functionality and interface to the timer wheel is intended to be similar to VPP C Timer
/// Wheels, but isn't binary-compatible with VPP C Timer Wheels, since this would constrain the
/// implementation too much and it's not envisaged to be likely that timer wheels would be used
/// across API boundaries.
#[derive(Debug)]
pub struct TimerWheel<T, const NUM_LEVELS: usize, const NUM_SLOTS: usize> {
    /// Timer pool
    timers: Slab<TimerEntry<T>>,
    /// The levels of the wheel
    levels: [Level<NUM_SLOTS>; NUM_LEVELS],
    /// Current time in ticks
    current_time: u64,
}

// Manually implement Default rather than deriving it to avoid the `T: Default` constraint that
// would otherwise be added
impl<T, const NUM_LEVELS: usize, const NUM_SLOTS: usize> Default
    for TimerWheel<T, NUM_LEVELS, NUM_SLOTS>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const NUM_LEVELS: usize, const NUM_SLOTS: usize> TimerWheel<T, NUM_LEVELS, NUM_SLOTS> {
    /// Create a new timer wheel
    pub fn new() -> Self {
        let mut uninit_self = MaybeUninit::uninit();
        Self::init(&mut uninit_self);

        // SAFETY: `Self::init` fully initialised `uninit_self` before call.
        unsafe { uninit_self.assume_init() }
    }

    /// Initialise a [`TimerWheel`] in uninitialized memory.
    ///
    /// This can be used to avoid excessive stage usage when `uninit_self` is located in
    /// allocated memory, such as from a `Vec`, `Box` or `Rc`.
    pub fn init(uninit_self: &mut MaybeUninit<Self>) -> &mut Self {
        // SAFETY: `uninit_self` points to valid writable memory and we initialise each field before returning.
        let init_self = unsafe {
            let ptr = uninit_self.as_mut_ptr();
            std::ptr::addr_of_mut!((*ptr).timers)
                .write(Slab::with_capacity(NUM_LEVELS * NUM_SLOTS));
            for level in 0..NUM_LEVELS {
                let level_ptr =
                    (std::ptr::addr_of_mut!((*ptr).levels) as *mut Level<NUM_SLOTS>).add(level);
                for slot in 0..NUM_SLOTS {
                    let slot_ptr =
                        (std::ptr::addr_of_mut!((*level_ptr).slots) as *mut EntryList).add(slot);
                    slot_ptr.write(Default::default());
                }
                std::ptr::addr_of_mut!((*level_ptr).position).write(0);
            }
            std::ptr::addr_of_mut!((*ptr).current_time).write(0);
            uninit_self.assume_init_mut()
        };

        // Populate all slots with a head entry - this allows us to remove a timer later without
        // needing to keep track of which level and slot the timer is added to and without having
        // to brute-force it.
        for level in 0..NUM_LEVELS {
            for slot in 0..NUM_SLOTS {
                init_self.levels[level].slots[slot].head = init_self.timers.insert(TimerEntry {
                    next: EMPTY_INDEX,
                    previous: EMPTY_INDEX,
                    data: TimerEntryData::Head,
                }) as u32;
            }
        }

        init_self
    }

    /// Start a timer which expires after the given interval
    ///
    /// The timer is one-shot, not periodic.
    ///
    /// `interval` is the interval past the last time timers were expired after which this timer
    /// should expire. `context` is a context to associate with the timer
    ///
    /// This has a time complexity of O(1).
    pub fn start_timer(&mut self, interval: u64, context: T) -> TimerHandle {
        let expire_time = self.current_time.saturating_add(interval);

        let handle = TimerHandle(self.timers.insert(TimerEntry {
            next: EMPTY_INDEX,
            previous: EMPTY_INDEX,
            data: TimerEntryData::Timer {
                expire_time,
                context,
            },
        }) as u32);

        self.start_timer_unchecked(handle.0);

        handle
    }

    /// Start a timer which expires at an absolute time
    ///
    /// `expire_time` is the absolute time after which this timer should expire. `context` is a
    /// context to associate with the timer
    ///
    /// This has a time complexity of O(1).
    pub fn start_timer_absolute(
        &mut self,
        expire_time: u64,
        context: T,
    ) -> Result<TimerHandle, StartTimerError<T>> {
        if expire_time <= self.current_time {
            return Err(StartTimerError::Expired(context));
        }

        let handle = TimerHandle(self.timers.insert(TimerEntry {
            next: EMPTY_INDEX,
            previous: EMPTY_INDEX,
            data: TimerEntryData::Timer {
                expire_time,
                context,
            },
        }) as u32);

        self.start_timer_unchecked(handle.0);

        Ok(handle)
    }

    /// Start a timer, not checking if it has already expired
    fn start_timer_unchecked(&mut self, index: u32) {
        let timer = &self.timers[index as usize];
        let delta = match &timer.data {
            TimerEntryData::Timer { expire_time, .. } => *expire_time - self.current_time,
            TimerEntryData::Head => unreachable!(),
        };
        // In case it's too far in the future, put in the last level's last slot
        let mut level = NUM_LEVELS - 1;
        let mut slot = (self.levels[level].position + NUM_SLOTS - 1) % NUM_SLOTS;

        for l in 0..NUM_LEVELS {
            if delta < (NUM_SLOTS as u64).pow((l + 1) as u32) {
                level = l;
                slot = (self.levels[l].position
                    + (delta / (NUM_SLOTS as u64).pow(l as u32)) as usize)
                    .saturating_sub(1)
                    % NUM_SLOTS;
                break;
            }
        }

        let head = &mut self.timers[self.levels[level].slots[slot].head as usize];
        let old_head_next = std::mem::replace(&mut head.next, index);
        self.timers[index as usize].next = old_head_next;
        self.timers[index as usize].previous = self.levels[level].slots[slot].head;
        if old_head_next != EMPTY_INDEX {
            self.timers[old_head_next as usize].previous = index;
        }
    }

    /// Stop a running timer
    ///
    /// This has a time complexity of O(1).
    pub fn stop_timer(&mut self, handle: TimerHandle) -> Option<T> {
        let timer = self.timers.get(handle.0 as usize)?;
        // Refuse to remove a special head entry - note that this shouldn't happen since head
        // entries are allocated at init time (so reuse isn't a factor) and otherwise a TimerHandle
        // cannot be constructed pointing to anything other than an a timer entry.
        if matches!(timer.data, TimerEntryData::Head) {
            return None;
        }
        let timer_next = timer.next;
        let timer_previous = timer.previous;

        // Unlink the timer from the doubly-linked list
        // timer_previous is never EMPTY_INDEX here because the head of the lists are
        // pre-populated with entries
        self.timers[timer_previous as usize].next = timer_next;
        if timer_next != EMPTY_INDEX {
            self.timers[timer_next as usize].previous = timer_previous;
        }

        let timer = self.timers.remove(handle.0 as usize);
        match timer.data {
            TimerEntryData::Timer { context, .. } => Some(context),
            // We should have returned early with the check for this above
            TimerEntryData::Head => unreachable!(),
        }
    }

    /// Process a level's current slot, expire timers, and cascade if needed
    fn process_level(&mut self, level: usize) -> Vec<T> {
        let mut expired_contexts = Vec::new();

        if level >= NUM_LEVELS {
            return expired_contexts;
        }

        // Extract timers from current slot
        let slot = self.levels[level].position;
        let head = &mut self.timers[self.levels[level].slots[slot].head as usize];
        let mut timer_index = std::mem::replace(&mut head.next, EMPTY_INDEX);

        // Process each timer
        while timer_index != EMPTY_INDEX {
            let timer = &self.timers[timer_index as usize];
            let next_timer_index = timer.next;

            match &timer.data {
                TimerEntryData::Timer { expire_time, .. } => {
                    // Check that the timer has actually expired. There are two cases where it
                    // might not have:
                    // 1. Timer started in level > 0, with a lower resolution than for
                    //    level == 0; and
                    // 2. Timer started in time not represented by the number of levels & slots,
                    //    i.e. too far in the future.
                    // In both of these cases, if the timer hasn't yet expired we want to
                    // re-start the timer at the appropriate level & slot.
                    if *expire_time <= self.current_time {
                        let timer = self.timers.remove(timer_index as usize);
                        match timer.data {
                            TimerEntryData::Timer { context, .. } => {
                                expired_contexts.push(context);
                            }
                            TimerEntryData::Head => unreachable!(),
                        }
                    } else {
                        self.start_timer_unchecked(timer_index);
                    }
                }
                TimerEntryData::Head => unreachable!(),
            }
            timer_index = next_timer_index;
        }

        // Advance position
        self.levels[level].position = (self.levels[level].position + 1) % NUM_SLOTS;

        // Cascade to next level if this level wrapped
        if self.levels[level].position == 0 {
            let cascaded_expired_contexts = self.process_level(level + 1);
            expired_contexts.extend(cascaded_expired_contexts);
        }

        expired_contexts
    }

    /// Advance time by one tick, expiring timers
    fn tick(&mut self) -> Vec<T> {
        self.current_time += 1;
        self.process_level(0)
    }

    /// Advance time by multiple ticks, expiring timers
    pub fn expire_timers(&mut self, ticks: u64) -> Vec<T> {
        let mut expired_contexts = Vec::new();

        for _ in 0..ticks {
            let tick_expired_contexts = self.tick();
            expired_contexts.extend(tick_expired_contexts);
        }

        expired_contexts
    }

    /// Get the next expiration time in ticks, or `None` if no unexpired, started timers
    ///
    /// This has worst-case time complexity O(n) where n is `NUM_LEVELS * NUM_SLOTS`.
    pub fn next_expiration(&self) -> Option<u64> {
        for level in 0..NUM_LEVELS {
            let start_slot = self.levels[level].position;
            for i in 0..NUM_SLOTS {
                let slot = (start_slot + i) % NUM_SLOTS;
                let head = &self.timers[self.levels[level].slots[slot].head as usize];
                if head.next != EMPTY_INDEX {
                    let mut timer_index = head.next;
                    let mut min_expire_time: Option<u64> = None;
                    while timer_index != EMPTY_INDEX {
                        let timer = &self.timers[timer_index as usize];
                        timer_index = timer.next;
                        match &timer.data {
                            TimerEntryData::Timer { expire_time, .. } => {
                                if let Some(prev_min_expire_time) = min_expire_time {
                                    min_expire_time = Some(prev_min_expire_time.min(*expire_time));
                                } else {
                                    min_expire_time = Some(*expire_time);
                                }
                            }
                            // We should have skipped over the head already
                            TimerEntryData::Head => unreachable!(),
                        }
                    }
                    return min_expire_time;
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::vppinfra::clib_mem_init;

    use super::*;

    #[test]
    fn test_immediate_timer() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        let e = wheel.start_timer_absolute(0, 1).expect_err("add timer");
        assert!(
            matches!(e, StartTimerError::Expired(1)),
            "{:?} != AddTimerError::Expired",
            e
        );
    }

    #[test]
    fn test_future_timer() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        wheel.start_timer_absolute(5, 1).expect("add timer");
        let contexts = wheel.expire_timers(4);
        assert_eq!(contexts, []);
        let contexts = wheel.expire_timers(1);
        assert_eq!(contexts, [1]);
    }

    #[test]
    fn test_multiple_timers() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        wheel.start_timer_absolute(1, 1).expect("add timer");
        wheel.start_timer(2, 2);
        wheel.start_timer_absolute(3, 3).expect("add timer");
        let contexts = wheel.expire_timers(1);
        assert_eq!(contexts, [1]);
        let contexts = wheel.expire_timers(2);
        assert_eq!(contexts, [2, 3]);
    }

    #[test]
    fn test_level_1() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        // Add a timer that will be placed in level 1 (delta = 257 > 256)
        wheel.start_timer(257, 1);
        // Advance 256 ticks: this should trigger cascade and re-insert the timer
        let contexts = wheel.expire_timers(256);
        assert_eq!(contexts, []);
        // Advance one more tick to expire the re-inserted timer
        let contexts = wheel.expire_timers(1);
        assert_eq!(contexts, [1]);
    }

    #[test]
    fn test_next_expiration() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        assert_eq!(wheel.next_expiration(), None);
        wheel.start_timer(5, 1);
        assert_eq!(wheel.next_expiration(), Some(5));
        wheel.start_timer_absolute(3, 2).expect("add timer");
        assert_eq!(wheel.next_expiration(), Some(3));
        wheel.start_timer(10, 3);
        assert_eq!(wheel.next_expiration(), Some(3));
        let contexts = wheel.expire_timers(3); // expire at 3
        assert_eq!(contexts, [2]);
        assert_eq!(wheel.next_expiration(), Some(5));
    }

    #[test]
    fn test_stop_timers() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 4, 256> = Default::default();
        let timer1 = wheel.start_timer(5, 1);
        let timer2 = wheel.start_timer_absolute(3, 2).expect("add timer");
        let timer3 = wheel.start_timer_absolute(5, 3).expect("add timer");
        // Stop a timer that isn't the head of the list
        assert_eq!(wheel.stop_timer(timer1), Some(1));
        let contexts = wheel.expire_timers(3);
        assert_eq!(contexts, [2]);
        // Stop an already-expired timer
        assert_eq!(wheel.stop_timer(timer2), None);
        // Stop a timer that is head of the list
        assert_eq!(wheel.stop_timer(timer3), Some(3));
        // Advance 2 more ticks and don't expect any of the timers to fire
        let contexts = wheel.expire_timers(2);
        assert_eq!(contexts, []);
    }

    #[test]
    fn test_timer_far_in_future() {
        clib_mem_init();

        let mut wheel: TimerWheel<u8, 2, 4> = Default::default();
        // The maximum ticks represented by the slots is 4^2, so 17 is beyond that - we still
        // expect it to expire at the correct time.
        let timer1 = wheel.start_timer_absolute(17, 1).expect("add timer");
        let contexts = wheel.expire_timers(16);
        assert_eq!(contexts, []);
        assert_eq!(wheel.stop_timer(timer1), Some(1));
        // Add two, both far in the future, and expect them to expire at the correct time as well
        // as the next expiration time to take the lower of the two.
        wheel.start_timer(18, 2);
        wheel.start_timer(17, 1);
        assert_eq!(wheel.next_expiration(), Some(16 + 17));
        let contexts = wheel.expire_timers(17);
        assert_eq!(contexts, [1]);
    }
}
