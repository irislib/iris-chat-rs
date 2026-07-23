import Foundation
import SQLite3
import UserNotifications
import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

extension InteropHarnessTests {
    func reportIdentity(_ snapshot: AccountSnapshot) {
        status("npub", snapshot.npub)
        status("public_key_hex", snapshot.publicKeyHex)
        status("device_npub", snapshot.deviceNpub)
        status("device_public_key_hex", snapshot.devicePublicKeyHex)
        status("authorization_state", String(describing: snapshot.authorizationState))
    }

    func reportDeviceRoster(_ roster: DeviceRosterSnapshot) {
        status("owner_npub", roster.ownerNpub)
        status("current_device_npub", roster.currentDeviceNpub)
        status("authorization_state", String(describing: roster.authorizationState))
        status("can_manage_devices", String(roster.canManageDevices))
        status("device_count", String(roster.devices.count))
        status("devices", roster.devices.map { device in
            [
                device.devicePubkeyHex,
                device.deviceNpub,
                String(device.isCurrentDevice),
                String(device.isAuthorized),
                String(device.isStale),
            ].joined(separator: ",")
        }.joined(separator: "|"))
    }

    func reportNearbySnapshot(manager: AppManager) {
        let peers = manager.nearbyIris.peers.map { peer in
            [
                "id": peer.id,
                "name": peer.name,
                "owner_pubkey_hex": peer.ownerPubkeyHex ?? "",
                "profile_event_id": peer.profileEventID ?? "",
            ]
        }
        let peersData = try? JSONSerialization.data(withJSONObject: peers)
        let peersJson = peersData.flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        status("nearby_visible", String(manager.nearbyIris.isVisible))
        status("nearby_status", manager.nearbyIris.status)
        status("nearby_lan_visible", String(manager.nearbyIris.isLanVisible))
        status("nearby_lan_status", manager.nearbyIris.lanStatus)
        status("nearby_peer_count", String(manager.nearbyIris.peers.count))
        status("nearby_peers", peersJson)
    }

    func reportRuntimeDebugSnapshot(manager: AppManager, dataDir: URL, liveDebugOverride: JsonObject? = nil) async {
        let state = manager.state
        let fileDebug = readJsonObject(at: dataDir.appendingPathComponent(debugSnapshotFilename))
        let liveDebug: JsonObject?
        if let liveDebugOverride {
            liveDebug = liveDebugOverride
        } else {
            liveDebug = await liveRuntimeDebugSnapshot(manager: manager)
        }
        let debug = liveDebug ?? fileDebug
        let plan = dictValue(debug?["current_protocol_plan"]) ?? dictValue(debug?["protocol"])
        let protocolEngine = dictValue(debug?["protocol_engine"])
        let pendingProtocolOutbound = joinValues(arrayValue(protocolEngine?["pending_outbound_targets"]))
        let pendingGroupFanouts = joinValues(arrayValue(protocolEngine?["pending_group_fanout_targets"]))
        let legacyPendingOutbound = summarizeRuntimePendingOutbound(arrayValue(debug?["pending_outbound"]))
        let networkStatus = state.networkStatus
        let liveSource = liveDebug == nil ? "file" : "live"
        let ownerHex = stringValue(debug?["local_owner_pubkey_hex"])
        let deviceHex = stringValue(debug?["local_device_pubkey_hex"])

        status("data_dir", dataDir.path)
        status("rev", String(state.rev))
        status("default_screen", String(describing: state.router.defaultScreen))
        status("screen_stack", state.router.screenStack.map { String(describing: $0) }.joined(separator: "|"))
        status("current_chat", summarizeCurrentChat(state.currentChat))
        status("chat_list", summarizeChatList(state.chatList))
        status("toast", state.toast ?? "")
        status("runtime_file_present", fileDebug == nil ? "false" : "true")
        status("runtime_live_snapshot_present", liveDebug == nil ? "false" : "true")
        status("runtime_snapshot_source", debug == nil ? "none" : liveSource)
        status("runtime_support_bundle_timed_out", String(boolValue(dictValue(liveDebug?["ffi_queue"])?["core_support_bundle_timed_out"])))
        status("generated_at_secs", stringValue(debug?["generated_at_secs"]))
        status("local_owner_pubkey_hex", ownerHex.isEmpty ? (state.account?.publicKeyHex ?? "") : ownerHex)
        status("local_device_pubkey_hex", deviceHex.isEmpty ? (state.account?.devicePublicKeyHex ?? "") : deviceHex)
        status("authorization_state", stringValue(debug?["authorization_state"]))
        status("tracked_owner_hexes", joinValues(arrayValue(debug?["tracked_owner_hexes"])))
        status("plan_roster_authors", joinValues(arrayValue(plan?["roster_authors"])))
        status("plan_invite_authors", joinValues(arrayValue(plan?["invite_authors"])))
        status("plan_message_authors", joinValues(arrayValue(plan?["message_authors"])))
        status("plan_invite_response_recipient", stringValue(plan?["invite_response_recipient"]))
        status("known_users", summarizeRuntimeKnownUsers(arrayValue(debug?["known_users"])))
        status("pending_protocol_outbound_count", stringValue(protocolEngine?["pending_outbound_count"]))
        status("pending_protocol_outbound", pendingProtocolOutbound)
        status("pending_group_fanout_count", stringValue(protocolEngine?["pending_group_fanout_count"]))
        status("pending_group_fanouts", pendingGroupFanouts)
        status("pending_group_sender_key_message_count", stringValue(protocolEngine?["pending_group_sender_key_message_count"]))
        status("pending_group_sender_key_retry_count", stringValue(protocolEngine?["pending_group_sender_key_retry_count"]))
        status("pending_group_sender_key_unmapped_count", stringValue(protocolEngine?["pending_group_sender_key_unmapped_count"]))
        status("pending_group_sender_key_repair_count", stringValue(protocolEngine?["pending_group_sender_key_repair_count"]))
        status("pending_group_sender_key_repair_next_retry_at_secs", stringValue(protocolEngine?["pending_group_sender_key_repair_next_retry_at_secs"]))
        status("pending_group_sender_key_repair_max_request_count", stringValue(protocolEngine?["pending_group_sender_key_repair_max_request_count"]))
        status("pending_outbound", legacyPendingOutbound.isEmpty ? pendingProtocolOutbound : legacyPendingOutbound)
        status("pending_relay_publishes", summarizeRuntimePendingRelayPublishes(arrayValue(debug?["pending_relay_publishes"])))
        status("pending_group_controls", summarizeRuntimePendingGroupControls(arrayValue(debug?["pending_group_controls"])))
        status("recent_handshake_peers", summarizeRecentHandshakePeers(arrayValue(debug?["recent_handshake_peers"])))
        status("event_counts", summarizeEventCounts(dictValue(debug?["event_counts"])))
        status("recent_log", summarizeRecentLog(arrayValue(debug?["recent_log"])))
        status("network_connected_relay_count", String(networkStatus?.connectedRelayCount ?? 0))
        status("network_all_relays_offline_since_secs", String(networkStatus?.allRelaysOfflineSinceSecs ?? 0))
        status("network_relay_urls", networkStatus?.relayUrls.joined(separator: ",") ?? "")
        status("network_relay_connections", summarizeRelayConnections(networkStatus?.relayConnections ?? []))
    }

    func liveRuntimeDebugSnapshot(manager: AppManager) async -> JsonObject? {
        let rawJson = await manager.supportBundleJsonAsync()
        guard let raw = try? jsonObjectFromString(rawJson) else {
            return nil
        }
        return runtimeDebugObject(from: raw)
    }

    func runtimeDebugObject(from object: JsonObject) -> JsonObject {
        if let runtime = dictValue(object["runtime_debug"]) ?? dictValue(object["runtime"]) {
            return runtime
        }
        if let rust = dictValue(object["rust"]),
           let runtime = dictValue(rust["runtime_debug"]) ?? dictValue(rust["runtime"]) {
            return runtime
        }
        return object
    }

    func runtimeSnapshotIsIdle(_ debug: JsonObject) -> Bool {
        if boolValue(dictValue(debug["ffi_queue"])?["core_support_bundle_timed_out"]) {
            return false
        }
        if !arrayValue(debug["pending_relay_publishes"]).isEmpty {
            return false
        }
        guard let protocolEngine = dictValue(debug["protocol_engine"]) else {
            return true
        }
        return protocolEnginePendingCount(protocolEngine) == 0
    }

    func runtimePendingSummary(_ debug: JsonObject) -> String {
        let protocolEngine = dictValue(debug["protocol_engine"])
        let protocolCount = protocolEngine.map(protocolEnginePendingCount) ?? 0
        let protocolSummary = protocolEngine.map(summarizeProtocolEnginePending) ?? ""
        let pendingRelayPublishes = arrayValue(debug["pending_relay_publishes"])
        let relayPublishes = summarizeRuntimePendingRelayPublishes(pendingRelayPublishes)
        let timedOut = boolValue(dictValue(debug["ffi_queue"])?["core_support_bundle_timed_out"])
        return [
            "protocol=\(protocolCount)",
            protocolSummary.isEmpty ? "" : "protocolPending=\(protocolSummary)",
            "relay=\(pendingRelayPublishes.count)",
            "ffiTimedOut=\(timedOut)",
            relayPublishes.isEmpty ? "" : "pending=\(relayPublishes)",
        ]
        .filter { !$0.isEmpty }
        .joined(separator: " ")
    }

    func summarizeProtocolEnginePending(_ protocolEngine: JsonObject) -> String {
        let senderMessageCountKey = protocolEngine.keys.contains("pending_group_sender_key_retry_count")
            ? "pending_group_sender_key_retry_count"
            : "pending_group_sender_key_message_count"
        let countKeys = [
            ("out", "pending_outbound_count"),
            ("in", "pending_inbound_count"),
            ("fanout", "pending_group_fanout_count"),
            ("pairwise", "pending_group_pairwise_payload_count"),
            ("senderMsg", senderMessageCountKey),
            ("senderRepair", "pending_group_sender_key_repair_count"),
        ]
        let counts = countKeys.compactMap { label, key -> String? in
            let count = intValue(protocolEngine[key])
            return count == 0 ? nil : "\(label)=\(count)"
        }.joined(separator: ",")
        let senderUnmapped = intValue(protocolEngine["pending_group_sender_key_unmapped_count"])
        let senderRepairNext = intValue(protocolEngine["pending_group_sender_key_repair_next_retry_at_secs"])
        let senderRepairRequests = intValue(protocolEngine["pending_group_sender_key_repair_max_request_count"])
        let outboundTargets = joinValues(arrayValue(protocolEngine["pending_outbound_targets"]), limit: 6)
        let fanoutTargets = joinValues(arrayValue(protocolEngine["pending_group_fanout_targets"]), limit: 6)
        let outboundDetails = joinObjects(arrayValue(protocolEngine["pending_outbound_details"]), limit: 3) { entry in
            [
                stringValue(entry["reason"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["recipient_owner_hex"]),
            ]
            .filter { !$0.isEmpty }
            .joined(separator: ",")
        }
        return [
            counts.isEmpty ? "" : "counts=\(counts)",
            senderUnmapped == 0 ? "" : "senderUnmapped=\(senderUnmapped)",
            senderRepairNext == 0 ? "" : "senderRepairNext=\(senderRepairNext)",
            senderRepairRequests == 0 ? "" : "senderRepairRequests=\(senderRepairRequests)",
            outboundTargets.isEmpty ? "" : "outboundTargets=\(outboundTargets)",
            outboundDetails.isEmpty ? "" : "outboundDetails=\(outboundDetails)",
            fanoutTargets.isEmpty ? "" : "fanoutTargets=\(fanoutTargets)",
        ]
        .filter { !$0.isEmpty }
        .joined(separator: " ")
    }

    func protocolEnginePendingCount(_ protocolEngine: JsonObject) -> Int {
        let senderMessageCount = protocolEngine.keys.contains("pending_group_sender_key_retry_count")
            ? intValue(protocolEngine["pending_group_sender_key_retry_count"])
            : intValue(protocolEngine["pending_group_sender_key_message_count"])
        return senderMessageCount + [
            "pending_outbound_count",
            "pending_inbound_count",
            "pending_group_fanout_count",
            "pending_group_pairwise_payload_count",
            "pending_group_sender_key_repair_count",
        ].reduce(0) { total, key in
            total + intValue(protocolEngine[key])
        }
    }

    func summarizeRelayConnections(_ connections: [RelayConnectionSnapshot]) -> String {
        connections
            .map { "\($0.url)=\($0.status)" }
            .joined(separator: ",")
    }

    func normalizedHarnessRelayURL(_ relayURL: String) -> String {
        if relayURL.hasPrefix("wss:/"), !relayURL.hasPrefix("wss://") {
            return "wss://" + relayURL.dropFirst("wss:/".count)
        }
        if relayURL.hasPrefix("ws:/"), !relayURL.hasPrefix("ws://") {
            return "ws://" + relayURL.dropFirst("ws:/".count)
        }
        return relayURL
    }

    func reportPersistedProtocolSnapshot(dataDir: URL) {
        let sqlite = readSqliteCoreSnapshot(dataDir: dataDir)
        let meta = readJsonObject(at: dataDir.appendingPathComponent("core/meta.json"))
        let appKeys = readJsonArray(at: dataDir.appendingPathComponent("core/app_keys.json"))
        let groups = readJsonArray(at: dataDir.appendingPathComponent("core/groups.json"))
        let seenEvents = readJsonObject(at: dataDir.appendingPathComponent("core/seen_events.json"))
        let threads = splitPersistenceThreadFiles(dataDir: dataDir).compactMap { readJsonObject(at: $0) }

        status("data_dir", dataDir.path)
        status("sqlite_file_present", String(sqlite.filePresent))
        status("sqlite_app_meta", sqlite.appMeta)
        status("sqlite_app_keys", sqlite.appKeys)
        status("sqlite_groups", sqlite.groups)
        status("sqlite_threads", sqlite.threads)
        status("sqlite_messages", sqlite.messages)
        status("sqlite_pending_relay_publishes", sqlite.pendingRelayPublishes)
        status("persisted_file_present", meta == nil ? "false" : "true")
        status("version", stringValue(meta?["version"]))
        status("active_chat_id", stringValue(meta?["active_chat_id"]))
        status("authorization_state", stringValue(meta?["authorization_state"]))
        status("app_keys", summarizePersistedAppKeys(appKeys))
        status("groups", summarizePersistedGroups(groups))
        status("seen_event_ids_count", String(arrayValue(seenEvents?["seen_event_ids"]).count))
        status("threads", summarizePersistedThreads(threads))
    }

    func reportMobilePushSnapshot(manager: AppManager) {
        let snapshot = manager.state.mobilePush
        status("owner_pubkey_hex", snapshot.ownerPubkeyHex ?? "")
        status("message_author_pubkeys", snapshot.messageAuthorPubkeys.joined(separator: ","))
        status("sessions", snapshot.sessions.map { session in
            [
                session.recipientPubkeyHex,
                session.displayName,
                session.trackedSenderPubkeys.joined(separator: "+"),
                "receiving=\(session.hasReceivingCapability)",
            ].joined(separator: ",")
        }.joined(separator: "|"))
    }

    func decryptNotificationPayloadFromArgs(
        secretStore: AccountSecretStore,
        dataDir: URL,
        env: [String: String]
    ) throws {
        let outerEventJson = try requiredEnv("IRIS_IOS_HARNESS_OUTER_EVENT_JSON", env: env)
        let expectedBody = env["IRIS_IOS_HARNESS_EXPECTED_BODY"] ?? ""
        let expectedTitle = env["IRIS_IOS_HARNESS_EXPECTED_TITLE"] ?? ""
        let eventObject = try jsonObjectFromString(outerEventJson)
        let payloadObject: JsonObject = [
            "aps": [
                "alert": [
                    "title": "Iris Chat",
                    "body": "New message",
                ],
                "mutable-content": 1,
            ],
            "event": eventObject,
            "title": "New message",
            "body": "New message",
        ]
        let payloadData = try JSONSerialization.data(withJSONObject: payloadObject)
        guard let payloadJson = String(data: payloadData, encoding: .utf8) else {
            throw HarnessError.unexpected("could not encode notification payload")
        }
        guard let bundle = secretStore.load() else {
            throw HarnessError.unexpected("stored account bundle unavailable")
        }

        let resolution = decryptMobilePushNotificationPayload(
            dataDir: dataDir.path,
            ownerPubkeyHex: bundle.ownerPubkeyHex,
            deviceNsec: bundle.deviceNsec,
            rawPayloadJson: payloadJson
        )
        status("notification_should_show", String(resolution.shouldShow))
        status("notification_title", resolution.title)
        status("notification_body", resolution.body)
        status("notification_payload_json", resolution.payloadJson)
        guard resolution.shouldShow else {
            throw HarnessError.unexpected("decrypted notification was suppressed")
        }
        if !expectedBody.isEmpty && resolution.body != expectedBody {
            throw HarnessError.unexpected("notification body `\(resolution.body)` != `\(expectedBody)`")
        }
        if !expectedTitle.isEmpty && resolution.title != expectedTitle {
            throw HarnessError.unexpected("notification title `\(resolution.title)` != `\(expectedTitle)`")
        }
    }

    func reportMobilePushServerSnapshot(manager: AppManager) async throws {
        guard let ownerNsec = manager.exportOwnerNsec() else {
            throw HarnessError.unexpected("owner nsec unavailable")
        }
        let request = buildMobilePushListSubscriptionsRequest(
            ownerNsec: ownerNsec,
            platformKey: "ios",
            isRelease: false,
            serverUrlOverride: ProcessInfo.processInfo.environment["IRIS_NOTIFICATION_SERVER_URL"]
        )
        guard let request else {
            throw HarnessError.unexpected("could not build mobile push list request")
        }
        guard let url = URL(string: request.url) else {
            throw HarnessError.unexpected("invalid mobile push url")
        }
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method
        urlRequest.setValue("application/json", forHTTPHeaderField: "accept")
        urlRequest.setValue(request.authorizationHeader, forHTTPHeaderField: "authorization")
        let expectedAuthors = Set(manager.state.mobilePush.messageAuthorPubkeys)
        guard !expectedAuthors.isEmpty else {
            throw HarnessError.unexpected("mobile push sender keys unavailable")
        }
        let currentToken: String?
        if let token = MobilePushTokenCenter.shared.currentApnsToken() {
            currentToken = token
        } else {
            currentToken = await MobilePushTokenCenter.shared.waitForApnsToken(
                timeoutNanoseconds: 15_000_000_000
            )
        }
        guard let currentToken else {
            throw HarnessError.unexpected("APNs token unavailable")
        }
        let deadline = Date().addingTimeInterval(30)
        var object: JsonObject = [:]
        var statusCode = 0
        var currentAuthorMatches = 0
        repeat {
            let (data, response) = try await URLSession.shared.data(for: urlRequest)
            statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
            object = (try? JSONSerialization.jsonObject(with: data) as? JsonObject) ?? [:]
            currentAuthorMatches = object.values.filter { value in
                let subscription = dictValue(value)
                let tokens = arrayValue(subscription?["apns_tokens"]).compactMap { $0 as? String }
                let filters = [subscription?["filter"]].compactMap { $0 } + arrayValue(subscription?["filters"])
                let authors = filters.flatMap { arrayValue(dictValue($0)?["authors"]) }.compactMap { $0 as? String }
                return tokens.contains(currentToken) && expectedAuthors.isSubset(of: Set(authors))
            }.count
            if currentAuthorMatches > 0 { break }
            try await Task.sleep(nanoseconds: 500_000_000)
        } while Date() < deadline
        status("status_code", String(statusCode))
        status("subscription_count", String(object.count))
        status("subscriptions", summarizeMobilePushServerSubscriptions(object))
        status("current_apns_author", String(currentAuthorMatches))
        guard currentAuthorMatches > 0 else {
            throw HarnessError.unexpected(
                "notifications.iris.to did not match this iPhone's APNs token and sender keys"
            )
        }
    }

    func summarizeCurrentChat(_ chat: CurrentChatSnapshot?) -> String {
        guard let chat else { return "" }
        return [
            chat.chatId,
            chat.displayName,
            chat.groupId ?? "",
            String(chat.memberCount),
            String(chat.messages.count),
        ].joined(separator: ",")
    }

    func summarizeChatList(_ threads: [ChatThreadSnapshot]) -> String {
        threads.map { thread in
            [
                thread.chatId,
                String(describing: thread.kind),
                thread.displayName,
                String(thread.memberCount),
                thread.lastMessagePreview ?? "",
                String(thread.unreadCount),
            ].joined(separator: ",")
        }.joined(separator: "|")
    }

    func summarizeMobilePushServerSubscriptions(_ subscriptions: JsonObject) -> String {
        subscriptions.map { id, value in
            let subscription = dictValue(value)
            let filter = dictValue(subscription?["filter"])
            let authors = arrayValue(filter?["authors"])
            let fcmTokens = arrayValue(subscription?["fcm_tokens"])
            let apnsTokens = arrayValue(subscription?["apns_tokens"])
            return [
                id,
                "authors=\(authors.count)",
                "fcm=\(fcmTokens.count)",
                "apns=\(apnsTokens.count)",
            ].joined(separator: ",")
        }
        .sorted()
        .joined(separator: "|")
    }

    func summarizeRuntimeKnownUsers(_ users: JsonArray) -> String {
        joinObjects(users) { user in
            [
                stringValue(user["owner_pubkey_hex"]),
                "roster=\(boolValue(user["has_roster"]))",
                "rosterDevices=\(intValue(user["roster_device_count"]))",
                "devices=\(intValue(user["device_count"]))",
                "authorized=\(intValue(user["authorized_device_count"]))",
                "active=\(intValue(user["active_session_device_count"]))",
                "inactive=\(intValue(user["inactive_session_count"]))",
            ].joined(separator: ",")
        }
    }

    func summarizeRuntimePendingOutbound(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["message_id"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["publish_mode"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    func summarizeRuntimePendingRelayPublishes(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["event_id"]),
                stringValue(entry["label"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["inner_event_id"]),
                "attempts=\(intValue(entry["attempt_count"]))",
                "error=\(stringValue(entry["last_error"]))",
            ].joined(separator: ",")
        }
    }

    func summarizeRuntimePendingGroupControls(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["operation_id"]),
                stringValue(entry["group_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["kind"]),
                "targets=\(joinValues(arrayValue(entry["target_owner_hexes"])))",
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    func summarizeRecentHandshakePeers(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["owner_hex"]),
                stringValue(entry["device_hex"]),
                stringValue(entry["observed_at_secs"]),
            ].joined(separator: ",")
        }
    }

    func summarizeEventCounts(_ eventCounts: JsonObject?) -> String {
        guard let eventCounts else { return "" }
        return [
            "roster=\(intValue(eventCounts["roster_events"]))",
            "invite=\(intValue(eventCounts["invite_events"]))",
            "inviteResponse=\(intValue(eventCounts["invite_response_events"]))",
            "message=\(intValue(eventCounts["message_events"]))",
            "other=\(intValue(eventCounts["other_events"]))",
        ].joined(separator: ",")
    }

    func summarizeRecentLog(_ entries: JsonArray) -> String {
        joinObjects(entries, limit: 20) { entry in
            [
                stringValue(entry["timestamp_secs"]),
                stringValue(entry["category"]),
                stringValue(entry["detail"]),
            ].joined(separator: ",")
        }
    }

    func summarizePersistedUsers(_ users: JsonArray) -> String {
        joinObjects(users) { user in
            let devices = arrayValue(user["devices"])
            let activeSessions = devices.reduce(into: 0) { count, entry in
                guard let device = dictValue(entry) else { return }
                if dictValue(device["active_session"]) != nil {
                    count += 1
                }
            }
            let inactiveSessions = devices.reduce(into: 0) { count, entry in
                guard let device = dictValue(entry) else { return }
                count += arrayValue(device["inactive_sessions"]).count
            }
            return [
                stringValue(user["owner_pubkey"]),
                "roster=\(dictValue(user["roster"]) != nil)",
                "devices=\(devices.count)",
                "active=\(activeSessions)",
                "inactive=\(inactiveSessions)",
            ].joined(separator: ",")
        }
    }

    func summarizePersistedGroups(_ groups: JsonArray) -> String {
        joinObjects(groups) { group in
            [
                stringValue(group["group_id"]),
                stringValue(group["name"]),
                "revision=\(intValue(group["revision"]))",
                "members=\(arrayValue(group["members"]).count)",
                "admins=\(arrayValue(group["admins"]).count)",
            ].joined(separator: ",")
        }
    }

    func summarizePersistedAppKeys(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["owner_pubkey_hex"]),
                "devices=\(arrayValue(entry["devices"]).count)",
            ].joined(separator: ",")
        }
    }

    func summarizePersistedPendingOutbound(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["message_id"]),
                stringValue(entry["chat_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["publish_mode"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    func summarizePersistedPendingGroupControls(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["operation_id"]),
                stringValue(entry["group_id"]),
                stringValue(entry["reason"]),
                stringValue(entry["kind"]),
                "inFlight=\(boolValue(entry["in_flight"]))",
            ].joined(separator: ",")
        }
    }

    func summarizePersistedThreads(_ entries: JsonArray) -> String {
        joinObjects(entries) { entry in
            [
                stringValue(entry["chat_id"]),
                "messages=\(arrayValue(entry["messages"]).count)",
                "unread=\(intValue(entry["unread_count"]))",
            ].joined(separator: ",")
        }
    }

    func readJsonObject(at url: URL) -> JsonObject? {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? JsonObject else {
            return nil
        }
        return object
    }

    func jsonObjectFromString(_ raw: String) throws -> JsonObject {
        guard let data = raw.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: data) as? JsonObject else {
            throw HarnessError.unexpected("invalid json object")
        }
        return object
    }

    func readJsonArray(at url: URL) -> JsonArray {
        guard let data = try? Data(contentsOf: url),
              let array = try? JSONSerialization.jsonObject(with: data) as? JsonArray else {
            return []
        }
        return array
    }

    func dictValue(_ value: Any?) -> JsonObject? {
        value as? JsonObject
    }

    func arrayValue(_ value: Any?) -> JsonArray {
        value as? JsonArray ?? []
    }

    func stringValue(_ value: Any?) -> String {
        switch value {
        case let value as String:
            return value
        case let value as NSNumber:
            return value.stringValue
        case _ as NSNull:
            return ""
        case .none:
            return ""
        default:
            return String(describing: value!)
        }
    }

    func boolValue(_ value: Any?) -> Bool {
        switch value {
        case let value as Bool:
            return value
        case let value as NSNumber:
            return value.boolValue
        case let value as String:
            return ["1", "true", "TRUE", "True"].contains(value)
        default:
            return false
        }
    }

    func intValue(_ value: Any?) -> Int {
        switch value {
        case let value as Int:
            return value
        case let value as UInt64:
            return Int(value)
        case let value as NSNumber:
            return value.intValue
        case let value as String:
            return Int(value) ?? 0
        default:
            return 0
        }
    }

    func joinObjects(_ entries: JsonArray, limit: Int = Int.max, block: (JsonObject) -> String) -> String {
        entries.prefix(limit).compactMap { dictValue($0).map(block) }.joined(separator: "|")
    }

    func joinValues(_ entries: JsonArray, limit: Int = Int.max) -> String {
        entries.prefix(limit).map(stringValue).joined(separator: "|")
    }

    func sameIdentifier(_ lhs: String, _ rhs: String) -> Bool {
        lhs.caseInsensitiveCompare(rhs) == .orderedSame
    }

    func optionalBoolEnv(_ key: String, env: [String: String]) -> Bool? {
        guard let raw = env[key]?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased(),
              !raw.isEmpty else {
            return nil
        }
        if ["1", "true", "yes", "on"].contains(raw) {
            return true
        }
        if ["0", "false", "no", "off"].contains(raw) {
            return false
        }
        return nil
    }

    func harnessTimeout(
        env: [String: String],
        defaultSeconds: TimeInterval,
        minimumSeconds: TimeInterval = 1,
        maximumSeconds: TimeInterval = 180
    ) -> TimeInterval {
        let requestedSeconds =
            Double(env["IRIS_IOS_HARNESS_TIMEOUT_SECS"] ?? "") ??
            env["IRIS_IOS_HARNESS_TIMEOUT_MS"].flatMap { Double($0).map { $0 / 1_000 } } ??
            defaultSeconds
        return min(max(requestedSeconds, minimumSeconds), maximumSeconds)
    }

    func nearbyProfileTimeout(env: [String: String]) -> TimeInterval {
        harnessTimeout(env: env, defaultSeconds: 20)
    }

    func holdNearbyIfRequested(env: [String: String]) async throws {
        let holdMs = min(max(Int(env["IRIS_IOS_HARNESS_HOLD_MS"] ?? "") ?? 0, 0), 60_000)
        guard holdMs > 0 else { return }
        status("nearby_hold_ms", String(holdMs))
        try await Task.sleep(nanoseconds: UInt64(holdMs) * 1_000_000)
    }

    func status(_ key: String, _ value: String) {
        print("HARNESS_STATUS: \(key)=\(value)")
        fflush(stdout)
        guard let path = ProcessInfo.processInfo.environment["IRIS_IOS_HARNESS_STATUS_FILE"],
              !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let line = "\(key)=\(value)\n"
        guard let data = line.data(using: .utf8) else {
            return
        }
        if FileManager.default.fileExists(atPath: path),
           let handle = try? FileHandle(forWritingTo: URL(fileURLWithPath: path)) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
        } else {
            try? data.write(to: URL(fileURLWithPath: path), options: .atomic)
        }
    }
}
