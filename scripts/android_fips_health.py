"""Validate aggregate-only, one-shot FIPS diagnostics; never infer missing counters."""

TRANSPORTS = frozenset(("udp", "websocket", "webrtc", "tcp", "ble", "ethernet", "tor", "other"))
TRAFFIC = ("rx_packets", "tx_packets", "rx_bytes", "tx_bytes")
PEERS = ("connected_peer_count", "configured_direct_peer_count",
         "connected_configured_direct_peer_count", "unexpected_connected_peer_count")
QUERY_ERRORS = frozenset(("unavailable", "timeout", "query_error", "invalid_counters", "diagnostics_error"))
INVALID_INTERVALS = frozenset(("baseline", "endpoint_changed", "peer_changed", "counter_reset", "no_elapsed_time"))
MAX_COUNT = (1 << 63) - 1


def _count(value):
    assert type(value) is int and 0 <= value <= MAX_COUNT, "Invalid FIPS counter"
    return value


def _boolean(value):
    assert type(value) is bool, "Invalid FIPS validity flag"
    return value


def _sample_id(value):
    assert type(value) is str and value, "Invalid FIPS sample ID"
    return value


def _rows(value, current):
    assert type(value) is dict and value.keys() <= TRANSPORTS, "Unknown FIPS transport"
    fields = (("connected_peer_count",) if current else ()) + TRAFFIC
    result = {}
    for transport, row in value.items():
        assert type(row) is dict
        result[transport] = {key: _count(row[key]) for key in fields}
    return result


def filter_fips_health(source):
    """Reconstruct an allowlisted tree. Invalid query results keep their explicit status."""
    assert type(source) is dict
    valid = _boolean(source["valid"])
    status = source["status"]
    assert type(status) is str and source["scope"] == "connected_authenticated_peers"
    result = dict(valid=valid, status=status, scope="connected_authenticated_peers")
    if not valid:
        assert status in QUERY_ERRORS
        assert not source.keys() & set(PEERS + ("sample_id", "transports", "interval"))
        return result
    assert status == "available"
    result["sample_id"] = _sample_id(source["sample_id"])
    result.update({key: _count(source[key]) for key in PEERS})
    matched = result["connected_configured_direct_peer_count"]
    assert matched <= result["configured_direct_peer_count"]
    assert matched + result["unexpected_connected_peer_count"] == result["connected_peer_count"]
    result["transports"] = _rows(source["transports"], current=True)
    assert sum(row["connected_peer_count"] for row in result["transports"].values()) == result["connected_peer_count"]

    interval = source["interval"]
    assert type(interval) is dict
    comparable = _boolean(interval["valid"])
    reason = interval["reason"]
    assert type(reason) is str
    since = interval["since_sample_id"]
    if since is not None:
        since = _sample_id(since)
    elapsed = interval["elapsed_ms"]
    if elapsed is not None:
        elapsed = _count(elapsed)
        assert elapsed > 0
    deltas = interval["transport_deltas"]
    if comparable:
        assert reason == "comparable" and since is not None and elapsed is not None
        assert since != result["sample_id"]
        deltas = _rows(deltas, current=False)
    else:
        assert reason in INVALID_INTERVALS and deltas is None
        if reason == "baseline":
            assert since is None and elapsed is None
    result["interval"] = dict(valid=comparable, reason=reason, since_sample_id=since,
                              elapsed_ms=elapsed, transport_deltas=deltas)
    return result


def require_single_static_udp_peer(source):
    """A configured peer alone is insufficient; require its authenticated live connection."""
    value = filter_fips_health(source)
    assert value["valid"], "FIPS connection observation unavailable"
    assert all(value[key] == 1 for key in PEERS[:3]) and value[PEERS[3]] == 0, "FIPS isolation not observed"
    assert set(value["transports"]) == {"udp"}, "Unexpected FIPS transport"
    assert value["transports"]["udp"]["connected_peer_count"] == 1
    return value


def comparable_fips_interval(before, after):
    """Require an exact pair and verify deltas against both observed cumulative snapshots."""
    before, after = filter_fips_health(before), filter_fips_health(after)
    assert before["valid"] and after["valid"], "FIPS query failed"
    interval = after["interval"]
    assert interval["valid"] and interval["since_sample_id"] == before["sample_id"], "FIPS interval not comparable"
    assert all(before[key] == after[key] for key in PEERS), "FIPS peer counts changed"
    assert before["transports"].keys() == after["transports"].keys() == interval["transport_deltas"].keys()
    for transport, old in before["transports"].items():
        new = after["transports"][transport]
        assert old["connected_peer_count"] == new["connected_peer_count"]
        for key in TRAFFIC:
            assert new[key] >= old[key], "FIPS counter reset"
            assert interval["transport_deltas"][transport][key] == new[key] - old[key], "Incorrect FIPS delta"
    return interval
