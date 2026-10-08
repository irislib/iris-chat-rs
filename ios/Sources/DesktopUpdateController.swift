#if os(macOS)
import AppKit
import Foundation
import SwiftUI

@MainActor
final class DesktopUpdateController: ObservableObject {
    @Published private(set) var checking = false
    @Published private(set) var installing = false
    @Published private(set) var available = false
    @Published private(set) var version = ""
    @Published private(set) var status = ""
    @Published private(set) var lastCheckedAt: Date?
    @Published var autoCheck: Bool {
        didSet {
            defaults.set(autoCheck, forKey: "updates.autoCheck")
            if autoCheck {
                startAutomaticChecks()
            } else {
                stopAutomaticChecks()
            }
        }
    }
    @Published var autoInstall: Bool {
        didSet {
            defaults.set(autoInstall, forKey: "updates.autoInstall")
            if autoInstall, canInstall {
                install()
            }
        }
    }

    private var hasUpdateAsset = false
    private var task: Task<Void, Never>?
    private var automaticCheckTask: Task<Void, Never>?
    private var startupCheckDone = false
    private let defaults: UserDefaults
    private let fetchUpdate: () async throws -> IrisDesktopUpdateResult
    private let now: () -> Date
    private(set) var diagnostics: [ClientDebugLogEntry] = []
    private static let checkFailureStatus = "Couldn’t check for updates. Try again."
    private static let unverifiedStatus = "Update could not be verified."
    private static let legacyFailurePrefix = "Couldn’t check for updates: "

    init(
        defaults: UserDefaults = .standard,
        fetchUpdate: @escaping () async throws -> IrisDesktopUpdateResult = {
            await Task.detached { irisDesktopUpdateCheck() }.value
        },
        now: @escaping () -> Date = Date.init
    ) {
        self.defaults = defaults
        self.fetchUpdate = fetchUpdate
        self.now = now
        self.autoCheck = defaults.object(forKey: "updates.autoCheck") as? Bool ?? true
        self.autoInstall = defaults.bool(forKey: "updates.autoInstall")
        self.status = defaults.string(forKey: "updates.lastCheckStatus") ?? ""
        self.lastCheckedAt = defaults.object(forKey: "updates.lastCheckedAt") as? Date
        if status.hasPrefix(Self.legacyFailurePrefix) {
            let detail = String(status.dropFirst(Self.legacyFailurePrefix.count))
            recordFailure(operation: "check", detail: detail)
            status = detail == Self.unverifiedStatus ? Self.unverifiedStatus : Self.checkFailureStatus
            defaults.set(status, forKey: "updates.lastCheckStatus")
        }
    }

    deinit {
        automaticCheckTask?.cancel()
        task?.cancel()
    }

    var canInstall: Bool {
        available && hasUpdateAsset && !checking && !installing
    }

    func runStartupCheckIfNeeded() {
        startAutomaticChecks()
    }

    func startAutomaticChecks() {
        guard autoCheck else {
            stopAutomaticChecks()
            return
        }
        if !startupCheckDone {
            startupCheckDone = true
            check(manual: false)
        }
        guard automaticCheckTask == nil else { return }
        automaticCheckTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.automaticCheckIntervalNanoseconds)
                guard let self, !Task.isCancelled else { return }
                guard self.autoCheck else {
                    self.stopAutomaticChecks()
                    return
                }
                self.check(manual: false)
            }
        }
    }

    func stopAutomaticChecks() {
        automaticCheckTask?.cancel()
        automaticCheckTask = nil
    }

    @discardableResult
    func check(manual: Bool = true) -> Task<Void, Never>? {
        guard !checking else { return nil }
        task?.cancel()
        checking = true
        if manual {
            status = "Checking for updates"
        }
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let result = try await self.fetch()
                await MainActor.run {
                    self.apply(result)
                }
            } catch {
                await MainActor.run {
                    self.recordFailure(operation: "check", detail: error.localizedDescription)
                    self.finishCheck(status: Self.failureStatus(error, fallback: Self.checkFailureStatus))
                }
            }
        }
        return task
    }

    private static var automaticCheckIntervalNanoseconds: UInt64 {
        if let raw = ProcessInfo.processInfo.environment["IRIS_UPDATE_POLL_SECONDS"],
           let seconds = Double(raw),
           seconds > 0 {
            return UInt64(seconds * 1_000_000_000)
        }
        return 6 * 60 * 60 * 1_000_000_000
    }

    func install() {
        guard hasUpdateAsset else {
            status = "No macOS update found"
            return
        }
        guard !installing else { return }
        installing = true
        status = "Downloading \(version)"
        Task { [weak self] in
            guard let self else { return }
            do {
                let savedUrl = try await self.downloadEmbeddedUpdate()
                try await MainActor.run {
                    try self.installDownloaded(savedUrl)
                }
            } catch {
                await MainActor.run {
                    self.installing = false
                    self.recordFailure(operation: "install", detail: error.localizedDescription)
                    self.status = Self.failureStatus(error, fallback: "Couldn’t install the update. Try again.")
                }
            }
        }
    }

    private static func failureStatus(_ error: Error, fallback: String) -> String {
        if case IrisUpdateError.updateReturnedUnverifiedSource = error {
            return unverifiedStatus
        }
        return fallback
    }

    private func recordFailure(operation: String, detail: String) {
        diagnostics.append(ClientDebugLogEntry(
            timestampSecs: UInt64(now().timeIntervalSince1970),
            category: "updates.\(operation).failed",
            detail: detail
        ))
        if diagnostics.count > 10 { diagnostics.removeFirst(diagnostics.count - 10) }
        NSLog("Iris Chat update %@ failed: %@", operation, detail)
    }

    private func fetch() async throws -> IrisUpdateCheck {
        let result = try await fetchUpdate()
        try validateIrisUpdateResult(result)
        return IrisUpdateCheck(
            tag: result.tag,
            assetName: result.asset.isEmpty ? nil : result.asset,
            isNewer: result.available
        )
    }

    private func finishCheck(status: String) {
        checking = false
        self.status = status
        lastCheckedAt = now()
        defaults.set(status, forKey: "updates.lastCheckStatus")
        defaults.set(lastCheckedAt, forKey: "updates.lastCheckedAt")
    }

    private func apply(_ check: IrisUpdateCheck) {
        available = check.isNewer
        version = check.tag
        hasUpdateAsset = check.isNewer && check.assetName != nil
        finishCheck(status: check.isNewer
            ? (!hasUpdateAsset
                ? "Update \(check.tag) found without a macOS app"
                : "Update \(check.tag) available")
            : "Up to date")
        if autoInstall, hasUpdateAsset {
            install()
        }
    }

    private func downloadEmbeddedUpdate() async throws -> URL {
        let downloadDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("IrisChatDownloads", isDirectory: true)
        let result = await Task.detached {
            irisDesktopUpdateDownload(downloadDir: downloadDir.path)
        }.value
        try validateIrisUpdateResult(result)
        guard let path = result.path, !path.isEmpty else {
            throw IrisUpdateError.missingDownloadedPath
        }
        return URL(fileURLWithPath: path)
    }

    private func installDownloaded(_ archiveUrl: URL) throws {
        status = "Installing \(version)"
        if archiveUrl.lastPathComponent.hasSuffix(".app.tar.gz") {
            let unpackDir = FileManager.default.temporaryDirectory
                .appendingPathComponent("IrisChatUpdate-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: unpackDir, withIntermediateDirectories: true)
            try runIrisUpdateProcess("/usr/bin/tar", arguments: ["-xzf", archiveUrl.path, "-C", unpackDir.path])
            guard let newApp = findIrisAppBundle(in: unpackDir) else {
                throw IrisUpdateError.missingAppBundle
            }
            let script = try irisUpdateInstallScript()
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/bin/sh")
            process.arguments = [script.path, Bundle.main.bundleURL.path, newApp.path]
            try process.run()
            NSApp.terminate(nil)
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([archiveUrl])
            installing = false
            status = "Downloaded \(archiveUrl.lastPathComponent)"
        }
    }
}

private struct IrisUpdateCheck {
    let tag: String
    let assetName: String?
    let isNewer: Bool
}

private enum IrisUpdateError: LocalizedError {
    case missingAppBundle
    case updateFailed(String)
    case updateReturnedUnverifiedSource
    case missingDownloadedPath

    var errorDescription: String? {
        switch self {
        case .missingAppBundle:
            return "Downloaded update did not contain Iris Chat.app."
        case .updateFailed(let message):
            return message.isEmpty ? "Update failed." : message
        case .updateReturnedUnverifiedSource:
            return "Update could not be verified."
        case .missingDownloadedPath:
            return "Downloaded update was not found."
        }
    }
}

private func validateIrisUpdateResult(_ result: IrisDesktopUpdateResult) throws {
    guard result.ok else {
        throw IrisUpdateError.updateFailed(result.error ?? "")
    }
    if ProcessInfo.processInfo.environment["IRIS_UPDATE_MANIFEST_URL"] == nil,
       result.available,
       (!result.verified || result.source != "hashtree-nostr-blossom") {
        throw IrisUpdateError.updateReturnedUnverifiedSource
    }
}

private func runIrisUpdateProcess(_ executable: String, arguments: [String]) throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    try process.run()
    process.waitUntilExit()
    if process.terminationStatus != 0 {
        throw CocoaError(.executableLoad)
    }
}

private func findIrisAppBundle(in directory: URL) -> URL? {
    guard let enumerator = FileManager.default.enumerator(
        at: directory,
        includingPropertiesForKeys: [.isDirectoryKey],
        options: [.skipsHiddenFiles]
    ) else {
        return nil
    }
    for case let url as URL in enumerator where url.pathExtension == "app" {
        if url.lastPathComponent == "Iris Chat.app" || url.lastPathComponent == "IrisChatMac.app" {
            return url
        }
    }
    return nil
}

private func irisUpdateInstallScript() throws -> URL {
    let script = FileManager.default.temporaryDirectory
        .appendingPathComponent("iris-chat-install-update-\(UUID().uuidString).sh")
    let contents = """
    #!/bin/sh
    set -eu
    current_app="$1"
    new_app="$2"
    sleep 1
    rm -rf "$current_app"
    ditto "$new_app" "$current_app"
    open "$current_app"
    """
    try contents.write(to: script, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    return script
}
#endif
