//! VPP application library
//!
//! This module contains abstractions around VPP's application layer, `vlib`.

pub mod buffer;
pub mod cli;
pub mod counter;
pub mod main;
pub mod node;
pub mod node_generic;
#[cfg(feature = "process-node")]
pub mod process_node;

pub use buffer::{BufferFlags, BufferIndex, BufferRef};
pub use main::{BarrierHeldMainRef, MainRef};
pub use node::{FrameRef, NodeFlags, NodeRef, NodeRuntimeRef};
#[cfg(feature = "process-node")]
pub use process_node::ProcessNode;
