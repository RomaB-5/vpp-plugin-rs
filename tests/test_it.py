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

    def enable_disable_api(self, sw_if_index, enable):
        self.vapi.api(
            self.vapi.papi.test_enable_disable,
            {
                'sw_if_index': sw_if_index,
                'enable': enable
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

    def test_vnet_error(self):
        """VNET error being generated and returned from a CLI command"""
        self.cli_verify_response(f"rust-test negative vnet-error", "rust-test negative: Invalid value (Test)")

if __name__ == "__main__":
    unittest.main(testRunner=VppTestRunner)
