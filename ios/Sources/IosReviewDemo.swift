#if os(iOS)
import Foundation

/// An ordinary, independently generated profile with a small amount of sample
/// history. No account keys or shared reviewer identity are shipped in the app.
enum IosReviewDemo {
    static let username = "AppStoreDemoUserMode"
    static let profileName = "Demo profile"
    // Public bundled audio; content hash, not an account or encryption key.
    static let audioHash = "nhash1qqsqlmx78qteykhagrv4yyzp39k8th9y8aumf59hj5gawt73ddyzkzg7gc5v7"
    static let audioFilename = "sample-audio.wav"

    static func bundledAudio() async -> Data? {
        await Task.detached(priority: .userInitiated) {
            guard let url = Bundle.main.url(forResource: "sample-audio", withExtension: "wav") else { return nil }
            return try? Data(contentsOf: url)
        }.value
    }

    static let sampleName = "Sample conversation"
    static let welcome = "This is a sample conversation. You can reply, react, record a voice message, or attach a photo."
    static let twoDeviceInstructions = "To test live messages and calls, create a profile on a second device. Open New chat on both. Tap Show on one and Scan code on the other, or use Copy or Share to send a chat link and paste it on the other device. Each device has its own secret key."

    static func matches(_ name: String) -> Bool {
        name.trimmingCharacters(in: .whitespacesAndNewlines).caseInsensitiveCompare(username) == .orderedSame
    }

    static func marker(in directory: URL) -> URL {
        directory.appendingPathComponent("app-store-demo")
    }

    static func isEnabled(in directory: URL) -> Bool {
        FileManager.default.fileExists(atPath: marker(in: directory).path)
    }

    static func needsPreparation(in directory: URL) -> Bool {
        guard isEnabled(in: directory) else { return false }
        return (try? String(contentsOf: marker(in: directory), encoding: .utf8)) != "ready"
    }

    static func enable(in directory: URL) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("pending".utf8).write(to: marker(in: directory), options: .atomic)
    }

    static func populate(primary: FfiApp, directory: URL) async throws {
        let progressURL = directory.appendingPathComponent("app-store-demo-chat")
        if let previous = try? String(contentsOf: progressURL, encoding: .utf8),
           let chat = primary.chatSnapshot(chatId: previous, limit: 30),
           chat.messages.contains(where: { $0.body == twoDeviceInstructions }),
           chat.messages.contains(where: { $0.attachments.contains(where: { $0.nhash == audioHash }) }) {
            primary.dispatch(action: .setContactDetails(ownerPubkeyHex: previous, nickname: sampleName, note: "Sample contact; offline after setup."))
            try await prepareGroup(primary)
            primary.dispatch(action: .updateScreenStack(stack: []))
            try Data("ready".utf8).write(to: marker(in: directory), options: .atomic)
            return
        }
        guard let audio = await bundledAudio() else { throw PreparationError.notReady }
        let cache = IrisAttachmentCache(dataDir: directory)
        _ = try await cache.store(audio, for: IrisAttachmentCache.attachmentKey(nhash: audioHash, filename: audioFilename))
        let peerDirectory = directory.appendingPathComponent("demo-seed-\(UUID().uuidString)")
        let peer = FfiApp(dataDir: peerDirectory.path, keychainGroup: "", appVersion: "")
        let originalBluetooth = primary.state().preferences.nearbyBluetoothEnabled
        defer {
            primary.dispatch(action: .setNearbyBluetoothEnabled(enabled: originalBluetooth))
            peer.shutdown()
            try? FileManager.default.removeItem(at: peerDirectory)
        }
        peer.dispatch(action: .setNostrRelays(relayUrls: []))
        peer.dispatch(action: .setNearbyLanEnabled(enabled: false))
        peer.dispatch(action: .setNearbyBluetoothEnabled(enabled: true))
        primary.dispatch(action: .setNearbyBluetoothEnabled(enabled: true))
        let link = try IosDemoLocalLink(first: primary, second: peer)
        var preparationError: Error?
        do {
            peer.dispatch(action: .createAccount(name: sampleName))
            try await waitUntil { peer.state().account != nil && primary.state().account != nil }
            guard let reviewerID = primary.state().account?.publicKeyHex,
                  let sampleID = peer.state().account?.publicKeyHex else { throw PreparationError.notReady }
            try Data(sampleID.utf8).write(to: progressURL, options: .atomic)
            primary.dispatch(action: .createChat(peerInput: sampleID))
            peer.dispatch(action: .createChat(peerInput: reviewerID))
            primary.dispatch(action: .setContactDetails(ownerPubkeyHex: sampleID, nickname: sampleName, note: "Sample contact; offline after setup."))
            primary.dispatch(action: .setMessageRequestAccepted(chatId: sampleID))
            peer.dispatch(action: .setMessageRequestAccepted(chatId: reviewerID))
            peer.dispatch(action: .sendMessage(chatId: reviewerID, text: welcome))
            try await waitUntil {
                primary.chatSnapshot(chatId: sampleID, limit: 30)?.messages.contains(where: { $0.body == welcome }) == true
            }
            primary.dispatch(action: .sendMessage(chatId: sampleID, text: "Thanks! I'll try the message controls here."))
            peer.dispatch(action: .sendMessage(chatId: reviewerID, text: "These are sample messages. This sample contact is offline after setup; use a second device for live calls."))
            peer.dispatch(action: .sendMessage(chatId: reviewerID, text: "Play this sample audio. You can pause, seek, and change playback speed.\n\(audioHash)/\(audioFilename)"))
            peer.dispatch(action: .sendMessage(chatId: reviewerID, text: twoDeviceInstructions))
            try await waitUntil {
                guard let messages = primary.chatSnapshot(chatId: sampleID, limit: 30)?.messages else { return false }
                return messages.contains(where: { $0.body == twoDeviceInstructions }) && messages.contains(where: { $0.attachments.contains(where: { $0.nhash == audioHash }) })
            }
            try await prepareGroup(primary)
            try Task.checkCancellation()
            primary.dispatch(action: .updateScreenStack(stack: []))
        } catch {
            preparationError = error
        }
        // Always try acknowledged teardown before restoring the real adapter.
        // A failed close is setup failure, never a ready demo or a silent retry.
        try link.close()
        if let preparationError { throw preparationError }
        try Data("ready".utf8).write(to: marker(in: directory), options: .atomic)
    }

    private static func prepareGroup(_ primary: FfiApp) async throws {
        let groupID: String
        if let existing = primary.state().chatList.first(where: { $0.kind == .group && $0.displayName == "Sample group" }) {
            groupID = existing.chatId
        } else {
            primary.dispatch(action: .createGroup(name: "Sample group", memberInputs: []))
            try await waitUntil { primary.state().currentChat?.kind == .group }
            guard let created = primary.state().currentChat?.chatId else { throw PreparationError.notReady }
            groupID = created
        }
        let text = "Try a group message here. Add your other test profile from the group details to test group delivery."
        if primary.chatSnapshot(chatId: groupID, limit: 30)?.messages.contains(where: { $0.body == text }) != true {
            primary.dispatch(action: .sendMessage(chatId: groupID, text: text))
            try await waitUntil {
                primary.chatSnapshot(chatId: groupID, limit: 30)?.messages.contains(where: { $0.body == text }) == true
            }
        }
    }

    private static func waitUntil(_ ready: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(25)
        while !ready() {
            try Task.checkCancellation()
            guard Date() < deadline else { throw PreparationError.notReady }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    private enum PreparationError: Error { case notReady }
}
#endif
