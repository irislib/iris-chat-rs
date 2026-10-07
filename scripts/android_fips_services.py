"""Allowlisted local service submissions; not wire totals or application delivery proof."""

SCOPE = "locally_originated_service_carrier_submissions"
SERVICE_PORTS = {"pubsub": 7368, "hashtree": 39018}
TRANSPORTS = ("udp", "ethernet", "tcp", "tor", "websocket", "webrtc", "ble", "sim", "other")
SERVICE_COUNTS = ("ambiguous_port_datagrams", "discarded_outputs")
TRANSPORT_COUNTS = ("submitted_packets", "fips_payload_bytes", "ethernet_framing_bytes")
DELIVERY_FIELDS = ("req_frames_received", "close_frames_received", "event_frames_received",
    "inv_frames_received", "want_frames_received", "want_frames_sent", "subscription_events_received",
    "expired_wants", "provider_cooldowns", "tcp_receive_batches", "tcp_datagrams_received",
    "tcp_datagrams_rejected", "tcp_poll_turns", "transport_errors")
MAX_COUNT = (1 << 63) - 1


def _count(value):
    assert type(value) is int and 0 <= value <= MAX_COUNT, "Invalid service counter"
    return value


def _identifier(value):
    assert type(value) is str and value, "Invalid service observation identity"
    return value


def _counts(value, fields):
    assert type(value) is dict
    return {key: _count(value[key]) for key in fields}


def filter_fips_services(source):
    """Reconstruct known aggregates; reject missing fields and unknown service/transport rows."""
    assert type(source) is dict and type(source["valid"]) is bool
    assert source["scope"] == SCOPE and type(source["status"]) is str
    result = dict(valid=source["valid"], status=source["status"], scope=SCOPE)
    if not result["valid"]:
        assert result["status"] in ("unavailable", "query_error", "timeout", "invalid_counters")
        assert not source.keys() & {"sample_id", "epoch_id", "elapsed_ms", "services", "pubsub_delivery"}
        return result
    assert result["status"] == "available"
    result.update(sample_id=_identifier(source["sample_id"]), epoch_id=_identifier(source["epoch_id"]),
                  elapsed_ms=_count(source["elapsed_ms"]))
    services = source["services"]
    assert type(services) is dict and services.keys() == SERVICE_PORTS.keys()
    result["services"] = {}
    for name, port in SERVICE_PORTS.items():
        service = services[name]
        assert type(service) is dict and _count(service["service_port"]) == port
        rows = service["transports"]
        assert type(rows) is list and len(rows) == len(TRANSPORTS)
        filtered = []
        for transport, row in zip(TRANSPORTS, rows):
            assert type(row) is dict and row["transport"] == transport
            filtered.append(dict(transport=transport, **_counts(row, TRANSPORT_COUNTS)))
        result["services"][name] = dict(service_port=port, **_counts(service, SERVICE_COUNTS), transports=filtered)
    delivery = source["pubsub_delivery"]  # Missing is distinct from explicit null.
    result["pubsub_delivery"] = None if delivery is None else _counts(delivery, DELIVERY_FIELDS)
    return result


def fips_service_interval(before, after):
    """Derive exact-pair deltas, retaining an explicit invalid reason without invented zeros.

    Carrier counters start when enabled; pubsub counters start with its client. Only
    differences within one epoch have a shared duration. Peer churn alone is irrelevant.
    """
    before, after = filter_fips_services(before), filter_fips_services(after)
    result = dict(valid=False, reason="query_unavailable", scope=SCOPE,
                  since_sample_id=before.get("sample_id"), sample_id=after.get("sample_id"),
                  elapsed_ms=None, service_deltas=None, pubsub_delivery_deltas=None)
    if not before["valid"] or not after["valid"]: return result
    if before["epoch_id"] != after["epoch_id"]:
        return dict(result, reason="epoch_changed")
    if before["sample_id"] == after["sample_id"]:
        return dict(result, reason="sample_repeated")
    elapsed = after["elapsed_ms"] - before["elapsed_ms"]
    if elapsed <= 0: return dict(result, reason="no_elapsed_time")
    result["elapsed_ms"] = elapsed
    old_delivery, new_delivery = before["pubsub_delivery"], after["pubsub_delivery"]
    if (old_delivery is None) != (new_delivery is None):
        return dict(result, reason="pubsub_changed")

    def delta(old, new, fields):
        values = {key: new[key] - old[key] for key in fields}
        if any(value < 0 for value in values.values()): raise ValueError("Counter reset")
        return values

    try:
        services = {}
        for name, port in SERVICE_PORTS.items():
            old, new = before["services"][name], after["services"][name]
            rows = [dict(transport=transport, **delta(left, right, TRANSPORT_COUNTS))
                    for transport, left, right in zip(TRANSPORTS, old["transports"], new["transports"])]
            services[name] = dict(service_port=port, **delta(old, new, SERVICE_COUNTS), transports=rows)
        delivery = None if old_delivery is None else delta(old_delivery, new_delivery, DELIVERY_FIELDS)
    except ValueError:
        return dict(result, reason="counter_reset")
    return dict(result, valid=True, reason="comparable", service_deltas=services, pubsub_delivery_deltas=delivery)
