import Foundation
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

@MainActor
final class SameProcessMeshPeer {
    let id: String
    let user: String
    let manager: AppManager
    let dataDir: URL
    var account: AccountSnapshot?

    init(id: String, user: String, manager: AppManager, dataDir: URL) {
        self.id = id
        self.user = user
        self.manager = manager
        self.dataDir = dataDir
    }
}

extension InteropHarnessTests {
    func runSameProcessMultiDeviceMesh(env: [String: String], rootDataDir: URL) async throws {
        let relays = parseList(env["IRIS_IOS_HARNESS_RELAY_URLS"] ?? "wss://relay.damus.io|wss://nos.lol|wss://relay.primal.net")
            .map(normalizedHarnessRelayURL)
        guard !relays.isEmpty else {
            throw HarnessError.unexpected("same-process mesh requires at least one relay")
        }

        let timeout = harnessTimeout(env: env, defaultSeconds: 120, maximumSeconds: 300)
        let stamp = env["IRIS_IOS_HARNESS_MESH_STAMP"] ?? String(Int(Date().timeIntervalSince1970))
        let meshRoot = rootDataDir.appendingPathComponent("same-process-mesh-\(stamp)", isDirectory: true)
        try? FileManager.default.removeItem(at: meshRoot)
        try FileManager.default.createDirectory(at: meshRoot, withIntermediateDirectories: true)

        status("mesh_data_root", meshRoot.path)
        status("mesh_relays", relays.joined(separator: "|"))

        let alice1 = try await makeSameProcessPeer(id: "alice1", user: "alice", root: meshRoot)
        let alice2 = try await makeSameProcessPeer(id: "alice2", user: "alice", root: meshRoot)
        let bob1 = try await makeSameProcessPeer(id: "bob1", user: "bob", root: meshRoot)
        let bob2 = try await makeSameProcessPeer(id: "bob2", user: "bob", root: meshRoot)
        let peers = [alice1, alice2, bob1, bob2]

        alice1.account = try await createMeshAccount(alice1, name: "Mesh Alice \(stamp)")
        bob1.account = try await createMeshAccount(bob1, name: "Mesh Bob \(stamp)")
        try await setMeshRelays(alice1, relays: relays, timeout: timeout)
        try await setMeshRelays(bob1, relays: relays, timeout: timeout)
        try await waitForMeshConnectedRelay(alice1, timeout: timeout)
        try await waitForMeshConnectedRelay(bob1, timeout: timeout)
        try await drainMesh(alice1, timeout: timeout)
        try await drainMesh(bob1, timeout: timeout)

        try await setMeshRelays(alice2, relays: relays, timeout: timeout)
        try await setMeshRelays(bob2, relays: relays, timeout: timeout)
        alice2.account = try await authorizeLinkedPeer(owner: alice1, linked: alice2, ownerInput: try ownerNpub(alice1), timeout: timeout)
        bob2.account = try await authorizeLinkedPeer(owner: bob1, linked: bob2, ownerInput: try ownerNpub(bob1), timeout: timeout)
        try await waitForMeshConnectedRelay(alice2, timeout: timeout)
        try await waitForMeshConnectedRelay(bob2, timeout: timeout)

        guard try ownerHex(alice1) == ownerHex(alice2) else {
            throw HarnessError.unexpected("Alice linked owner mismatch")
        }
        guard try ownerHex(bob1) == ownerHex(bob2) else {
            throw HarnessError.unexpected("Bob linked owner mismatch")
        }

        let aliceLinkedDirect = "mesh-a2-to-b-\(stamp)"
        let aliceLinkedDirectCounts = try await directMeshSend(
            sender: alice2,
            senderDevices: [alice1, alice2],
            recipientDevices: [bob1, bob2],
            recipientOwnerNpub: try ownerNpub(bob1),
            senderOwnerNpub: try ownerNpub(alice1),
            message: aliceLinkedDirect,
            timeout: timeout
        )

        let bobLinkedDirect = "mesh-b2-to-a-\(stamp)"
        let bobLinkedDirectCounts = try await directMeshSend(
            sender: bob2,
            senderDevices: [bob1, bob2],
            recipientDevices: [alice1, alice2],
            recipientOwnerNpub: try ownerNpub(alice1),
            senderOwnerNpub: try ownerNpub(bob1),
            message: bobLinkedDirect,
            timeout: timeout
        )

        let groupName = "Mesh Group \(stamp)"
        let groupChat = try await createMeshGroup(
            creator: alice1,
            name: groupName,
            memberInputs: [try ownerNpub(bob1)],
            timeout: timeout
        )
        for peer in peers {
            _ = try await waitForMeshGroup(peer, chatID: groupChat.chatId, timeout: timeout)
        }

        let aliceLinkedGroup = "mesh-group-a2-\(stamp)"
        let aliceLinkedGroupCounts = try await groupMeshSend(
            sender: alice2,
            peers: peers,
            chatID: groupChat.chatId,
            message: aliceLinkedGroup,
            timeout: timeout
        )

        let bobLinkedGroup = "mesh-group-b2-\(stamp)"
        let bobLinkedGroupCounts = try await groupMeshSend(
            sender: bob2,
            peers: peers,
            chatID: groupChat.chatId,
            message: bobLinkedGroup,
            timeout: timeout
        )

        for peer in peers {
            try await drainMesh(peer, timeout: timeout)
        }

        status("mesh_status", "passed")
        status("alice_owner_hex", try ownerHex(alice1))
        status("alice_primary_device_hex", try deviceHex(alice1))
        status("alice_linked_owner_hex", try ownerHex(alice2))
        status("alice_linked_device_hex", try deviceHex(alice2))
        status("bob_owner_hex", try ownerHex(bob1))
        status("bob_primary_device_hex", try deviceHex(bob1))
        status("bob_linked_owner_hex", try ownerHex(bob2))
        status("bob_linked_device_hex", try deviceHex(bob2))
        status("direct_alice_linked_to_bob", aliceLinkedDirect)
        status("direct_alice_linked_to_bob_counts", summarizeMeshCounts(aliceLinkedDirectCounts))
        status("direct_bob_linked_to_alice", bobLinkedDirect)
        status("direct_bob_linked_to_alice_counts", summarizeMeshCounts(bobLinkedDirectCounts))
        status("group_chat_id", groupChat.chatId)
        status("group_id", groupChat.groupId ?? "")
        status("group_alice_linked", aliceLinkedGroup)
        status("group_alice_linked_counts", summarizeMeshCounts(aliceLinkedGroupCounts))
        status("group_bob_linked", bobLinkedGroup)
        status("group_bob_linked_counts", summarizeMeshCounts(bobLinkedGroupCounts))
    }

    func makeSameProcessPeer(id: String, user: String, root: URL) async throws -> SameProcessMeshPeer {
        let dataDir = root.appendingPathComponent(id, isDirectory: true)
        try? FileManager.default.removeItem(at: dataDir)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let secretStore = FileAccountSecretStore(
            url: dataDir.appendingPathComponent("account-secret.json"),
            fileManager: .default
        )
        secretStore.clear()
        let environment = [
            "IRIS_UI_TEST_RUN_ID": "same-process-mesh-\(id)",
            "IRIS_UI_TEST_BYPASS_KEYCHAIN": "1",
            "IRIS_DISABLE_NOTIFICATIONS": "1",
        ]
        let manager = AppManager(secretStore: secretStore, dataDir: dataDir, environment: environment)
        _ = try await waitFor(label: "bootstrap \(id)", timeout: 30) {
            manager.bootstrapInFlight ? nil : true
        }
        status("\(id)_data_dir", dataDir.path)
        return SameProcessMeshPeer(id: id, user: user, manager: manager, dataDir: dataDir)
    }

    func createMeshAccount(_ peer: SameProcessMeshPeer, name: String) async throws -> AccountSnapshot {
        peer.manager.dispatch(.createAccount(name: name))
        let account: AccountSnapshot = try await waitFor(label: "\(peer.id) account", timeout: 90) {
            peer.manager.state.account
        }
        status("\(peer.id)_owner_hex", account.publicKeyHex)
        status("\(peer.id)_device_hex", account.devicePublicKeyHex)
        return account
    }

    func authorizeLinkedPeer(
        owner: SameProcessMeshPeer,
        linked: SameProcessMeshPeer,
        ownerInput: String,
        timeout: TimeInterval
    ) async throws -> AccountSnapshot {
        linked.manager.startLinkedDevice(ownerInput: ownerInput)
        let link: LinkDeviceSnapshot = try await waitFor(label: "\(linked.id) link invite", timeout: timeout) {
            linked.manager.state.linkDevice
        }
        status("\(linked.id)_link_url", link.url)
        status("\(linked.id)_device_input", link.deviceInput)
        statusMeshPeerSnapshot(linked, prefix: "link_invite")

        let deviceHex = peerInputToHex(input: link.deviceInput)
        owner.manager.addAuthorizedDevice(deviceInput: link.url)
        _ = try await waitFor(label: "\(owner.id) roster contains \(linked.id)", timeout: min(timeout, 90)) {
            self.meshRosterContainsDevice(owner, deviceHex: deviceHex) ? true : nil
        }
        status("\(owner.id)_roster_after_authorize", meshRosterSummary(owner))
        try await drainMesh(owner, timeout: timeout)
        statusMeshPeerSnapshot(owner, prefix: "after_authorize_drain")
        statusMeshPeerSnapshot(linked, prefix: "after_owner_drain")
        let linkedAccount = try await waitForLinkedAuthorization(linked, timeout: min(timeout, 120))
        try await drainMesh(linked, timeout: timeout)
        status("\(linked.id)_owner_hex", linkedAccount.publicKeyHex)
        status("\(linked.id)_device_hex", linkedAccount.devicePublicKeyHex)
        return linkedAccount
    }

    func setMeshRelays(_ peer: SameProcessMeshPeer, relays: [String], timeout: TimeInterval) async throws {
        peer.manager.dispatch(.setNostrRelays(relayUrls: relays))
        _ = try await waitFor(label: "\(peer.id) relays", timeout: timeout) {
            peer.manager.state.preferences.nostrRelayUrls == relays ? true : nil
        }
        status("\(peer.id)_relays", peer.manager.state.preferences.nostrRelayUrls.joined(separator: "|"))
    }

    func waitForMeshConnectedRelay(_ peer: SameProcessMeshPeer, timeout: TimeInterval) async throws {
        let connected = try await waitFor(label: "\(peer.id) connected relay", timeout: timeout) {
            peer.manager.state.networkStatus?.connectedRelayCount ?? 0 > 0
                ? peer.manager.state.networkStatus?.connectedRelayCount
                : nil
        }
        status("\(peer.id)_connected_relay_count", String(connected))
    }

    func waitForLinkedAuthorization(_ peer: SameProcessMeshPeer, timeout: TimeInterval) async throws -> AccountSnapshot {
        let deadline = Date().addingTimeInterval(timeout)
        var nextReport = Date()
        while Date() < deadline {
            if let account = peer.manager.state.account {
                let state = String(describing: account.authorizationState).lowercased()
                if state == "authorized" {
                    return account
                }
            }
            if Date() >= nextReport {
                statusMeshPeerSnapshot(peer, prefix: "waiting_authorization")
                nextReport = Date().addingTimeInterval(15)
            }
            try await Task.sleep(nanoseconds: 500_000_000)
        }
        statusMeshPeerSnapshot(peer, prefix: "authorization_timeout")
        throw HarnessError.timeout("\(peer.id) linked-device authorization; \(meshAccountSummary(peer.manager.state.account)); \(meshNetworkSummary(peer))")
    }

    func drainMesh(_ peer: SameProcessMeshPeer, timeout: TimeInterval) async throws {
        try await waitForRelayDrainIfRequested(
            manager: peer.manager,
            dataDir: peer.dataDir,
            env: [
                "IRIS_IOS_HARNESS_WAIT_FOR_RELAY_DRAIN": "true",
                "IRIS_IOS_HARNESS_RELAY_DRAIN_TIMEOUT_SECS": String(Int(timeout)),
                "IRIS_IOS_HARNESS_RELAY_DRAIN_RUNTIME_ONLY": "true",
            ]
        )
    }

    func directMeshSend(
        sender: SameProcessMeshPeer,
        senderDevices: [SameProcessMeshPeer],
        recipientDevices: [SameProcessMeshPeer],
        recipientOwnerNpub: String,
        senderOwnerNpub: String,
        message: String,
        timeout: TimeInterval
    ) async throws -> [String: Int] {
        let chatID = try await ensureChatOpen(
            manager: sender.manager,
            dataDir: sender.dataDir,
            chatID: nil,
            peerInput: recipientOwnerNpub
        )
        sender.manager.dispatch(.sendMessage(chatId: chatID, text: message))
        try await waitForMeshDelivery(sender, chatID: chatID, message: message, timeout: timeout)
        try await drainMesh(sender, timeout: timeout)

        var counts: [String: Int] = [:]
        for peer in senderDevices {
            counts[peer.id] = try await waitForMeshMessage(
                peer,
                chatID: nil,
                peerInput: recipientOwnerNpub,
                message: message,
                direction: "outgoing",
                timeout: timeout
            )
        }
        for peer in recipientDevices {
            counts[peer.id] = try await waitForMeshMessage(
                peer,
                chatID: nil,
                peerInput: senderOwnerNpub,
                message: message,
                direction: "incoming",
                timeout: timeout
            )
        }
        return counts
    }

    func createMeshGroup(
        creator: SameProcessMeshPeer,
        name: String,
        memberInputs: [String],
        timeout: TimeInterval
    ) async throws -> CurrentChatSnapshot {
        creator.manager.dispatch(.createGroup(name: name, memberInputs: memberInputs))
        let chat: CurrentChatSnapshot = try await waitFor(label: "group \(name)", timeout: timeout) {
            creator.manager.state.currentChat.flatMap { current in
                current.groupId != nil && current.displayName == name ? current : nil
            }
        }
        try await drainMesh(creator, timeout: timeout)
        return chat
    }

    func waitForMeshGroup(
        _ peer: SameProcessMeshPeer,
        chatID: String,
        timeout: TimeInterval
    ) async throws -> CurrentChatSnapshot {
        _ = try await waitFor(label: "\(peer.id) group \(chatID)", timeout: timeout) {
            peer.manager.state.chatList.first(where: { self.sameIdentifier($0.chatId, chatID) })
        }
        peer.manager.dispatch(.openChat(chatId: chatID))
        return try await waitFor(label: "\(peer.id) open group \(chatID)", timeout: 30) {
            peer.manager.state.currentChat.flatMap { current in
                self.sameIdentifier(current.chatId, chatID) ? current : nil
            }
        }
    }

    func groupMeshSend(
        sender: SameProcessMeshPeer,
        peers: [SameProcessMeshPeer],
        chatID: String,
        message: String,
        timeout: TimeInterval
    ) async throws -> [String: Int] {
        _ = try await waitForMeshGroup(sender, chatID: chatID, timeout: timeout)
        sender.manager.dispatch(.sendMessage(chatId: chatID, text: message))
        try await waitForMeshDelivery(sender, chatID: chatID, message: message, timeout: timeout)
        try await drainMesh(sender, timeout: timeout)
        var counts: [String: Int] = [:]
        for peer in peers {
            let direction = peer.user == sender.user ? "outgoing" : "incoming"
            counts[peer.id] = try await waitForMeshMessage(
                peer,
                chatID: chatID,
                peerInput: nil,
                message: message,
                direction: direction,
                timeout: timeout
            )
        }
        return counts
    }

    func waitForMeshDelivery(
        _ peer: SameProcessMeshPeer,
        chatID: String,
        message: String,
        timeout: TimeInterval
    ) async throws {
        let delivery: String = try await waitFor(label: "\(peer.id) delivery \(message)", timeout: timeout) {
            if let current = peer.manager.state.currentChat,
               self.sameIdentifier(current.chatId, chatID),
               let entry = current.messages.first(where: { $0.isOutgoing && $0.body == message }) {
                let value = String(describing: entry.delivery)
                if value.caseInsensitiveCompare("queued") != .orderedSame &&
                    value.caseInsensitiveCompare("pending") != .orderedSame {
                    return value
                }
            }
            return self.splitPersistenceMessageDelivery(
                dataDir: peer.dataDir,
                chatID: chatID,
                message: message,
                direction: "outgoing"
            )
        }
        if delivery.caseInsensitiveCompare("failed") == .orderedSame {
            throw HarnessError.unexpected("\(peer.id) failed to publish \(message)")
        }
    }

    func waitForMeshMessage(
        _ peer: SameProcessMeshPeer,
        chatID: String?,
        peerInput: String?,
        message: String,
        direction: String,
        timeout: TimeInterval
    ) async throws -> Int {
        let resolvedChatID = try await ensureChatOpen(
            manager: peer.manager,
            dataDir: peer.dataDir,
            chatID: chatID,
            peerInput: peerInput
        )
        let count = try await waitFor(label: "\(peer.id) \(direction) \(message)", timeout: timeout) {
            let current = self.countMessages(
                manager: peer.manager,
                dataDir: peer.dataDir,
                chatID: resolvedChatID,
                message: message,
                direction: direction,
                peerInput: peerInput
            )
            return current > 0 ? current : nil
        }
        try await Task.sleep(nanoseconds: 2_000_000_000)
        let finalCount = countMessages(
            manager: peer.manager,
            dataDir: peer.dataDir,
            chatID: resolvedChatID,
            message: message,
            direction: direction,
            peerInput: peerInput
        )
        guard finalCount == count else {
            throw HarnessError.unexpected("\(peer.id) duplicate drift for \(message): \(count) -> \(finalCount)")
        }
        return finalCount
    }

    func ownerNpub(_ peer: SameProcessMeshPeer) throws -> String {
        guard let npub = peer.account?.npub, !npub.isEmpty else {
            throw HarnessError.unexpected("missing user ID for \(peer.id)")
        }
        return npub
    }

    func ownerHex(_ peer: SameProcessMeshPeer) throws -> String {
        guard let hex = peer.account?.publicKeyHex, !hex.isEmpty else {
            throw HarnessError.unexpected("missing owner hex for \(peer.id)")
        }
        return hex
    }

    func deviceHex(_ peer: SameProcessMeshPeer) throws -> String {
        guard let hex = peer.account?.devicePublicKeyHex, !hex.isEmpty else {
            throw HarnessError.unexpected("missing device hex for \(peer.id)")
        }
        return hex
    }

    func summarizeMeshCounts(_ counts: [String: Int]) -> String {
        counts.keys.sorted().map { "\($0)=\(counts[$0] ?? 0)" }.joined(separator: ",")
    }

    func statusMeshPeerSnapshot(_ peer: SameProcessMeshPeer, prefix: String) {
        status("\(peer.id)_\(prefix)_account", meshAccountSummary(peer.manager.state.account))
        status("\(peer.id)_\(prefix)_network", meshNetworkSummary(peer))
        status("\(peer.id)_\(prefix)_roster", meshRosterSummary(peer))
        status("\(peer.id)_\(prefix)_pending_durable", sqlitePendingRelayPublishCount(dataDir: peer.dataDir).map(String.init) ?? "unavailable")
        status("\(peer.id)_\(prefix)_toast", peer.manager.state.toast ?? "")
    }

    func meshAccountSummary(_ account: AccountSnapshot?) -> String {
        guard let account else {
            return "none"
        }
        return [
            "owner=\(account.publicKeyHex)",
            "device=\(account.devicePublicKeyHex)",
            "authorization=\(String(describing: account.authorizationState))",
        ].joined(separator: ",")
    }

    func meshNetworkSummary(_ peer: SameProcessMeshPeer) -> String {
        guard let network = peer.manager.state.networkStatus else {
            return "none"
        }
        return [
            "connected=\(network.connectedRelayCount)",
            "pending=\(network.pendingOutboundCount)",
            "pending_group=\(network.pendingGroupControlCount)",
            "syncing=\(network.syncing)",
            "relays=\(network.relayUrls.joined(separator: "|"))",
        ].joined(separator: ",")
    }

    func meshRosterSummary(_ peer: SameProcessMeshPeer) -> String {
        guard let roster = peer.manager.state.deviceRoster else {
            return "none"
        }
        let devices = roster.devices.map { device in
            [
                String(device.devicePubkeyHex.prefix(12)),
                "authorized=\(device.isAuthorized)",
                "current=\(device.isCurrentDevice)",
                "stale=\(device.isStale)",
            ].joined(separator: ":")
        }.joined(separator: "|")
        return [
            "owner=\(roster.ownerPublicKeyHex)",
            "current=\(roster.currentDevicePublicKeyHex)",
            "devices=\(devices)",
        ].joined(separator: ",")
    }

    func meshRosterContainsDevice(_ peer: SameProcessMeshPeer, deviceHex: String) -> Bool {
        guard !deviceHex.isEmpty else {
            return false
        }
        return peer.manager.state.deviceRoster?.devices.contains {
            sameIdentifier($0.devicePubkeyHex, deviceHex) && $0.isAuthorized && !$0.isStale
        } ?? false
    }
}
