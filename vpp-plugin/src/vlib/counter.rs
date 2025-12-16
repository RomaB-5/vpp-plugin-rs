//! Per-thread, per-object counters
//!
//! Optimised counters suitable for use in node functions.

use std::{
    ffi::CString,
    mem::ManuallyDrop,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    bindings::{
        vlib_combined_counter_main_t, vlib_counter_t, vlib_free_combined_counter,
        vlib_free_simple_counter, vlib_helper_zero_combined_counter,
        vlib_helper_zero_simple_counter, vlib_simple_counter_main_t,
        vlib_validate_combined_counter, vlib_validate_simple_counter,
    },
    vlib::{BarrierHeldMainRef, MainRef},
    vppinfra,
};

/// Per-thread, per-object simple counters
pub struct SimpleCounter {
    counter: vlib_simple_counter_main_t,
}

impl SimpleCounter {
    /// Create a new `SimpleCounter`
    pub fn new(name: &str, stat_segment_name: &str) -> Self {
        let name_cstr = CString::new(name).unwrap();
        let stats_segment_name_cstr = CString::new(stat_segment_name).unwrap();
        Self {
            counter: vlib_simple_counter_main_t {
                counters: std::ptr::null_mut(),
                name: name_cstr.into_raw(),
                stat_segment_name: stats_segment_name_cstr.into_raw(),
                stats_entry_index: 0,
            },
        }
    }

    /// Allocate the given index for use, zeroing it in the process
    ///
    /// This is similar to the VPP C API `vlib_validate_simple_counter`.
    ///
    /// Note that no checks are performed to ensure that the index isn't already in use, but it
    /// doesn't result in unsoundness if the same value is allocated for an already-allocated and
    /// not dropped index.
    pub fn allocate_index(&self, _vm: &BarrierHeldMainRef, index: u32) -> SimpleCounterIndex<'_> {
        // SAFETY: self.counter is correctly initialised, the fact that the barrier is held
        // (which can only be done from the main thread) guaranteed that no other threads are
        // using the counters vector when it is potentially grown in size, and the zero without
        // using atomics is safe for the same reason.
        unsafe {
            vlib_validate_simple_counter(std::ptr::addr_of!(self.counter).cast_mut(), index);
            vlib_helper_zero_simple_counter(std::ptr::addr_of!(self.counter).cast_mut(), index);
            SimpleCounterIndex::from_parts(self, index)
        }
    }
}

impl Drop for SimpleCounter {
    fn drop(&mut self) {
        // SAFETY: `self.counter` is correctly initialised and both `self.counter.name` and
        // `self.counter.stat_segment_name` are pointers created from `CString`s
        unsafe {
            let _name_cstr = CString::from_raw(self.counter.name);
            let _stat_segment_name_cstr = CString::from_raw(self.counter.stat_segment_name);
            vlib_free_simple_counter(std::ptr::addr_of_mut!(self.counter));
        }
    }
}

// SAFETY: it's safe to drop the counter from other threads provided that there are no outstanding
// references, which is guaranteed by safety preconditions.
unsafe impl Send for SimpleCounter {}
// SAFETY: anything that retrieves state that other threads may write to or modifies state is
// guarded by suitable preconditions, such as SimpleCounter::allocate_index taking a
// BarrierHeldMainRef reference ensuring it can only be called from the main thread with no
// worker threads using it concurrently.
unsafe impl Sync for SimpleCounter {}

/// An allocated index for a [`SimpleCounter`]
pub struct SimpleCounterIndex<'counter> {
    counter: &'counter SimpleCounter,
    index: u32,
}

impl<'counter> SimpleCounterIndex<'counter> {
    /// Decomposes a simple counter index into its component parts
    pub fn into_parts(self) -> (&'counter SimpleCounter, u32) {
        let me = ManuallyDrop::new(self);
        (me.counter, me.index)
    }

    /// Creates a `SimpleCounterIndex` from component parts
    ///
    /// # Safety
    ///
    /// - Must only be done from VPP worker threads or the main thread
    /// - `index` must be a valid (previously allocated) index for the counter
    pub unsafe fn from_parts(counter: &'counter SimpleCounter, index: u32) -> Self {
        Self { counter, index }
    }

    fn counter_ptr(&self, vm: &MainRef) -> *mut u64 {
        let thread_index = vm.thread_index();

        // SAFETY: worker threads cannot be added after VPP has initialised and creation
        // preconditions ensure `self.index` is valid
        unsafe {
            let this_thread_counters = *self.counter.counter.counters.add(thread_index as usize);

            this_thread_counters.add(self.index as usize)
        }
    }

    /// Increment this thread's per-thread counter
    pub fn increment(&self, vm: &MainRef, count: u64) {
        // SAFETY: no concurrent writers, since the counts are per-thread and safety conditions of zero() are upheld
        unsafe {
            let counter = self.counter_ptr(vm);
            let new_count = *counter + count;
            AtomicU64::from_ptr(counter).store(new_count, Ordering::Relaxed);
        }
    }

    /// Zero all threads' per-thread counters
    ///
    /// # Safety
    ///
    /// - There must be no concurrent writers to this index. For example, worker threads
    ///   without the barrier held.
    pub unsafe fn zero(&self) {
        // SAFETY: no concurrent writers
        unsafe {
            vlib_helper_zero_simple_counter(
                std::ptr::addr_of!(self.counter.counter).cast_mut(),
                self.index,
            );
        }
    }

    /// Get the total count, summing for all threads
    // Note: &MainRef taken to ensure this is only called from a VPP main/worker thread
    pub fn get(&self, _vm: &MainRef) -> u64 {
        // SAFETY: counter is a valid vector
        // Note: vlib_get_simple_counter doesn't use an atomic load and so isn't sound with
        // concurrent writers.
        unsafe {
            let per_thread_counters =
                vppinfra::vec::VecRef::from_raw(self.counter.counter.counters);
            per_thread_counters
                .iter()
                .fold(0u64, |sum, counter_by_index| {
                    let counter = AtomicU64::from_ptr(counter_by_index.add(self.index as usize));
                    sum + counter.load(Ordering::Relaxed)
                })
        }
    }
}

// SAFETY: methods can be called on the SimpleCounterIndex provided they are done so only on VPP
// main/worker threads, which are ensured by non-unsafe methods taking a `&MainRef`.
unsafe impl Send for SimpleCounterIndex<'_> {}
// SAFETY: methods that mutate the counter index state do so via a combination of atomics and
// per-thread state.
unsafe impl Sync for SimpleCounterIndex<'_> {}

/// Combined count of packets and bytes
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CombinedCount {
    /// The number of packets
    pub packets: u64,
    /// The number of bytes
    pub bytes: u64,
}

/// Per-thread, per-object combined packets and bytes counters
pub struct CombinedCounter {
    counter: vlib_combined_counter_main_t,
}

impl CombinedCounter {
    /// Create a new `CombinedCounter`
    pub fn new(name: &str, stat_segment_name: &str) -> Self {
        let name_cstr = CString::new(name).unwrap();
        let stats_segment_name_cstr = CString::new(stat_segment_name).unwrap();
        Self {
            counter: vlib_combined_counter_main_t {
                counters: std::ptr::null_mut(),
                name: name_cstr.into_raw(),
                stat_segment_name: stats_segment_name_cstr.into_raw(),
                stats_entry_index: 0,
            },
        }
    }

    /// Allocate the given index for use, zeroing it in the process
    ///
    /// This is similar to the VPP C API `vlib_validate_combined_counter`.
    ///
    /// Note that no checks are performed to ensure that the index isn't already in use, but it
    /// doesn't result in unsoundness if the same value is allocated for an already-allocated and
    /// not dropped index.
    pub fn allocate_index(&self, _vm: &BarrierHeldMainRef, index: u32) -> CombinedCounterIndex<'_> {
        // SAFETY: self.counter is correctly initialised, the fact that the barrier is held
        // (which can only be done from the main thread) guaranteed that no other threads are
        // using the counters vector when it is potentially grown in size, and the zero without
        // using atomics is safe for the same reason.
        unsafe {
            vlib_validate_combined_counter(std::ptr::addr_of!(self.counter).cast_mut(), index);
            vlib_helper_zero_combined_counter(std::ptr::addr_of!(self.counter).cast_mut(), index);
            CombinedCounterIndex::from_parts(self, index)
        }
    }
}

impl Drop for CombinedCounter {
    fn drop(&mut self) {
        // SAFETY: `self.counter` is correctly initialised and both `self.counter.name` and
        // `self.counter.stat_segment_name` are pointers created from `CString`s
        unsafe {
            let _name_cstr = CString::from_raw(self.counter.name);
            let _stat_segment_name_cstr = CString::from_raw(self.counter.stat_segment_name);
            vlib_free_combined_counter(std::ptr::addr_of_mut!(self.counter));
        }
    }
}

// SAFETY: it's safe to drop the counter from other threads provided that there are no outstanding
// references, which is guaranteed by safety preconditions.
unsafe impl Send for CombinedCounter {}
// SAFETY: anything that retrieves state that other threads may write to or modifies state is
// guarded by suitable preconditions, such as CombinedCounter::allocate_index taking a
// BarrierHeldMainRef reference ensuring it can only be called from the main thread with no
// worker threads using it concurrently.
unsafe impl Sync for CombinedCounter {}

/// An allocated index for a [`CombinedCounter`]
pub struct CombinedCounterIndex<'counter> {
    counter: &'counter CombinedCounter,
    index: u32,
}

impl<'counter> CombinedCounterIndex<'counter> {
    /// Decomposes a combined counter index into its component parts
    pub fn into_parts(self) -> (&'counter CombinedCounter, u32) {
        let me = ManuallyDrop::new(self);
        (me.counter, me.index)
    }

    /// Creates a `CombinedCounterIndex` from component parts
    ///
    /// # Safety
    ///
    /// - Must only be done from VPP worker threads or the main thread
    /// - `index` must be a valid (previously allocated) index for the counter
    pub unsafe fn from_parts(counter: &'counter CombinedCounter, index: u32) -> Self {
        Self { counter, index }
    }

    fn counter_ptr(&self, vm: &MainRef) -> *mut vlib_counter_t {
        let thread_index = vm.thread_index();

        // SAFETY: worker threads cannot be added after VPP has initialised and creation
        // preconditions ensure `self.index` is valid
        unsafe {
            let this_thread_counters = *self.counter.counter.counters.add(thread_index as usize);

            this_thread_counters.add(self.index as usize)
        }
    }

    /// Increment this thread's per-thread counter
    pub fn increment(&self, vm: &MainRef, packets: u64, bytes: u64) {
        // SAFETY: no concurrent writers, since the counts are per-thread and safety conditions of zero() are upheld
        unsafe {
            let counter = self.counter_ptr(vm);
            let new_packets = (*counter).packets + packets;
            AtomicU64::from_ptr(std::ptr::addr_of_mut!((*counter).packets))
                .store(new_packets, Ordering::Relaxed);
            let new_bytes = (*counter).bytes + bytes;
            AtomicU64::from_ptr(std::ptr::addr_of_mut!((*counter).bytes))
                .store(new_bytes, Ordering::Relaxed);
        }
    }

    /// Zero all threads' per-thread counters
    ///
    /// # Safety
    ///
    /// - There must be no concurrent writers to this index. For example, worker threads
    ///   without the barrier held.
    pub unsafe fn zero(&self) {
        // SAFETY: no concurrent writers
        unsafe {
            vlib_helper_zero_combined_counter(
                std::ptr::addr_of!(self.counter.counter).cast_mut(),
                self.index,
            );
        }
    }

    /// Get the total count, summing for all threads
    // Note: &MainRef taken to ensure this is only called from a VPP main/worker thread
    pub fn get(&self, _vm: &MainRef) -> CombinedCount {
        // SAFETY: counter is a valid vector
        // Note: vlib_get_combined_counter doesn't use an atomic load and so isn't sound with
        // concurrent writers.
        unsafe {
            let per_thread_counters =
                vppinfra::vec::VecRef::from_raw(self.counter.counter.counters);
            per_thread_counters.iter().fold(
                CombinedCount {
                    packets: 0,
                    bytes: 0,
                },
                |sum, counter_by_index| {
                    let packets = AtomicU64::from_ptr(std::ptr::addr_of_mut!(
                        (*counter_by_index.add(self.index as usize)).packets
                    ));
                    let bytes = AtomicU64::from_ptr(std::ptr::addr_of_mut!(
                        (*counter_by_index.add(self.index as usize)).bytes
                    ));
                    CombinedCount {
                        packets: sum.packets + packets.load(Ordering::Relaxed),
                        bytes: sum.bytes + bytes.load(Ordering::Relaxed),
                    }
                },
            )
        }
    }
}

// SAFETY: methods can be called on the CombinedCounterIndex provided they are done so only on VPP
// main/worker threads, which are ensured by non-unsafe methods taking a `&MainRef`.
unsafe impl Send for CombinedCounterIndex<'_> {}
// SAFETY: methods that mutate the counter index state do so via a combination of atomics and
// per-thread state.
unsafe impl Sync for CombinedCounterIndex<'_> {}
