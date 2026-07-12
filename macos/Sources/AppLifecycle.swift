import AppKit
import Darwin
import Foundation
import UserNotifications

@MainActor
final class IrisChatAppDelegate: NSObject, NSApplicationDelegate {
    private let singleInstance = SingleInstanceCoordinator()
    private weak var manager: AppManager?
    private var pendingUrls: [URL] = []
    private var startsHidden = false
    private let notificationDelegate = MacUserNotificationDelegate()

    func applicationWillFinishLaunching(_ notification: Notification) {
        singleInstance.onOpen = { [weak self] urls in
            self?.route(urls: urls, activate: true)
        }
        if !singleInstance.claimOrNotifyCurrentLaunch() {
            NSApp.terminate(nil)
        }
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(repurposeMiniaturizeButton(_:)),
            name: NSWindow.didBecomeKeyNotification,
            object: nil
        )
        // Owning the delegate ensures banners display while the app is the
        // frontmost process. Without it macOS silently drops foreground
        // notifications and the user only sees them in Notification Center.
        UNUserNotificationCenter.current().delegate = notificationDelegate
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        manager?.appForegrounded()
    }

    func applicationDidResignActive(_ notification: Notification) {
        manager?.appBackgrounded()
    }

    // Redirect the yellow minimize and red close buttons to NSApp.hide so the
    // window folds into the existing dock icon instead of spawning a second
    // thumbnail or destroying the WindowGroup window outright.
    // applicationShouldHandleReopen brings it back when the user clicks the
    // dock. Without this, closing the window leaves the app running with no
    // way to surface a window again from the dock icon.
    @objc private func repurposeMiniaturizeButton(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        for kind in [NSWindow.ButtonType.miniaturizeButton, .closeButton] {
            guard let button = window.standardWindowButton(kind) else { continue }
            button.target = NSApp
            button.action = #selector(NSApplication.hide(_:))
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if mainWindow() == nil {
            // The window has actually been destroyed (Cmd+W, or some path
            // we haven't redirected to hide). Letting AppKit run its default
            // reopen behavior nudges SwiftUI's WindowGroup into recreating a
            // fresh window from the group; trying to show NSApp.windows.first
            // here would be a no-op.
            return true
        }
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
        NSApp.activate(ignoringOtherApps: true)
        if let window = mainWindow() {
            if window.isMiniaturized {
                window.deminiaturize(nil)
            }
            window.makeKeyAndOrderFront(nil)
        }
    }

    private func hideMainWindowSoon() {
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.mainWindow() else { return }
            window.orderOut(nil)
        }
    }

    private func mainWindow() -> NSWindow? {
        NSApp.windows.first(where: { $0.title == "Iris Chat" })
            ?? NSApp.windows.first(where: { $0.canBecomeKey })
            ?? NSApp.windows.first
    }

    private static var launchArgumentsContainDeepLink: Bool {
        CommandLine.arguments.contains { $0.starts(with: "irischat://") }
    }
}

final class MacUserNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .list])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        DispatchQueue.main.async {
            NSApp.activate(ignoringOtherApps: true)
            let window = NSApp.windows.first(where: { $0.title == "Iris Chat" })
                ?? NSApp.windows.first(where: { $0.canBecomeKey })
                ?? NSApp.windows.first
            window?.makeKeyAndOrderFront(nil)
        }
        completionHandler()
    }
}

final class SingleInstanceCoordinator: NSObject {
    private let isolatedRunID = ProcessInfo.processInfo.environment["IRIS_UI_TEST_RUN_ID"]?
        .trimmingCharacters(in: .whitespacesAndNewlines)
    private lazy var notificationName = Notification.Name(
        isolatedRunID.flatMap { $0.isEmpty ? nil : "to.iris.chat.macos.open.\(Self.lockSuffix($0))" }
            ?? "to.iris.chat.macos.open"
    )
    private var lockFds: [Int32] = []
    var onOpen: (([URL]) -> Void)?

    func claimOrNotifyCurrentLaunch() -> Bool {
        var acquiredFds: [Int32] = []
        for lockPath in Self.lockFilePaths(isolatedRunID: isolatedRunID) {
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

        if acquiredFds.isEmpty && isolatedRunID?.isEmpty != false {
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

    private static func lockFilePaths(isolatedRunID: String?) -> [String] {
        if let isolatedRunID, !isolatedRunID.isEmpty {
            let suffix = lockSuffix(isolatedRunID)
            return ["/tmp/to.iris.chat.gui.\(getuid()).\(suffix).lock"]
        }
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

    private static func lockSuffix(_ value: String) -> String {
        String(value.unicodeScalars.map { scalar in
            CharacterSet.alphanumerics.contains(scalar) ? Character(String(scalar)) : "-"
        }.prefix(80))
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
