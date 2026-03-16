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

pub mod core;
pub mod mpsc;
mod tw_timer;

// Re-export these for convenience
pub use core::{LocalFutureObj, ProcessAsyncContext, ProcessNode, ProcessNodeRegistration, sleep};
pub use mpsc::channel;
