#!/usr/bin/env python3
import copy
import unittest

from android_fips_services import (DELIVERY_FIELDS, MAX_COUNT, SERVICE_PORTS, SCOPE,
                                   TRANSPORTS, filter_fips_services, fips_service_interval)


def sample(identifier="sample:1", elapsed=0):
    return dict(valid=True, status="available", scope=SCOPE, sample_id=identifier,
                epoch_id="epoch:1", elapsed_ms=elapsed,
                services={name: dict(service_port=port, ambiguous_port_datagrams=0, discarded_outputs=0,
                    transports=[dict(transport=transport, submitted_packets=0, fips_payload_bytes=0,
                                     ethernet_framing_bytes=0) for transport in TRANSPORTS])
                    for name, port in SERVICE_PORTS.items()},
                pubsub_delivery={field: 0 for field in DELIVERY_FIELDS})


def pair():
    before, after = sample(), sample("sample:2", 130_000)
    after["services"]["pubsub"]["transports"][0].update(submitted_packets=3, fips_payload_bytes=250)
    after["pubsub_delivery"]["want_frames_sent"] = 2
    return before, after


class FipsServicesTest(unittest.TestCase):
    def rejects(self, value):
        with self.assertRaises((AssertionError, KeyError)):
            filter_fips_services(value)

    def test_rebuilds_allowlist_at_every_depth(self):
        value = sample()
        expected = copy.deepcopy(value)
        for row in (value, value["services"]["pubsub"], value["services"]["pubsub"]["transports"][0],
                    value["pubsub_delivery"]):
            row["peer_identity"] = "private-identity"
            row["address"] = "private-address"
        actual = filter_fips_services(value)
        self.assertEqual(expected, actual)
        value["services"]["pubsub"]["transports"][0]["submitted_packets"] = 99
        self.assertEqual(0, actual["services"]["pubsub"]["transports"][0]["submitted_packets"])

    def test_invalid_queries_are_explicit_without_synthetic_counters(self):
        for status in ("unavailable", "timeout", "query_error", "invalid_counters"):
            value = dict(valid=False, status=status, scope=SCOPE)
            self.assertEqual(value, filter_fips_services({**value, "private_peer": "secret"}))
            for key, data in (("services", {}), ("elapsed_ms", 0), ("pubsub_delivery", None),
                              ("sample_id", "sample:1"), ("epoch_id", "epoch:1")):
                self.rejects({**value, key: data})
            interval = fips_service_interval(sample(), value)
            self.assertFalse(interval["valid"])
            self.assertIsNone(interval["service_deltas"])
        self.rejects(dict(valid=False, status="available", scope=SCOPE))

    def test_counts_never_coerce_default_or_overflow(self):
        for bad in (False, "1", 1.0, -1, MAX_COUNT + 1, None):
            for location, key in ((lambda v: v, "elapsed_ms"),
                    (lambda v: v["services"]["pubsub"], "discarded_outputs"),
                    (lambda v: v["services"]["hashtree"]["transports"][0], "fips_payload_bytes"),
                    (lambda v: v["pubsub_delivery"], "transport_errors")):
                value = sample(); location(value)[key] = bad
                self.rejects(value)
        value = sample(); value["pubsub_delivery"]["transport_errors"] = MAX_COUNT
        self.assertEqual(MAX_COUNT, filter_fips_services(value)["pubsub_delivery"]["transport_errors"])
        value = sample(); del value["services"]["pubsub"]["ambiguous_port_datagrams"]
        self.rejects(value)
        for field in DELIVERY_FIELDS:
            value = sample(); del value["pubsub_delivery"][field]
            self.rejects(value)

    def test_exact_services_ports_transports_and_identifiers(self):
        for key, bad in (("valid", 1), ("status", "query_error"), ("scope", "all_traffic"),
                         ("sample_id", ""), ("epoch_id", 1)):
            value = sample(); value[key] = bad
            self.rejects(value)
        for mutate in (lambda v: v["services"].pop("pubsub"),
                       lambda v: v["services"].update(calls={}),
                       lambda v: v["services"]["pubsub"].update(service_port=39018),
                       lambda v: v["services"]["pubsub"]["transports"].pop(),
                       lambda v: v["services"]["pubsub"]["transports"][0].update(transport="unknown"),
                       lambda v: v["services"]["pubsub"]["transports"][0].update(transport="ethernet")):
            value = sample(); mutate(value); self.rejects(value)
        value = sample(); del value["pubsub_delivery"]
        self.rejects(value)
        value = sample(); value["pubsub_delivery"] = None
        self.assertIsNone(filter_fips_services(value)["pubsub_delivery"])

    def test_delta_retains_service_units_and_is_independent_of_peer_counts(self):
        before, after = pair()
        before["connected_peer_count"] = 1; after["connected_peer_count"] = 3
        interval = fips_service_interval(before, after)
        self.assertTrue(interval["valid"])
        self.assertEqual("comparable", interval["reason"])
        self.assertEqual(130_000, interval["elapsed_ms"])
        row = interval["service_deltas"]["pubsub"]["transports"][0]
        self.assertEqual(dict(transport="udp", submitted_packets=3, fips_payload_bytes=250,
                              ethernet_framing_bytes=0), row)
        self.assertEqual(2, interval["pubsub_delivery_deltas"]["want_frames_sent"])

    def test_changed_epoch_repeated_sample_time_or_regressions_invalidate_delta(self):
        for mutate, reason in (
                (lambda v: v.update(epoch_id="epoch:2"), "epoch_changed"),
                (lambda v: v.update(sample_id="sample:1"), "sample_repeated"),
                (lambda v: v.update(elapsed_ms=0), "no_elapsed_time"),
                (lambda v: v.update(pubsub_delivery=None), "pubsub_changed")):
            before, after = pair(); mutate(after)
            interval = fips_service_interval(before, after)
            self.assertEqual(reason, interval["reason"])
            self.assertFalse(interval["valid"])
            self.assertIsNone(interval["service_deltas"])
            self.assertIsNone(interval["pubsub_delivery_deltas"])
        for location, key in ((lambda v: v["services"]["hashtree"], "discarded_outputs"),
                (lambda v: v["services"]["hashtree"]["transports"][1], "ethernet_framing_bytes"),
                (lambda v: v["pubsub_delivery"], "tcp_poll_turns")):
            before, after = pair(); location(before)[key] = 10
            interval = fips_service_interval(before, after)
            self.assertEqual("counter_reset", interval["reason"])
            self.assertIsNone(interval["service_deltas"])
        before, after = pair(); before["pubsub_delivery"] = after["pubsub_delivery"] = None
        interval = fips_service_interval(before, after)
        self.assertTrue(interval["valid"])
        self.assertIsNone(interval["pubsub_delivery_deltas"])


if __name__ == "__main__": unittest.main()
