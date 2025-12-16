#!/usr/bin/env python3
""" Integration tests """

import unittest

from scapy.layers.inet import IP
from scapy.layers.l2 import Ether
from scapy.layers.inet import UDP

from framework import VppTestCase
from asfframework import (
    tag_run_solo,
    VppTestRunner,
)
from util import ppp
from vpp_papi_provider import CliFailedCommandError
from vpp_papi import VppEnum


@tag_run_solo
class IntegrationTestCase(VppTestCase):
    """Integration tests"""

    pg0 = None
    pg1 = None

    @classmethod
    def setUpClass(cls):
        super(IntegrationTestCase, cls).setUpClass()
        cls.__doc__ = (
            """Integration tests"""
        )
        try:
            cls.create_pg_interfaces([0, 1])
            cls.pg0.config_ip4()
            cls.pg0.config_ip6()
            cls.pg0.configure_ipv4_neighbors()
            cls.pg0.admin_up()
            cls.pg0.resolve_arp()
            cls.pg0.resolve_ndp()
            cls.pg1.config_ip4()
            cls.pg1.config_ip6()
            cls.pg1.configure_ipv4_neighbors()
            cls.pg1.admin_up()
            cls.pg1.resolve_arp()
            cls.pg1.resolve_ndp()

        except Exception:
            super(IntegrationTestCase, cls).tearDownClass()
            raise

    @classmethod
    def tearDownClass(cls):
        super(IntegrationTestCase, cls).tearDownClass()

    def setUp(self):
        super(IntegrationTestCase, self).setUp()
        self.pg0.enable_capture()
        self.pg1.enable_capture()

    def tearDown(self):
        self.vapi.collect_events()  # clear the event queue
        super(IntegrationTestCase, self).tearDown()

    def cli_verify_no_response(self, cli):
        """execute a CLI, asserting that the response is empty"""
        self.assert_equal(self.vapi.cli(cli), "", "CLI command response")

    def cli_verify_response(self, cli, expected):
        """execute a CLI, asserting that the response matches expectation"""
        try:
            reply = self.vapi.cli(cli)
        except CliFailedCommandError as cli_error:
            reply = str(cli_error)
        self.assert_equal(reply.strip(), expected, "CLI command response")

    def create_packet(self, test_case):
        """create a packet"""
        packet = (
            Ether(
                src=self.pg0.remote_mac, dst=self.pg0.local_mac
            )
            / IP(src=self.pg0.remote_ip4, dst=self.pg1.remote_ip4, ttl=255)
            / UDP(dport=test_case)
        )
        return packet

    def test_drop_cli(self):
        """Drop with feature enabled via CLI"""
        err = self.statistics.get_err_counter("/err/test/Drop")
        self.cli_verify_no_response(f"rust-test node {self.pg0.name}")

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counter to have been incremented by one
        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err + 1)

        # Now disable and expect the packet counter to not be incremented
        err = new_err
        self.cli_verify_no_response(f"rust-test node {self.pg0.name} disable")

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err)

    def enable_disable_api(self, sw_if_index, enable, node_type = "x1"):

        test_node_type = VppEnum.vl_api_test_node_type_t
        if node_type == "x1":
            api_node_type = test_node_type.TEST_NODE_TYPE_X1
        elif node_type == "x4":
            api_node_type = test_node_type.TEST_NODE_TYPE_X4
        else:
            raise Exception(f"Invalid node type: {node_type}")
        self.vapi.api(
            self.vapi.papi.test_enable_disable,
            {
                'sw_if_index': sw_if_index,
                'enable': enable,
                'node_type': api_node_type,
            },
        )

    def test_drop_api(self):
        """Drop with feature enabled via API"""
        err = self.statistics.get_err_counter("/err/test/Drop")
        self.enable_disable_api(self.pg0.sw_if_index, True)

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counter to have been incremented by one
        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err + 1)

        # Now disable and expect the packet counter to not be incremented
        err = new_err
        self.enable_disable_api(self.pg0.sw_if_index, False)

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err)

    def test_next_feature(self):
        """Forward to next feature"""
        self.enable_disable_api(self.pg0.sw_if_index, True)

        packet = self.create_packet(2)
        self.logger.info(ppp("Sending packet:", packet))
        self.send_and_expect(self.pg0, packet, self.pg1)

        self.logger.debug(self.vapi.cli("show trace"))

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False)

    def test_drop_manual_counter(self):
        """Drop with drop counter manual increment"""
        err = self.statistics.get_err_counter("/err/test/Drop")
        self.enable_disable_api(self.pg0.sw_if_index, True)

        packet = self.create_packet(3)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counters to have been incremented by one
        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err + 1)

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False)

    def test_drop_using_runtime_data_cache(self):
        """Drop using runtime data cache"""
        err = self.statistics.get_err_counter("/err/test/Drop")
        self.enable_disable_api(self.pg0.sw_if_index, True)

        # Send two packets, one to prime the cache and the second to use the cache
        packet = self.create_packet(4)
        self.logger.info(ppp("Sending two packets of:", packet))
        self.pg0.add_stream([packet, packet])
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counters to have been incremented by two
        new_err = self.statistics.get_err_counter("/err/test/Drop")
        self.assertEqual(new_err, err + 2)

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False)

    def test_node_simple_counter(self):
        """Feature node incrementing simple counter"""
        simple_count = self.statistics.get_counter("/net/test/simple")[0][0]
        self.enable_disable_api(self.pg0.sw_if_index, True)

        packet = self.create_packet(5)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counter to have been incremented by one
        new_simple_count = self.statistics.get_counter("/net/test/simple")[0][0]
        self.assertEqual(new_simple_count, simple_count + 1)

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False)

    def test_node_combined_counter(self):
        """Feature node incrementing combined counter"""
        combined_count = self.statistics.get_counter("/net/test/combined")[0]
        self.enable_disable_api(self.pg0.sw_if_index, True)

        packet = self.create_packet(6)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        self.logger.debug(self.vapi.cli("show buffers"))

        # We only count the layer-3 packet size from an ip4-input feature
        packet_len = len(packet[IP])
        # Expect the combined counter to have been incremented by one packet and its size in bytes
        new_combined_count = self.statistics.get_counter("/net/test/combined")[0]
        self.assertEqual(new_combined_count[0]["packets"], combined_count[0]["packets"] + 1)
        self.assertEqual(new_combined_count[0]["bytes"], combined_count[0]["bytes"] + packet_len)

        # Now send a large packet that results in a chained buffer
        combined_count = new_combined_count
        packet = self.create_packet(6) / (3000 * "0")
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # We only count the layer-3 packet size from an ip4-input feature
        packet_len = len(packet[IP])
        # Expect the combined counter to have been incremented by one packet and its size in bytes
        new_combined_count = self.statistics.get_counter("/net/test/combined")[0]
        self.assertEqual(new_combined_count[0]["packets"], combined_count[0]["packets"] + 1)
        self.assertEqual(new_combined_count[0]["bytes"], combined_count[0]["bytes"] + packet_len)

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False)

    def test_node_x4(self):
        """Node processing 4 buffers at a time"""
        err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.enable_disable_api(self.pg0.sw_if_index, True, node_type="x4")

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        # Send full frame of 256 packets to validate that corner case
        self.pg0.add_stream(256 * packet)
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counter to have been incremented by 256
        new_err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.assertEqual(new_err, err + 256)

        # Now send frame of just one packet to validate that corner case
        err = new_err
        self.pg0.add_stream(packet)
        self.pg_start()

        # Expect the packet counter to have been incremented by 1
        new_err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.assertEqual(new_err, err + 1)

        # Now disable and expect the packet counter to not be incremented
        err = new_err
        self.enable_disable_api(self.pg0.sw_if_index, False, node_type="x4")

        packet = self.create_packet(1)
        self.logger.info(ppp("Sending packet:", packet))
        self.pg0.add_stream(packet)
        self.pg_start()

        new_err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.assertEqual(new_err, err)

    def test_node_x4_next_feature(self):
        """Node processing 4 buffers at a time, forward to next feature"""
        self.enable_disable_api(self.pg0.sw_if_index, True, node_type="x4")

        packet = self.create_packet(2)
        self.logger.info(ppp("Sending packet:", packet))
        # Send full frame of 256 packets to validate that corner case
        self.send_and_expect(self.pg0, 256 * packet, self.pg1)

        # Now send frame of just one packet to validate that corner case
        self.send_and_expect(self.pg0, packet, self.pg1)

        self.logger.debug(self.vapi.cli("show trace"))

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False, node_type="x4")

    def test_node_x4_mixed_next(self):
        """Node processing 4 buffers at a time, mixed next nodes"""
        err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.enable_disable_api(self.pg0.sw_if_index, True, node_type="x4")

        packet1 = self.create_packet(1)
        packet2 = self.create_packet(2)
        # Send frame of 4 packets resulting in interleaved mixed next nodes
        self.pg0.add_stream([packet1, packet2, packet1, packet2])
        self.pg_start()

        self.logger.debug(self.vapi.cli("show trace"))

        # Expect the packet counter to have been incremented by 2
        new_err = self.statistics.get_err_counter("/err/testx4/Drop")
        self.assertEqual(new_err, err + 2)

        # Clean up
        self.enable_disable_api(self.pg0.sw_if_index, False, node_type="x4")

    def test_vnet_error(self):
        """VNET error being generated and returned from a CLI command"""
        self.cli_verify_response(f"rust-test negative vnet-error", "rust-test negative: Invalid value (Test)")

    def test_message(self):
        """Messages"""
        self.cli_verify_no_response("rust-test message")

    def test_counters(self):
        """Counters"""
        self.cli_verify_no_response(f"rust-test counter simple")
        self.cli_verify_no_response(f"rust-test counter combined")

if __name__ == "__main__":
    unittest.main(testRunner=VppTestRunner)
