//! Test VPP plugin
//!

use std::{fmt, ptr::NonNull, str::FromStr, sync::atomic::AtomicU64};

use vpp_plugin::{
    bindings::{ip4_header_t, vnet_api_error_t_VNET_API_ERROR_INVALID_VALUE},
    vlib::{
        self,
        node_generic::{generic_feature_node_x1, FeatureNextNode, GenericFeatureNodeX1},
        BufferIndex,
    },
    vlib_cli_command, vlib_init_function, vlib_node, vlib_plugin_register, vlibapi,
    vnet::{error::VnetError, types::SwIfIndex},
    vnet_feature_init,
    vppinfra::{error::ErrorStack, unlikely},
    ErrorCounters, NextNodes,
};

use crate::test_api::TestEnableDisableReply;

mod test_api {
    include!(concat!(env!("OUT_DIR"), "/src/test_api.rs"));
}

#[repr(C, packed)]
#[derive(Debug, Default, Copy, Clone)]
struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

#[repr(C, packed)]
#[derive(Debug, Default, Copy, Clone)]
struct IpUdpHeader {
    pub ip: ip4_header_t,
    pub udp: UdpHeader,
}

#[derive(Copy, Clone)]
struct TestTrace {
    header: IpUdpHeader,
}

impl fmt::Display for TestTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "header: {:?}", self.header)
    }
}

fn format_test_trace(
    _vm: &mut vlib::MainRef,
    _node: &mut vlib::NodeRef<TestNode>,
    t: &TestTrace,
) -> String {
    t.to_string()
}

#[derive(NextNodes)]
enum TestNextNode {
    #[next_node = "drop"]
    Drop,
}

#[derive(ErrorCounters)]
enum TestErrorCounter {
    #[error_counter(description = "Drops", severity = ERROR)]
    Drop,
}

#[derive(Copy, Clone)]
struct TestRuntimeData {
    drop_error_ptr: Option<NonNull<u64>>,
}

// SAFETY: this is safe to implement even though drop_error_ptr is thread-local because there are
// no safe methods on TestRuntimeData, so accessing drop_error_ptr is unsafe anyway.
unsafe impl Send for TestRuntimeData {}
// SAFETY: this is safe to implement even though drop_error_ptr is thread-local because there are
// no safe methods on TestRuntimeData, so accessing drop_error_ptr is unsafe anyway.
unsafe impl Sync for TestRuntimeData {}

/// Initialisation data used in node function init
static TEST_RUNTIME_DATA_INIT: TestRuntimeData = TestRuntimeData {
    drop_error_ptr: None,
};

/// Tests using runtime data by incrementing a drop counter, caching the pointer to it
///
/// This probably isn't better for performance and certainly isn't better for maintainability,
/// so don't re-use this without profiling before and after.
fn increment_drop_counter_cached(
    vm: &vlib::MainRef,
    node: &mut vlib::NodeRuntimeRef<TestNode>,
    increment: u64,
) {
    unsafe {
        let node_counter_base_index = (*node.node(vm).as_ptr()).error_heap_index;
        let runtime_data = node.runtime_data_mut();
        let ptr = runtime_data.drop_error_ptr.get_or_insert_with(|| {
            let em = &(*vm.as_ptr()).error_main;
            NonNull::new_unchecked(
                em.counters
                    .add(node_counter_base_index as usize + TestErrorCounter::Drop as usize),
            )
        });
        AtomicU64::from_ptr(ptr.as_ptr()).store(
            *ptr.as_ptr() + increment,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

static TEST_NODE: TestNode = TestNode::new();

#[vlib_node(
    name = "test",
    instance = TEST_NODE,
    runtime_data_default = TEST_RUNTIME_DATA_INIT,
    format_trace = format_test_trace,
)]
struct TestNode;

impl TestNode {
    const fn new() -> Self {
        Self
    }
}

impl vlib::node::Node for TestNode {
    type Vector = BufferIndex;
    type Scalar = ();
    type Aux = ();

    type NextNodes = TestNextNode;
    type RuntimeData = TestRuntimeData;
    type TraceData = TestTrace;
    type Errors = TestErrorCounter;
    type FeatureData = ();

    #[inline(always)]
    unsafe fn function(
        &self,
        vm: &mut vlib::MainRef,
        node: &mut vlib::NodeRuntimeRef<Self>,
        frame: &mut vlib::FrameRef<Self>,
    ) -> u16 {
        struct Impl;
        impl GenericFeatureNodeX1<TestNode> for Impl {
            #[inline(always)]
            unsafe fn map_buffer_to_next(
                &self,
                vm: &vlib::MainRef,
                node: &mut vlib::NodeRuntimeRef<TestNode>,
                b0: &mut vlib::BufferRef<()>,
            ) -> FeatureNextNode<TestNextNode> {
                if usize::from(b0.current_length()) < std::mem::size_of::<IpUdpHeader>() {
                    b0.set_error(node, TestErrorCounter::Drop);
                    return TestNextNode::Drop.into();
                }

                let ip_udp: *const IpUdpHeader = b0.current_ptr_mut() as *const IpUdpHeader;

                let next = match u16::from_be((*ip_udp).udp.dst_port) {
                    // 1 falls through into default case to test the simple case
                    2 => FeatureNextNode::NextFeature,
                    3 => {
                        node.increment_error_counter(vm, TestErrorCounter::Drop, 1);
                        TestNextNode::Drop.into()
                    }
                    4 => {
                        increment_drop_counter_cached(vm, node, 1);
                        TestNextNode::Drop.into()
                    }
                    _ => {
                        b0.set_error(node, TestErrorCounter::Drop);
                        TestNextNode::Drop.into()
                    }
                };

                if unlikely(b0.flags().contains(vlib::BufferFlags::IS_TRACED)) {
                    let t = b0.add_trace(vm, node);
                    t.write(TestTrace { header: *ip_udp });
                }

                next
            }
        }
        generic_feature_node_x1(vm, node, frame, Impl)
    }
}

vnet_feature_init! {
    identifier: TEST_FEAT,
    arc_name: "ip4-unicast",
    node: TestNode,
}

#[vlib_cli_command(
    path = "rust-test node",
    short_help = "rust-test node <interface-name> [disable]"
)]
fn enable_disable_command(
    vm: &mut vlib::BarrierHeldMainRef,
    input: &str,
) -> Result<(), ErrorStack> {
    let args: Vec<_> = input.split(' ').collect();
    if args.is_empty() {
        return Err(ErrorStack::msg("Missing interface name"));
    }

    let mut enable = true;
    let sw_if_index = SwIfIndex::from_str(args[0])
        .map_err(|_| ErrorStack::msg(format!("Invalid interface name {}", args[0])))?;

    if args.len() >= 2 {
        if args[1] == "disable" {
            enable = false;
        } else {
            return Err(ErrorStack::msg(format!("Unrecognised option {}", args[1])));
        }
    }

    if enable {
        TEST_FEAT
            .enable(vm, sw_if_index, ())
            .map_err(|e| e.context("Failed to enable test feature"))?;
    } else {
        TEST_FEAT
            .disable(vm, sw_if_index)
            .map_err(|e| e.context("Failed to disable test feature"))?;
    }

    Ok(())
}

#[vlib_cli_command(
    path = "rust-test negative",
    short_help = "rust-test negative <vnet-error>"
)]
fn negative_test_command(
    _vm: &mut vlib::BarrierHeldMainRef,
    input: &str,
) -> Result<(), ErrorStack> {
    if input == "vnet-error" {
        return Err(VnetError::from(vnet_api_error_t_VNET_API_ERROR_INVALID_VALUE).context("Test"));
    } else {
        return Err(ErrorStack::msg(format!("Unrecognised input {}", input)));
    }
}

#[vlib_cli_command(path = "rust-test message", short_help = "rust-test message")]
fn message_test_command(
    _vm: &mut vlib::BarrierHeldMainRef,
    _input: &str,
) -> Result<(), ErrorStack> {
    // Test some basic operations not tested from the auto-generated API code
    let mut message = vlibapi::Message::from(0u8);
    if *message != 0 {
        return Err(ErrorStack::msg(format!(
            "Expected *message to be 0, but is {}",
            *message
        )));
    }
    *message = 1;
    if *message != 1 {
        return Err(ErrorStack::msg(format!(
            "Expected *message to be 1, but is {}",
            *message
        )));
    }

    // Test functions for partial initialisation
    let mut message = vlibapi::Message::<[u8; 256]>::new_uninit();
    unsafe {
        for i in 0..256 {
            *(message.as_mut_ptr().add(i) as *mut u8) = 0;
        }
        let message = message.assume_init();
        if *message != [0u8; 256] {
            return Err(ErrorStack::msg(format!(
                "Expected *message to be [0u8; 256], but is {:?}",
                *message
            )));
        }
    }
    let message = vlibapi::Message::<u8>::new_uninit();
    let message = message.write(0);
    if *message != 0 {
        return Err(ErrorStack::msg(format!(
            "Expected *message to be 0, but is {}",
            *message
        )));
    }

    Ok(())
}

struct ApiHandler;

impl test_api::Handlers for ApiHandler {
    fn test_enable_disable(
        vm: &vpp_plugin::vlib::BarrierHeldMainRef,
        mp: &test_api::TestEnableDisable,
    ) -> Result<vlibapi::Message<test_api::TestEnableDisableReply>, i32> {
        let sw_if_index = SwIfIndex::new(mp.sw_if_index);

        if mp.enable {
            TEST_FEAT.enable(vm, sw_if_index, ())?;
        } else {
            TEST_FEAT.disable(vm, sw_if_index)?;
        }

        Ok(TestEnableDisableReply {
            context: mp.context,
            ..Default::default()
        }
        .into())
    }
}

#[vlib_init_function]
fn test_init(_vm: &mut vlib::BarrierHeldMainRef) -> Result<(), ErrorStack> {
    test_api::test_register_messages::<ApiHandler>();

    Ok(())
}

vlib_plugin_register! {
    version: "1.0",
    description: "Test",
}
