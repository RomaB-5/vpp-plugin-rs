//! Process node module for async/await support in VPP plugins.
//!
//! This module provides the infrastructure for running async Rust futures
//! within VPP's process node framework, enabling plugin authors to write
//! async code that integrates with VPP's event loop.
//!
//! # Overview
//!
//! VPP process nodes are cooperative multi-tasking threads that run within the
//! vlib framework. They are ideal for control-plane tasks that need to wait for
//! events or timers without consuming CPU while idle. This module enables writing
//! such process nodes using Rust's async/await syntax, combining the safety and
//! expressiveness of modern Rust with VPP's event-driven architecture.
//!
//! ## Key Features
//!
//! - **Single async coroutine per process node**: Each process node runs exactly one async function (future)
//! - **MPSC channels for events**: External code can send events to the future via multiple-producer single-consumer channels
//! - **Timer integration**: Async sleep and timeout functions built on timer wheel
//! - **VPP event loop integration**: Suspends using `vlib_process_wait_for_event_or_clock()`
//!
//! ## Basic Usage
//!
//! ```
//! use vpp_plugin::{
//!     vlib_process_node,
//!     vlib::{ProcessNode, MainRef, process_node::{channel, Sender, Receiver}},
//!     vlib::node::{NodeRuntimeRef, NextNodes, ErrorCounters},
//!     ErrorCounters,
//!     NextNodes,
//! };
//! use std::sync::{Mutex, LazyLock};
//!
//! #[derive(NextNodes)]
//! enum MyProcessNextNode {
//!     #[next_node = "error-drop"]
//!     Drop,
//! }
//!
//! #[derive(ErrorCounters)]
//! enum MyProcessErrors {
//!     #[error_counter(description = "Example error", severity = ERROR)]
//!     Example,
//! }
//!
//! #[derive(Debug)]
//! enum MyProcessMessage {
//!     Enable,
//!     Disable,
//! }
//!
//! // The mutex is in case the channel is created (for the sender) in another thread
//! type GetOnceProcessMessageReceiver = Mutex<Option<Receiver<MyProcessMessage>>>;
//!
//! // Create a static instance of the process node
//! static MY_PROCESS_NODE: MyProcessNode = MyProcessNode::new();
//!
//! // Register the process node with VPP
//! #[vlib_process_node(
//!     name = "my-process",
//!     instance = MY_PROCESS_NODE,
//! )]
//! struct MyProcessNode {
//!     channel: LazyLock<(Sender<MyProcessMessage>, GetOnceProcessMessageReceiver)>,
//! }
//!
//! impl MyProcessNode {
//!    const fn new() -> Self {
//!         Self {
//!             channel: LazyLock::new(|| {
//!                 let (sender, receiver) = channel();
//!                 (sender, Mutex::new(Some(receiver)))
//!             }),
//!         }
//!     }
//!
//!     fn channel_sender(&self) -> Sender<MyProcessMessage> {
//!         self.channel.0.clone()
//!     }
//!
//!     fn channel_receiver(&self) -> Receiver<MyProcessMessage> {
//!         self.channel.1.lock().unwrap().take().unwrap()
//!     }
//! }
//!
//! // Implement the ProcessNode trait with an async function
//! impl ProcessNode for MyProcessNode {
//!     type NextNodes = MyProcessNextNode;
//!     type RuntimeData = ();
//!     type Errors = MyProcessErrors;
//!
//!     async fn function(&self, vm: &mut MainRef, node: &mut NodeRuntimeRef<Self>) {
//!         // Get the event channel receiver
//!         let rx = self.channel_receiver();
//!
//!         loop {
//!             // Wait for events from the channel
//!             match rx.recv().await {
//!                 Some(event) => {
//!                     println!("Received event: {:?}", event);
//!                 }
//!                 None => break,
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # Comparison with VPP C Process Nodes
//!
//! Writing process nodes in C requires manual management of the event loop:
//!
//! ## C Implementation Pattern
//!
//! ```c
//! // Typical C process node structure
//! while (1) {
//!     // Explicitly wait for event or timeout
//!     vlib_process_wait_for_event_or_clock(vm, timeout);
//!
//!     // Manually dispatch on event type
//!     event_type = vlib_process_get_events(vm, &event_data);
//!     switch (event_type) {
//!         case EVENT1:
//!             handle_event1(event_data);
//!             vlib_process_suspend(vm, 0.1);
//!             handle_event1_continued(event_data);
//!             break;
//!         case ~0:  // timeout
//!             handle_periodic();
//!             break;
//!     }
//!     vec_reset_length(event_data);
//! }
//! ```
//!
//! ## Key Differences
//!
//! The most obvious difference when using Rust async for process nodes is that the event loop is
//! taken core of by infrastructure.
//!
//! Another difference is that a caller cannot be surprised by a callee suspending, since with
//! Rust the only way to suspend inside an async function, and the caller must call .await for an
//! async function to do anything.
//!
//! # Comparison with Other Rust Async Runtimes
//!
//! This runtime is designed for the unique constraints of VPP plugin development and
//! differs significantly from popular async runtimes such as [tokio][tokio].
//!
//! The primary difference is that there is a single async coroutine per VPP process node and
//! tasks cannot be spawned. However, multiple futures created and awaited simultaneously from
//! within the single VPP process node future. If you have a number of futures all of the same
//! type, you can use [`FuturesUnordered`][FuturesUnordered] to achieve this. If they are of
//! different types you can use [`select()`][select()].
//!
//! ## Important Notes for Plugin Authors
//!
//! ### CPU-Bound Work
//!
//! In common with VPP C code and regular tasks in other Rust async runtimes, CPU-bound work will
//! block both simultaneous events from being processed by the current task/process as well as
//! potentially other tasks/processes from performing work. Therefore, it is recommended to break
//! up large computations into chunks that yield periodically.
//!
//! ### Blocking I/O in Async Context
//!
//! In common with VPP C code and regular tasks in other Rust async runtimes, performing blocking
//! operations (file I/O, syscalls, synchronous locking) may impact concurrent events for the
//! current task/process as we as other tasks/processes. Generally, it's OK to use blocking I/O
//! when it's expected to complete quickly (such as writing a small amount of data to a file
//! locally). Similarly, it's OK to use synchronous locking if the contention is low.
//!
//! [tokio]: https://tokio.rs
//! [FuturesUnordered]: https://docs.rs/futures-util/0.3.32/futures_util/stream/futures_unordered/struct.FuturesUnordered.html
//! [select()]: https://docs.rs/futures-util/0.3.32/futures_util/future/fn.select.html

pub mod core;
pub mod mpsc;
mod tw_timer;

// Re-export these for convenience
pub use core::{
    LocalFutureObj, ProcessAsyncContext, ProcessNode, ProcessNodeRegistration, sleep, timeout,
};
pub use mpsc::{Receiver, Sender, channel};
