//! Core infrastructure for VPP process nodes
//!
//! This module provides the [`ProcessNode`] trait and core infrastructure
//! for running async/await coroutines within VPP process nodes.

use futures_task::{ArcWake, waker_ref};
use pin_project_lite::pin_project;

use crate::{
    bindings::{
        _vlib_node_registration, async_context, vl_api_force_rpc_call_main_thread,
        vlib_helper_get_global_main, vlib_helper_process_node_loop,
        vlib_helper_remove_node_from_registrations, vlib_main_t, vlib_node_registration_t,
        vlib_node_runtime_t, vlib_process_signal_event_mt_args_t,
        vlib_process_signal_event_mt_helper,
    },
    vlib::{
        MainRef, NodeRuntimeRef,
        node::{ErrorCounters, NextNodes},
        process_node::tw_timer::{Timer, TimerWheel},
    },
};
use std::{
    cell::{RefCell, UnsafeCell},
    ffi::c_void,
    fmt,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

pub use futures_task::LocalFutureObj;

// This could be set to 1000 / VLIB_TW_TICKS_PER_SECOND (defined in VPP code), but that is 10μs, which is smaller than
// the epoll_wait granularity of 1ms (which is called by vlib_file_poll in the main event loop), and so wouldn't be
// able to be achieved reliably (even with no other Unix processes in the system pre-empting the VPP main thread).
//
// So instead, this is just set to 1ms which is the minimum theoretically reliably achievable for process nodes.
const TICK_INTERVAL_PER_MS: u64 = 1;
const TICK_INTERVAL_S: f64 = TICK_INTERVAL_PER_MS as f64 / 1000.0;

/// Trait for defining a VPP process (async) node
pub trait ProcessNode {
    /// Type defining the next nodes of this node
    ///
    /// Typically an enum using the [`vpp_plugin_macros::NextNodes`] derive macro.
    type NextNodes: NextNodes;

    /// Type defining the runtime data of this node
    ///
    /// This data is per-node instance and per-thread.
    // Send + Copy due to:
    //     if (vec_len (n->runtime_data) > 0)
    //       clib_memcpy (rt->runtime_data, n->runtime_data,
    //                    vec_len (n->runtime_data));
    //     else
    //       clib_memset (rt->runtime_data, 0, VLIB_NODE_RUNTIME_DATA_SIZE);
    type RuntimeData: Send + Copy;
    /// Type defining the error counters of this node
    ///
    /// Typically an enum using the [`vpp_plugin_macros::ErrorCounters`] derive macro.
    type Errors: ErrorCounters;

    /// The main async coroutine for this process node
    #[must_use = "Futures do nothing unless awaited"]
    fn function(
        &self,
        vm: &mut MainRef,
        node: &mut NodeRuntimeRef<Self>,
    ) -> impl Future<Output = ()>;
}

/// Registration information for a VPP process node
///
/// Used for registering and unregistering process nodes with VPP.
///
/// This is typically created automatically using the [`vpp_plugin_macros::vlib_process_node`] macro.
pub struct ProcessNodeRegistration<N: ProcessNode, const N_NEXT_NODES: usize> {
    registration: UnsafeCell<_vlib_node_registration<[*mut std::os::raw::c_char; N_NEXT_NODES]>>,
    _marker: std::marker::PhantomData<N>,
}

impl<N: ProcessNode, const N_NEXT_NODES: usize> ProcessNodeRegistration<N, N_NEXT_NODES> {
    /// Creates a new `ProcessNodeRegistration` from the given registration data
    pub const fn new(
        registration: _vlib_node_registration<[*mut std::os::raw::c_char; N_NEXT_NODES]>,
    ) -> Self {
        Self {
            registration: UnsafeCell::new(registration),
            _marker: ::std::marker::PhantomData,
        }
    }

    /// Registers the node with VPP
    ///
    /// # Safety
    ///
    /// - Must be called only once for this node registration.
    /// - Must be called from a constructor function that is invoked before VPP initialises.
    /// - The following pointers in the registration data must be valid:
    ///   - `name` (must be a valid, nul-terminated string)
    ///   - `function` (must point to a valid node function)
    ///   - `error_descriptions` (must point to an array of `n_errors` valid `vlib_error_desc_t` entries)
    ///   - `next_nodes` (each entry must be a valid nul-terminated string and length must be at least `n_next_nodes`)
    /// - Other pointers in the registration data must be either valid or null as appropriate.
    /// - `vector_size`, `scalar_size`, and `aux_size` must match the sizes of the corresponding types in `N`.
    /// - `n_errors` must match the discriminants in N::Errors
    /// - `n_next_nodes` must match the discriminants in N::NextNodes
    pub unsafe fn register(&'static self) {
        // SAFETY: The safety requirements are documented in the function's safety comment.
        unsafe {
            let vgm = vlib_helper_get_global_main();
            let reg = self.registration.get();
            (*reg).next_registration = (*vgm).node_registrations;
            (*vgm).node_registrations = reg as *mut vlib_node_registration_t;
        }
    }

    /// Unregisters the node from VPP
    ///
    /// # Safety
    ///
    /// - Must be called only once for this node registration.
    /// - Must be called from a destructor function that is invoked after VPP uninitialises.
    /// - The node must have been previously registered with VPP using [`Self::register`].
    pub unsafe fn unregister(&self) {
        // SAFETY: The safety requirements are documented in the function's safety comment.
        unsafe {
            let vgm = vlib_helper_get_global_main();
            vlib_helper_remove_node_from_registrations(
                vgm,
                self.registration.get() as *mut vlib_node_registration_t,
            );
        }
    }

    /// Creates a `&mut NodeRuntimeRef` directly from a pointer
    ///
    /// This is a convenience method that calls [`NodeRuntimeRef::from_ptr_mut`], for code that
    /// has an instance of `NodeRegistration`, but doesn't know the name of the type for the node.
    /// As such, `self` isn't used, it's just taken so that the generic types are known.
    ///
    /// # Safety
    ///
    /// - The same preconditions as [`NodeRuntimeRef::from_ptr_mut`] apply.
    pub unsafe fn node_runtime_from_ptr<'a>(
        &self,
        ptr: *mut vlib_node_runtime_t,
    ) -> &'a mut NodeRuntimeRef<N> {
        // SAFETY: The safety requirements are documented in the function's safety comment.
        unsafe { NodeRuntimeRef::from_ptr_mut(ptr) }
    }
}

// SAFETY: there is nothing in vlib_node_registration that is tied to a specific thread or that
// mutates global state, so it's safe to send between threads.
unsafe impl<N: ProcessNode, const N_NEXT_NODES: usize> Send
    for ProcessNodeRegistration<N, N_NEXT_NODES>
{
}
// SAFETY: NodeRegistration doesn't allow any modification after creation (and vpp doesn't
// modify it afterwards either), so it's safe to access from multiple threads. The only exception
// to this is the register/unregister methods, but it's the duty of the caller
// to ensure they are called at times when no other threads have a reference to the object.
unsafe impl<N: ProcessNode, const N_NEXT_NODES: usize> Sync
    for ProcessNodeRegistration<N, N_NEXT_NODES>
{
}

/// Async context shared with other objects that need scheduling
pub(crate) struct ProcessAsyncContextShared {
    timer_wheel: Rc<RefCell<Box<TimerWheel>>>,
    waker: Arc<ProcessAsyncContextWaker>,
    start_time: Instant,
}

impl ProcessAsyncContextShared {
    fn new(node_index: u32) -> Self {
        // Initialise on the heap to avoid excessive stack usage
        let mut timer_wheel = Box::new_uninit();
        TimerWheel::init(&mut timer_wheel);
        // SAFETY: timer_wheel is initialized by TimerWheel::init above
        let timer_wheel = unsafe { timer_wheel.assume_init() };
        Self {
            timer_wheel: Rc::new(RefCell::new(timer_wheel)),
            waker: Arc::new(ProcessAsyncContextWaker { node_index }),
            start_time: Instant::now(),
        }
    }

    /// Convert an instant in time to number of ticks since the start time
    ///
    /// If the instant in time is before the start time, it will be classed as 0 ticks. Times
    /// greater than [`u64::MAX`] ticks into the future are treated as just [`u64::MAX`] ticks.
    fn instant_to_ticks(&self, t: Instant) -> u64 {
        let duration = t.saturating_duration_since(self.start_time);
        duration
            .as_millis()
            .div_ceil(TICK_INTERVAL_PER_MS.into())
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

pin_project! {
    /// Async context for running a future within a VPP process node.
    ///
    /// This struct holds the state needed to poll a async future from the
    /// VPP process node loop, including a timer wheel for async operations.
    pub struct ProcessAsyncContext<'a> {
        main_ref: *mut vlib_main_t,
        #[pin]
        future: Option<LocalFutureObj<'a, ()>>,
        shared: Rc<ProcessAsyncContextShared>,
    }
}

impl<'a> ProcessAsyncContext<'a> {
    /// Create a new async context for the given future.
    pub fn new<N>(
        vm: &'a mut MainRef,
        node: &NodeRuntimeRef<N>,
        future: LocalFutureObj<'a, ()>,
    ) -> Self {
        Self {
            main_ref: vm.as_ptr(),
            future: Some(future),
            shared: Rc::new(ProcessAsyncContextShared::new(node.node_index())),
        }
    }

    /// Run the async context in the VPP process node loop.
    ///
    /// This method never returns as it enters VPP's process node loop.
    pub fn run(mut self) -> ! {
        // SAFETY: This enters the VPP process node loop which is the intended
        // usage of this function. Since `Self::new` enforces that the MainRef must live as long
        // as self then the underlying pointer must also last that long.
        unsafe {
            vlib_helper_process_node_loop(
                self.main_ref,
                &mut self as *mut Self as *mut async_context,
            )
        }
    }
}

struct ProcessAsyncContextWaker {
    node_index: u32,
}

impl ArcWake for ProcessAsyncContextWaker {
    fn wake_by_ref(arc_self: &std::sync::Arc<Self>) {
        let mut args = vlib_process_signal_event_mt_args_t {
            node_index: arc_self.node_index as u64,
            type_opaque: 0,
            data: 0,
        };
        // This is conservative since we don't know whether or not we're on the main thread
        // SAFETY: this is safe to call on any thread since VPP takes a spinlock around the
        // critical section and the arguments match what vlib_process_signal_event_mt_helper
        // expects.
        unsafe {
            vl_api_force_rpc_call_main_thread(
                vlib_process_signal_event_mt_helper as *mut c_void,
                std::ptr::addr_of_mut!(args) as *mut u8,
                std::mem::size_of_val(&args) as u32,
            )
        };
    }
}

/// Poll the async coroutine once.
///
/// This function is called by VPP to advance the async future forward.
/// It should be called repeatedly until the future completes.
///
/// # Safety
///
/// - `context` must be a valid, non-null pointer to a live `ProcessAsyncContext`.
/// - The caller must ensure that the context remains valid for the duration of this call.
/// - This function must only be called from a single thread at a time.
#[unsafe(no_mangle)]
unsafe extern "C" fn vpp_plugin_rs_poll_async_coroutine(context: *mut ProcessAsyncContext) {
    // SAFETY: `context` is guaranteed non-null and points to a valid `ProcessAsyncContext`.
    let mut ctx = unsafe { Pin::new_unchecked(&mut *context) };

    let ticks_since_start = ctx.shared.instant_to_ticks(Instant::now());
    ctx.shared
        .timer_wheel
        .borrow_mut()
        .expire_timers(ticks_since_start);

    let ctx_project = ctx.as_mut().project();
    if let Some(fut) = ctx_project.future.as_pin_mut() {
        ASYNC_CONTEXT.with(|tls_ctx| {
            tls_ctx.replace(Some(ctx_project.shared.clone()));
        });
        let waker = waker_ref(&ctx_project.shared.waker);
        let mut executor_context = Context::from_waker(&waker);
        if matches!(fut.poll(&mut executor_context), Poll::Ready(_)) {
            // > Once a future has finished, clients should not poll it again.
            // [https://doc.rust-lang.org/std/future/trait.Future.html]
            ctx.project().future.set(None);
        }
        ASYNC_CONTEXT.with(|tls_ctx| {
            tls_ctx.replace(None);
        });
    }
}

/// Get the amount of time to wait before the next timer expires.
///
/// If there is no next timer, then [`f64::MAX`] will be returned.
///
/// # Safety
///
/// - `context` must be a pointer to a live `ProcessAsyncContext`.
/// - The pointer must not be null and must remain valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn vpp_plugin_rs_next_timer_duration(context: *mut ProcessAsyncContext) -> f64 {
    // SAFETY: `context` is validated by the caller contract to be non-null and valid.
    let ctx = unsafe { &*context };
    let next_expiration = ctx.shared.timer_wheel.borrow().next_expiration();
    next_expiration
        .map(|ticks| ticks as f64 * TICK_INTERVAL_S)
        .unwrap_or(f64::MAX)
}

thread_local! {
    /// Async context for process nodes
    ///
    /// This is updated before and after suspending a VPP process node and is only valid when
    /// polling the `ProcessAsyncContext` future.
    static ASYNC_CONTEXT: RefCell<Option<Rc<ProcessAsyncContextShared>>> = const { RefCell::new(None) };
}

/// Execute a closure that receives a reference to the current process node async context
///
/// The result of the function is that of the closure.
///
/// # Panics
///
/// If not called from a vpp-plugin-rs process node.
pub(crate) fn with_current_async_context<F, R>(f: F) -> R
where
    F: FnOnce(&Rc<ProcessAsyncContextShared>) -> R,
{
    ASYNC_CONTEXT.with(|ctx| {
        f(ctx.borrow().as_ref().expect(
            "There is no async context present - must be called from a vpp-plugin-rs process node",
        ))
    })
}

pin_project! {
    /// Future returned by [`sleep()`]
    #[project(!Unpin)]
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct Sleep {
        // The link between the `Sleep` instance and the timer that drives it.
        #[pin]
        entry: Timer,
    }
}

impl Sleep {
    pub(crate) fn new_timeout(deadline: Instant, ctx: &Rc<ProcessAsyncContextShared>) -> Self {
        let deadline_ticks = ctx.instant_to_ticks(deadline);
        let entry = Timer::new(ctx.timer_wheel.clone(), deadline_ticks);
        Self { entry }
    }

    /// Returns `true` if `Sleep` has elapsed.
    ///
    /// A `Sleep` instance is elapsed when the requested duration has elapsed.
    pub fn is_elapsed(&self) -> bool {
        self.entry.is_ready()
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        self.project().entry.poll(cx)
    }
}

/// Waits until `duration` has elapsed.
///
/// An asynchronous analog to [`std::thread::sleep`].
pub fn sleep(duration: Duration) -> Sleep {
    let deadline = Instant::now().checked_add(duration).unwrap_or_else(|| {
        // Roughly 30 years from now.
        // Standard library does not provide a way to obtain max `Instant`
        // or convert specific date in the future to instant.
        // 1000 years overflows on macOS, 100 years overflows on FreeBSD.
        Instant::now() + Duration::from_secs(86400 * 365 * 30)
    });
    with_current_async_context(|ctx| Sleep::new_timeout(deadline, ctx))
}

/// Errors returned by `Timeout`.
///
/// This error is returned when a timeout expires before the function was able
/// to finish.
#[derive(Debug, PartialEq, Eq)]
// It may become more complicated in the future
#[allow(missing_copy_implementations)]
pub struct Elapsed(());

impl fmt::Display for Elapsed {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        "deadline has elapsed".fmt(fmt)
    }
}

impl std::error::Error for Elapsed {}

impl From<Elapsed> for std::io::Error {
    fn from(_err: Elapsed) -> std::io::Error {
        std::io::ErrorKind::TimedOut.into()
    }
}

/// Apply a timeout to the given `future`
///
/// If `future` completes before `duration` has elapsed, then the completed value is returned.
/// Otherwise, an [`Elapsed`] error is returned and the future is cancelled.
pub fn timeout<F>(duration: Duration, future: F) -> Timeout<F::IntoFuture>
where
    F: IntoFuture,
{
    let delay = sleep(duration);
    Timeout::new_with_delay(future.into_future(), delay)
}

pin_project! {
    /// Future returned by [`timeout`](timeout).
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    #[derive(Debug)]
    pub struct Timeout<T> {
        #[pin]
        value: T,
        #[pin]
        delay: Sleep,
    }
}

impl<T> Timeout<T> {
    pub(crate) fn new_with_delay(value: T, delay: Sleep) -> Timeout<T> {
        Timeout { value, delay }
    }

    /// Gets a reference to the underlying value in this timeout.
    pub fn get_ref(&self) -> &T {
        &self.value
    }

    /// Gets a mutable reference to the underlying value in this timeout.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Consumes this timeout, returning the underlying value.
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> Future for Timeout<T>
where
    T: Future,
{
    type Output = Result<T::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let me = self.project();

        // First try polling the value future
        if let Poll::Ready(v) = me.value.poll(cx) {
            return Poll::Ready(Ok(v));
        }

        // Then try polling the delay future
        match me.delay.poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(Elapsed(()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Elapsed, sleep};

    use std::time::Duration;

    #[test]
    #[should_panic(
        expected = "There is no async context present - must be called from a vpp-plugin-rs process node"
    )]
    fn sleep_outside_process_node_panics() {
        std::mem::drop(sleep(Duration::from_secs(1)));
    }

    #[test]
    fn elapsed_to_std_error() {
        let e: std::io::Error = Elapsed(()).into();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
    }
}
