#!/usr/bin/env python3
import copy
import unittest

from android_fips_health import (INVALID_INTERVALS, MAX_COUNT, QUERY_ERRORS, TRAFFIC,
                                 comparable_fips_interval, filter_fips_health,
                                 require_single_static_udp_peer)


def sample(identifier="process:1"):
    return dict(valid=True, status="available", scope="connected_authenticated_peers",
                sample_id=identifier, connected_peer_count=1, configured_direct_peer_count=1,
                connected_configured_direct_peer_count=1, unexpected_connected_peer_count=0,
                transports={"udp": dict(connected_peer_count=1, rx_packets=2, tx_packets=3,
                                        rx_bytes=100, tx_bytes=200)},
                interval=dict(valid=False, reason="baseline", since_sample_id=None,
                              elapsed_ms=None, transport_deltas=None))


def pair():
    before, after = sample(), sample("process:2")
    delta = dict(rx_packets=1, tx_packets=2, rx_bytes=50, tx_bytes=75)
    for key in TRAFFIC:
        after["transports"]["udp"][key] += delta[key]
    after["interval"] = dict(valid=True, reason="comparable", since_sample_id=before["sample_id"],
                             elapsed_ms=130_000, transport_deltas={"udp": delta})
    return before, after


class FipsHealthTests(unittest.TestCase):
    def rejects(self, value):
        with self.assertRaises((AssertionError, KeyError)):
            filter_fips_health(value)

    def test_nested_private_fields_are_not_copied(self):
        before, after = pair()
        expected = copy.deepcopy(after)
        for row in [after, after["transports"]["udp"], after["interval"],
                    after["interval"]["transport_deltas"]["udp"]]:
            row["peer_identity"] = "private-key"
            row["address"] = "udp:private-host"
        result = filter_fips_health(after)
        self.assertEqual(expected, result)
        after["transports"]["udp"]["rx_packets"] = 900
        self.assertEqual(3, result["transports"]["udp"]["rx_packets"])
        self.assertEqual(expected["interval"], comparable_fips_interval(before, result))

    def test_errors_are_explicit_and_never_fabricate_zero_counters(self):
        for status in QUERY_ERRORS:
            with self.subTest(status=status):
                value = dict(valid=False, status=status, scope="connected_authenticated_peers")
                self.assertEqual(value, filter_fips_health({**value, "raw_peer": "private"}))
                with self.assertRaises(AssertionError):
                    require_single_static_udp_peer(value)
                self.rejects({**value, "connected_peer_count": 0})

    def test_missing_coerced_negative_and_overflow_counts_fail(self):
        for value in [False, "1", 1.0, -1, MAX_COUNT + 1, None]:
            with self.subTest(value=value):
                changed = sample()
                changed["transports"]["udp"]["rx_packets"] = value
                self.rejects(changed)
        changed = sample()
        del changed["transports"]["udp"]["rx_packets"]
        self.rejects(changed)
        changed = sample()
        changed["transports"]["udp"]["rx_bytes"] = MAX_COUNT
        self.assertEqual(MAX_COUNT, filter_fips_health(changed)["transports"]["udp"]["rx_bytes"])

    def test_invalid_enums_identity_types_and_count_sums_fail(self):
        for key, value in [("valid", 1), ("status", "timeout"), ("scope", "all_peers"),
                           ("sample_id", 123), ("sample_id", ""),
                           ("configured_direct_peer_count", 0), ("unexpected_connected_peer_count", 1)]:
            with self.subTest(key=key, value=value):
                changed = sample()
                changed[key] = value
                self.rejects(changed)
        changed = sample()
        changed["transports"]["udp"]["connected_peer_count"] = 0
        self.rejects(changed)
        changed = sample()
        changed["transports"]["unknown"] = changed["transports"].pop("udp")
        self.rejects(changed)

    def test_no_peer_is_observable_but_not_a_successful_isolation_gate(self):
        value = sample()
        value.update(connected_peer_count=0, connected_configured_direct_peer_count=0, transports={})
        self.assertEqual(value, filter_fips_health(value))
        with self.assertRaises(AssertionError):
            require_single_static_udp_peer(value)
        value = sample()
        value.update(connected_configured_direct_peer_count=0, unexpected_connected_peer_count=1)
        with self.assertRaises(AssertionError):
            require_single_static_udp_peer(value)
        self.assertEqual(sample(), require_single_static_udp_peer(sample()))

    def test_invalid_intervals_never_supply_deltas(self):
        for reason in INVALID_INTERVALS:
            with self.subTest(reason=reason):
                value = sample("process:2")
                value["interval"]["reason"] = reason
                if reason != "baseline":
                    value["interval"]["since_sample_id"] = "process:1"
                    value["interval"]["elapsed_ms"] = None if reason == "no_elapsed_time" else 100
                self.assertEqual(value, filter_fips_health(value))
                with self.assertRaises(AssertionError):
                    comparable_fips_interval(sample(), value)
                value["interval"]["transport_deltas"] = {}
                self.rejects(value)

    def test_interval_requires_explicit_nulls_and_consistent_validity(self):
        for field in ["since_sample_id", "elapsed_ms", "transport_deltas"]:
            changed = sample()
            del changed["interval"][field]
            self.rejects(changed)
        for field, value in [("valid", True), ("reason", "comparable"),
                             ("since_sample_id", "process:0"), ("elapsed_ms", 0)]:
            changed = sample()
            changed["interval"][field] = value
            self.rejects(changed)

    def test_exact_sample_pair_and_counter_difference_are_required(self):
        before, after = pair()
        self.assertEqual(130_000, comparable_fips_interval(before, after)["elapsed_ms"])
        mutations = [lambda v: v["interval"].update(since_sample_id="another-process:1"),
                     lambda v: v["interval"]["transport_deltas"]["udp"].update(rx_bytes=0),
                     lambda v: v["transports"]["udp"].update(rx_bytes=1),
                     lambda v: v["interval"].update(transport_deltas={})]
        for mutate in mutations:
            changed = copy.deepcopy(after)
            mutate(changed)
            with self.assertRaises(AssertionError):
                comparable_fips_interval(before, changed)


if __name__ == "__main__":
    unittest.main()
