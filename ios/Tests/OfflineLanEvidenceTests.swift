import Foundation
import XCTest

final class OfflineLanEvidenceTests: XCTestCase {
    private let http = "http://127.0.0.1:49000"
    private let scope = "iris-offline-lan-0123456789abcdef"
    private let wifi = [OfflineLanWiFiAddress(interface: "wifi", address: "192.168.1.10", netmask: "255.255.255.0")]

    private func sample(
        epoch: String = "first", hits: UInt64 = 0, misses: UInt64 = 0,
        blocks: UInt64 = 0, bytes: UInt64 = 0, httpBlocks: UInt64 = 0,
        transport: String = "udp", address: String = "192.168.1.20:5000"
    ) throws -> OfflineLanReadEvidence {
        try OfflineLanReadEvidence([
            "schema": 1, "valid": true, "isolation_valid": true, "epoch": epoch,
            "network": ["lan_scope": scope, "fixture_scope_valid": true],
            "blossom_read_servers": [http], "blossom_write_servers": [http],
            "peers": [["npub": "expected-peer", "transport": transport, "address": address]],
            "reads": ["local_hits": hits, "local_misses": misses, "peer_blocks": blocks,
                      "peer_bytes": bytes, "http_blocks": httpBlocks, "http_bytes": UInt64(0)],
        ], httpURL: http, discoveryScope: scope)
    }

    func testOnlyMinimalInitializingShapeCanBeRetried() {
        XCTAssertTrue(OfflineLanReadEvidence.isInitializing(["schema": 1, "valid": false]))
        XCTAssertFalse(OfflineLanReadEvidence.isInitializing([:]))
        XCTAssertFalse(OfflineLanReadEvidence.isInitializing(["schema": 2, "valid": false]))
        XCTAssertFalse(OfflineLanReadEvidence.isInitializing(["schema": 1, "valid": false, "isolation_valid": false]))
    }

    func testDiscoveryScopeMustBeDedicatedAndMatchEffectiveConfiguration() throws {
        XCTAssertEqual(try OfflineLanFixture.validatedScope(scope), scope)
        for invalid in [nil, "", "iris-chat-nearby-v1", scope + "\n", scope + "G",
                        "iris-offline-lan-" + String(repeating: "a", count: 65)] {
            XCTAssertThrowsError(try OfflineLanFixture.validatedScope(invalid))
        }
        let raw = try sample().raw
        XCTAssertThrowsError(try OfflineLanReadEvidence(raw, httpURL: http,
                                                      discoveryScope: "iris-offline-lan-fedcba9876543210"))
        let invalidNetworks: [JsonObject] = [[:], ["lan_scope": scope],
                                            ["lan_scope": scope, "fixture_scope_valid": false]]
        for network in invalidNetworks {
            var changed = raw
            changed["network"] = network
            XCTAssertThrowsError(try OfflineLanReadEvidence(changed, httpURL: http, discoveryScope: scope))
        }
    }

    func testLocalhostUSBCarrierAndWrongPeerCannotCountAsWiFi() throws {
        try sample().verifyPeers(expected: "expected-peer", wifi: wifi, required: true)
        for address in ["127.0.0.1:5000", "169.254.5.2:5000", "192.168.2.20:5000", "192.168.1.10:5000"] {
            XCTAssertThrowsError(try sample(address: address).verifyPeers(expected: "expected-peer", wifi: wifi, required: true))
        }
        XCTAssertThrowsError(try sample(transport: "websocket").verifyPeers(expected: "expected-peer", wifi: wifi, required: true))
        XCTAssertThrowsError(try sample().verifyPeers(expected: "another-peer", wifi: wifi, required: true))
    }

    func testColdReadRequiresWholePayloadAcrossMultiplePeerBlocks() throws {
        let before = try sample()
        let complete = try sample(misses: 6, blocks: 6, bytes: UInt64(OfflineLanFixture.byteCount + 256))
        try before.verifyRead(after: complete, warm: false)
        XCTAssertThrowsError(try before.verifyRead(after: sample(hits: 6), warm: false))
        XCTAssertThrowsError(try before.verifyRead(after: sample(misses: 1, blocks: 1, bytes: 100), warm: false))
        XCTAssertThrowsError(try sample(httpBlocks: 1))
    }

    func testRestartReadCannotBeSatisfiedByNetworkOrChangedEpoch() throws {
        let before = try sample(hits: 2)
        try before.verifyRead(after: sample(hits: 8), warm: true)
        XCTAssertThrowsError(try before.verifyRead(after: sample(hits: 8, blocks: 1, bytes: 1), warm: true))
        XCTAssertThrowsError(try before.verifyRead(after: sample(epoch: "replaced", hits: 8), warm: true))
        XCTAssertThrowsError(try before.verifyRead(after: sample(hits: 1), warm: true))
    }
}
