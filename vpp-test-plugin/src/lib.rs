//! Test VPP plugin
//!

use lazy_static::lazy_static;
use std::{fmt, ptr::NonNull, str::FromStr, sync::atomic::AtomicU64};

use vpp_plugin::{
    bindings::{ip4_header_t, vnet_api_error_t_VNET_API_ERROR_INVALID_VALUE},
    vlib::{
        self,
        counter::{CombinedCounter, CombinedCounterIndex, SimpleCounter, SimpleCounterIndex},
        node_generic::{
            generic_feature_node_x1, generic_feature_node_x4, FeatureNextNode,
            GenericFeatureNodeX1, GenericFeatureNodeX4,
        },
        BufferIndex,
    },
    vlib_cli_command, vlib_init_function, vlib_node, vlib_plugin_register, vlibapi,
    vnet::{
        error::{VnetError, VNET_ERR_INVALID_ARGUMENT},
        types::SwIfIndex,
    },
    vnet_feature_init,
    vppinfra::{error::ErrorStack, unlikely},
    ErrorCounters, NextNodes,
};

use crate::test_api::{TEST_NODE_TYPE_X1, TEST_NODE_TYPE_X4};

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

fn format_test_trace2(
    _vm: &mut vlib::MainRef,
    _node: &mut vlib::NodeRef<TestX4Node>,
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

lazy_static! {
    static ref SIMPLE_COUNTER: SimpleCounter =
        SimpleCounter::new("test-simple", "/net/test/simple");
    static ref COMBINED_COUNTER: CombinedCounter =
        CombinedCounter::new("test-combined", "/net/test/combined");
}

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
                    5 => {
                        SimpleCounterIndex::from_parts(&SIMPLE_COUNTER, 0).increment(vm, 1);
                        FeatureNextNode::NextFeature
                    }
                    6 => {
                        CombinedCounterIndex::from_parts(&COMBINED_COUNTER, 0).increment(
                            vm,
                            1,
                            b0.length_in_chain(vm),
                        );
                        FeatureNextNode::NextFeature
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

static TESTX4_NODE: TestX4Node = TestX4Node::new();

#[vlib_node(
    name = "testx4",
    instance = TESTX4_NODE,
    format_trace = format_test_trace2,
)]
struct TestX4Node;

impl TestX4Node {
    const fn new() -> Self {
        Self
    }
}

impl vlib::node::Node for TestX4Node {
    type Vector = BufferIndex;
    type Scalar = ();
    type Aux = ();

    type NextNodes = TestNextNode;
    type RuntimeData = ();
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

        impl GenericFeatureNodeX4<TestX4Node> for Impl {
            fn prefetch_buffer_x4(
                &self,
                _vm: &vlib::MainRef,
                _node: &mut vlib::NodeRuntimeRef<TestX4Node>,
                b: &mut [&mut vlib::BufferRef<<TestX4Node as vlib::node::Node>::FeatureData>; 4],
            ) {
                b.iter().for_each(|b0| {
                    b0.prefetch_header_load();
                    b0.prefetch_data_load();
                });
            }

            #[inline(always)]
            unsafe fn map_buffer_to_next_x4(
                &self,
                vm: &vlib::MainRef,
                node: &mut vlib::NodeRuntimeRef<TestX4Node>,
                b: &mut [&mut vlib::BufferRef<()>; 4],
            ) -> [FeatureNextNode<TestNextNode>; 4] {
                [
                    self.map_buffer_to_next(vm, node, b[0]),
                    self.map_buffer_to_next(vm, node, b[1]),
                    self.map_buffer_to_next(vm, node, b[2]),
                    self.map_buffer_to_next(vm, node, b[3]),
                ]
            }

            unsafe fn trace_buffer(
                &self,
                vm: &vlib::MainRef,
                node: &mut vlib::NodeRuntimeRef<TestX4Node>,
                b0: &mut vlib::BufferRef<<TestX4Node as vlib::node::Node>::FeatureData>,
            ) {
                let ip_udp = b0.current_ptr_mut() as *const IpUdpHeader;
                if usize::from(b0.current_length()) >= std::mem::size_of::<IpUdpHeader>() {
                    let t = b0.add_trace(vm, node);
                    t.write(TestTrace { header: *ip_udp });
                }
            }
        }

        impl GenericFeatureNodeX1<TestX4Node> for Impl {
            #[inline(always)]
            unsafe fn map_buffer_to_next(
                &self,
                _vm: &vlib::MainRef,
                node: &mut vlib::NodeRuntimeRef<TestX4Node>,
                b0: &mut vlib::BufferRef<()>,
            ) -> FeatureNextNode<TestNextNode> {
                if usize::from(b0.current_length()) < std::mem::size_of::<IpUdpHeader>() {
                    b0.set_error(node, TestErrorCounter::Drop);
                    return TestNextNode::Drop.into();
                }

                let ip_udp: *const IpUdpHeader = b0.current_ptr_mut() as *const IpUdpHeader;

                match u16::from_be((*ip_udp).udp.dst_port) {
                    1 => {
                        b0.set_error(node, TestErrorCounter::Drop);
                        TestNextNode::Drop.into()
                    }
                    _ => FeatureNextNode::NextFeature,
                }
            }
        }

        generic_feature_node_x4(vm, node, frame, Impl)
    }
}

vnet_feature_init! {
    identifier: TESTX4_FEAT,
    arc_name: "ip4-unicast",
    node: TestX4Node,
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

#[vlib_cli_command(
    path = "rust-test counter",
    short_help = "rust-test counter <simple|combined>"
)]
fn counter_test_command(vm: &mut vlib::BarrierHeldMainRef, input: &str) -> Result<(), ErrorStack> {
    if input == "simple" {
        let counter = SimpleCounter::new("ut-simple", "/net/ut/simple");
        let counter_index = counter.allocate_index(vm, 0);
        let (counter_ref, index) = counter_index.into_parts();
        let counter_index = unsafe { SimpleCounterIndex::from_parts(counter_ref, index) };
        counter_index.increment(vm, 1);
        let counter_val = counter_index.get(vm);
        if counter_val != 1 {
            return Err(ErrorStack::msg(format!(
                "Expected counter value to be 1 instead of {}",
                counter_val
            )));
        }
        unsafe {
            counter_index.zero();
        }
        let counter_val = counter_index.get(vm);
        if counter_val != 0 {
            return Err(ErrorStack::msg(format!(
                "Expected counter value to be 0 instead of {}",
                counter_val
            )));
        }
    } else if input == "combined" {
        let counter = CombinedCounter::new("ut-combined", "/net/ut/combined");
        let counter_index = counter.allocate_index(vm, 0);
        let (counter_ref, index) = counter_index.into_parts();
        let counter_index = unsafe { CombinedCounterIndex::from_parts(counter_ref, index) };
        counter_index.increment(vm, 1, 64); // Increment by 1 packet and 64 bytes
        let counter_val = counter_index.get(vm);
        if counter_val.packets != 1 || counter_val.bytes != 64 {
            return Err(ErrorStack::msg(format!(
                "Expected counter value to be (1, 64) instead of ({}, {})",
                counter_val.packets, counter_val.bytes
            )));
        }
        unsafe {
            counter_index.zero();
        }
        let counter_val = counter_index.get(vm);
        if counter_val.packets != 0 || counter_val.bytes != 0 {
            return Err(ErrorStack::msg(format!(
                "Expected counter value to be (0, 0) instead of ({}, {})",
                counter_val.packets, counter_val.bytes
            )));
        }
    } else {
        return Err(ErrorStack::msg(format!("Unrecognised input {}", input)));
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

        match mp.node_type {
            TEST_NODE_TYPE_X1 => {
                if mp.enable {
                    TEST_FEAT.enable(vm, sw_if_index, ())?;
                } else {
                    TEST_FEAT.disable(vm, sw_if_index)?;
                }
            }
            TEST_NODE_TYPE_X4 => {
                if mp.enable {
                    TESTX4_FEAT.enable(vm, sw_if_index, ())?;
                } else {
                    TESTX4_FEAT.disable(vm, sw_if_index)?;
                }
            }
            _ => return Err(VNET_ERR_INVALID_ARGUMENT.into()),
        }

        Ok(test_api::TestEnableDisableReply {
            context: mp.context,
            ..Default::default()
        }
        .into())
    }

    fn test_type_in_message(
        _vm: &vlib::BarrierHeldMainRef,
        mp: &test_api::TestTypeInMessage,
    ) -> Result<vlibapi::Message<test_api::TestTypeInMessageReply>, i32> {
        if mp.test_type.field1 != 42 {
            return Err(VNET_ERR_INVALID_ARGUMENT.into());
        }
        Ok(test_api::TestTypeInMessageReply {
            context: mp.context,
            ..Default::default()
        }
        .into())
    }

    fn test_array(
        _vm: &vlib::BarrierHeldMainRef,
        mp: &test_api::TestArray,
    ) -> Result<vlibapi::Message<test_api::TestArrayReply>, i32> {
        let array = mp.array1;
        if array != [42, 0xdeadbeef, 0, 0xffffffff] {
            return Err(VNET_ERR_INVALID_ARGUMENT.into());
        }
        let array = mp.array2.array;
        if array != [42, 0xffff] {
            return Err(VNET_ERR_INVALID_ARGUMENT.into());
        }
        let array = mp.array3;
        if array != [TEST_NODE_TYPE_X1, TEST_NODE_TYPE_X4] {
            return Err(VNET_ERR_INVALID_ARGUMENT.into());
        }
        if mp.array4[0] != 0.0 || mp.array4[1] != 42.0 {
            return Err(VNET_ERR_INVALID_ARGUMENT.into());
        }
        Ok(test_api::TestArrayReply {
            context: mp.context,
            ..Default::default()
        }
        .into())
    }
}

#[vlib_init_function]
fn test_init(vm: &mut vlib::BarrierHeldMainRef) -> Result<(), ErrorStack> {
    test_api::test_register_messages::<ApiHandler>();
    SIMPLE_COUNTER.allocate_index(vm, 0);
    COMBINED_COUNTER.allocate_index(vm, 0);

    Ok(())
}

vlib_plugin_register! {
    version: "1.0",
    description: "Test",
}
