import AppKit
import Darwin
import Foundation

@MainActor
final class IrisChatAppDelegate: NSObject, NSApplicationDelegate {
    private let singleInstance = SingleInstanceCoordinator()
    private weak var manager: AppManager?
    private var pendingUrls: [URL] = []
    private var startsHidden = false

    func applicationWillFinishLaunching(_ notification: Notification) {
        singleInstance.onOpen = { [weak self] urls in
            self?.route(urls: urls, activate: true)
        }
        if !singleInstance.claimOrNotifyCurrentLaunch() {
            NSApp.terminate(nil)
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showMainWindow()
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        singleInstance.release()
    }

    func configure(manager: AppManager, startInBackground: Bool) {
        self.manager = manager
        startsHidden = startInBackground && !Self.launchArgumentsContainDeepLink
        route(urls: pendingUrls, activate: !startsHidden)
        pendingUrls.removeAll()
        if startsHidden {
            hideMainWindowSoon()
        }
    }

    private func route(urls: [URL], activate: Bool) {
        guard !urls.isEmpty else {
            if activate {
                showMainWindow()
            }
            return
        }
        guard let manager else {
            pendingUrls.append(contentsOf: urls)
            return
        }
        for url in urls {
            _ = manager.handleShareURL(url)
        }
        if activate {
            showMainWindow()
        }
    }

    private func showMainWindow() {
        NSApp.unhide(nil)
        NSApp.activate()
        if let window = NSApp.windows.first(where: { $0.title == "Iris Chat" }) ?? NSApp.windows.first {
            window.makeKeyAndOrderFront(nil)
        }
    }

    private func hideMainWindowSoon() {
        DispatchQueue.main.async {
            NSApp.windows.first(where: { $0.title == "Iris Chat" })?.orderOut(nil)
        }
    }

    private static var launchArgumentsContainDeepLink: Bool {
        CommandLine.arguments.contains { $0.starts(with: "irischat://") }
    }
}

final class SingleInstanceCoordinator: NSObject {
    private let notificationName = Notification.Name("to.iris.chat.macos.open")
    private var lockFds: [Int32] = []
    var onOpen: (([URL]) -> Void)?

    func claimOrNotifyCurrentLaunch() -> Bool {
        var acquiredFds: [Int32] = []
        for lockPath in Self.lockFilePaths() {
            let fd = open(lockPath, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
            guard fd >= 0 else {
                continue
            }
            if flock(fd, LOCK_EX | LOCK_NB) == 0 {
                acquiredFds.append(fd)
                continue
            }

            close(fd)
            Self.release(fds: acquiredFds)
            notifyCurrentLaunch()
            return false
        }

        if acquiredFds.isEmpty {
            if Self.activateRunningCopy() {
                notifyCurrentLaunch()
                return false
            }
        }

        lockFds = acquiredFds
        DistributedNotificationCenter.default().addObserver(
            self,
            selector: #selector(receiveOpenNotification(_:)),
            name: notificationName,
            object: nil
        )
        return true
    }

    func release() {
        DistributedNotificationCenter.default().removeObserver(self)
        Self.release(fds: lockFds)
        lockFds = []
    }

    @objc private func receiveOpenNotification(_ notification: Notification) {
        let urls = (notification.userInfo?["urls"] as? [String] ?? [])
            .compactMap(URL.init(string:))
        onOpen?(urls)
    }

    private func notifyCurrentLaunch() {
        DistributedNotificationCenter.default().postNotificationName(
            notificationName,
            object: nil,
            userInfo: ["urls": Self.startupUrls().map(\.absoluteString)],
            deliverImmediately: true
        )
    }

    private static func lockFilePaths() -> [String] {
        var paths = ["/tmp/to.iris.chat.gui.\(getuid()).lock"]
        if let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("iris-chat", isDirectory: true) {
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            paths.append(dir.appendingPathComponent("IrisChatMac.lock").path)
        }
        return paths
    }

    private static func release(fds: [Int32]) {
        for fd in fds {
            flock(fd, LOCK_UN)
            close(fd)
        }
    }

    private static func activateRunningCopy() -> Bool {
        let currentPid = getpid()
        guard let app = NSWorkspace.shared.runningApplications.first(where: { app in
            app.processIdentifier != currentPid
                && app.activationPolicy == .regular
                && app.bundleIdentifier == "to.iris.chat.macos"
        }) else {
            return false
        }
        app.activate(options: [.activateAllWindows])
        return true
    }

    private static func startupUrls() -> [URL] {
        CommandLine.arguments.compactMap { argument in
            guard argument.starts(with: "irischat://") else {
                return nil
            }
            return URL(string: argument)
        }
    }
}
