import Foundation
import SwiftUI
import UserNotifications

private var irisDebugLoggingEnabled = false
private let irisClientDebugLogLock = NSLock()
private var irisClientDebugLog: [IrisClientDebugLogEntry] = []
private let irisMaxClientDebugLogEntries = 50
private let irisMaxClientDebugLogDetailChars = 1_000

private struct IrisClientDebugLogEntry {
    let timestampSecs: UInt64
    let category: String
    let detail: String

    var jsonObject: [String: Any] {
        [
            "timestamp_secs": timestampSecs,
            "category": category,
            "detail": detail
        ]
    }
}

func irisSetDebugLoggingEnabled(_ enabled: Bool) {
    irisDebugLoggingEnabled = enabled
}

func irisDebugLog(_ format: String, _ args: CVarArg...) {
    guard irisDebugLoggingEnabled else { return }
    withVaList(args) { pointer in
        NSLogv(format, pointer)
    }
}

func irisAppendClientDebugLog(category: String, detail: String) {
    irisClientDebugLogLock.lock()
    defer { irisClientDebugLogLock.unlock() }
    irisClientDebugLog.append(
        IrisClientDebugLogEntry(
            timestampSecs: UInt64(Date().timeIntervalSince1970),
            category: category,
            detail: String(detail.prefix(irisMaxClientDebugLogDetailChars))
        )
    )
    let excessCount = irisClientDebugLog.count - irisMaxClientDebugLogEntries
    if excessCount > 0 {
        irisClientDebugLog.removeFirst(excessCount)
    }
}

func irisClientDebugLogObjects() -> [[String: Any]] {
    irisClientDebugLogLock.lock()
    defer { irisClientDebugLogLock.unlock() }
    return irisClientDebugLog.map(\.jsonObject)
}

#if os(iOS)
import UIKit
import WebKit

typealias PlatformImage = UIImage

extension Image {
    init(platformImage: PlatformImage) {
        self.init(uiImage: platformImage)
    }
}

struct IrisAnimatedImageDataView: UIViewRepresentable {
    let data: Data

    func makeUIView(context: Context) -> WKWebView {
        let webView = WKWebView(frame: .zero)
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        webView.scrollView.isScrollEnabled = false
        webView.scrollView.bounces = false
        load(data, in: webView)
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        load(data, in: webView)
    }

    private func load(_ data: Data, in webView: WKWebView) {
        webView.loadHTMLString(irisAnimatedImageHTML(data: data), baseURL: nil)
    }
}
#elseif os(macOS)
import AppKit
import WebKit

typealias PlatformImage = NSImage

extension Image {
    init(platformImage: PlatformImage) {
        self.init(nsImage: platformImage)
    }
}

struct IrisAnimatedImageDataView: NSViewRepresentable {
    let data: Data

    func makeNSView(context: Context) -> WKWebView {
        let webView = WKWebView(frame: .zero)
        webView.setValue(false, forKey: "drawsBackground")
        load(data, in: webView)
        return webView
    }

    func updateNSView(_ webView: WKWebView, context: Context) {
        load(data, in: webView)
    }

    private func load(_ data: Data, in webView: WKWebView) {
        webView.loadHTMLString(irisAnimatedImageHTML(data: data), baseURL: nil)
    }
}
#endif

private func irisAnimatedImageHTML(data: Data) -> String {
    let encoded = data.base64EncodedString()
    return """
    <!doctype html>
    <html>
    <head>
    <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1">
    <style>
    html, body {
      margin: 0;
      width: 100%;
      height: 100%;
      overflow: hidden;
      background: transparent;
    }
    body {
      display: flex;
      align-items: center;
      justify-content: center;
    }
    img {
      width: 100%;
      height: 100%;
      object-fit: contain;
    }
    </style>
    </head>
    <body><img src="data:image/gif;base64,\(encoded)" alt=""></body>
    </html>
    """
}

enum PlatformClipboard {
    static func string() -> String? {
        #if os(iOS)
        UIPasteboard.general.string
        #elseif os(macOS)
        NSPasteboard.general.string(forType: .string)
        #else
        nil
        #endif
    }

    static func setString(_ value: String) {
        #if os(iOS)
        UIPasteboard.general.string = value
        #elseif os(macOS)
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(value, forType: .string)
        #endif
    }
}

enum PlatformDocumentOpener {
    static func open(_ url: URL) -> Bool {
        #if os(iOS)
        return IrisDocumentInteractionPresenter.shared.present(url)
        #elseif os(macOS)
        return NSWorkspace.shared.open(url)
        #else
        return false
        #endif
    }
}

enum PlatformExternalURL {
    static func open(_ url: URL) {
        #if os(iOS)
        UIApplication.shared.open(url)
        #elseif os(macOS)
        NSWorkspace.shared.open(url)
        #endif
    }
}

enum PlatformAppSettings {
    static func open() {
        #if os(iOS)
        guard let url = URL(string: UIApplication.openSettingsURLString) else {
            return
        }
        UIApplication.shared.open(url)
        #elseif os(macOS)
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy") else {
            return
        }
        NSWorkspace.shared.open(url)
        #endif
    }
}

enum PlatformHaptics {
    static func messageMenuOpened() {
        #if os(iOS)
        let generator = UIImpactFeedbackGenerator(style: .rigid)
        generator.impactOccurred(intensity: 0.85)
        #endif
    }
}

enum PlatformDeviceLabels {
    static var currentDeviceLabel: String {
        #if os(iOS)
        let name = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? "iPhone" : name
        #elseif os(macOS)
        let name = Host.current().localizedName?.trimmingCharacters(in: .whitespacesAndNewlines)
        return (name?.isEmpty == false) ? name! : "Mac"
        #else
        return "This device"
        #endif
    }

    static var currentClientLabel: String {
        #if os(iOS)
        return "Iris Chat Mobile"
        #elseif os(macOS)
        return "Iris Chat Desktop"
        #else
        return "Iris Chat"
        #endif
    }
}

#if os(iOS)
private final class IrisDocumentInteractionPresenter: NSObject, UIDocumentInteractionControllerDelegate {
    static let shared = IrisDocumentInteractionPresenter()

    private var controller: UIDocumentInteractionController?

    func present(_ url: URL) -> Bool {
        guard let source = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows)
            .first(where: \.isKeyWindow)?
            .rootViewController
        else {
            return false
        }

        let controller = UIDocumentInteractionController(url: url)
        controller.delegate = self
        self.controller = controller
        if controller.presentPreview(animated: true) {
            return true
        }
        return controller.presentOpenInMenu(from: source.view.bounds, in: source.view, animated: true)
    }

    func documentInteractionControllerViewControllerForPreview(
        _ controller: UIDocumentInteractionController
    ) -> UIViewController {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow)?
            .rootViewController ?? UIViewController()
    }
}
#endif

var irisSupportsQrScanning: Bool {
    #if canImport(UIKit)
    true
    #else
    false
    #endif
}

extension View {
    @ViewBuilder
    func irisIdentifierInputModifiers() -> some View {
        #if canImport(UIKit)
        self
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisDraftInputModifiers() -> some View {
        #if canImport(UIKit)
        self
            .textInputAutocapitalization(.sentences)
            .autocorrectionDisabled(false)
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisDesktopSubmit(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.onSubmit(action)
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisOnChange<Value: Equatable>(
        of value: Value,
        _ action: @escaping (Value) -> Void
    ) -> some View {
        #if canImport(AppKit)
        self.onChange(of: value) { _, newValue in
            action(newValue)
        }
        #else
        self.onChange(of: value, perform: action)
        #endif
    }

    @ViewBuilder
    func irisInteractiveKeyboardDismiss() -> some View {
        #if canImport(UIKit)
        self.scrollDismissesKeyboard(.interactively)
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisInlineTitleDisplayMode() -> some View {
        #if canImport(UIKit)
        self.navigationBarTitleDisplayMode(.inline)
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisOnExitCommand(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.onExitCommand(perform: action)
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisOnEscapeKey(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.background(IrisEscapeKeyHandler(action: action).frame(width: 0, height: 0))
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisOnLeftArrowKey(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.background(IrisArrowKeyHandler(keyCode: 123, action: action).frame(width: 0, height: 0))
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisOnRightArrowKey(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.background(IrisArrowKeyHandler(keyCode: 124, action: action).frame(width: 0, height: 0))
        #else
        self
        #endif
    }

    @ViewBuilder
    func irisDismissOnMacOutsideClick(_ action: @escaping () -> Void) -> some View {
        #if canImport(AppKit)
        self.background(IrisOutsideClickDismissHandler(action: action).allowsHitTesting(false))
        #else
        self
        #endif
    }
}

#if canImport(AppKit)
private struct IrisEscapeKeyHandler: NSViewRepresentable {
    let action: () -> Void

    func makeNSView(context: Context) -> IrisEscapeKeyView {
        let view = IrisEscapeKeyView()
        view.action = action
        DispatchQueue.main.async {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ view: IrisEscapeKeyView, context: Context) {
        view.action = action
        DispatchQueue.main.async {
            view.window?.makeFirstResponder(view)
        }
    }
}

private final class IrisEscapeKeyView: NSView {
    var action: (() -> Void)?

    override var acceptsFirstResponder: Bool {
        true
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 {
            action?()
        } else {
            super.keyDown(with: event)
        }
    }
}

private struct IrisArrowKeyHandler: NSViewRepresentable {
    let keyCode: UInt16
    let action: () -> Void

    func makeNSView(context: Context) -> IrisArrowKeyView {
        let view = IrisArrowKeyView()
        view.keyCode = keyCode
        view.action = action
        DispatchQueue.main.async {
            view.window?.makeFirstResponder(view)
        }
        return view
    }

    func updateNSView(_ view: IrisArrowKeyView, context: Context) {
        view.keyCode = keyCode
        view.action = action
        DispatchQueue.main.async {
            view.window?.makeFirstResponder(view)
        }
    }
}

private final class IrisArrowKeyView: NSView {
    var keyCode: UInt16 = 0
    var action: (() -> Void)?

    override var acceptsFirstResponder: Bool {
        true
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == keyCode {
            action?()
        } else {
            super.keyDown(with: event)
        }
    }
}

private struct IrisOutsideClickDismissHandler: NSViewRepresentable {
    let action: () -> Void

    func makeNSView(context: Context) -> IrisOutsideClickDismissView {
        let view = IrisOutsideClickDismissView()
        view.action = action
        return view
    }

    func updateNSView(_ view: IrisOutsideClickDismissView, context: Context) {
        view.action = action
        view.ensureMonitoring()
    }

    static func dismantleNSView(_ nsView: IrisOutsideClickDismissView, coordinator: ()) {
        nsView.stopMonitoring()
    }
}

private final class IrisOutsideClickDismissView: NSView {
    var action: (() -> Void)?
    private var localMonitor: Any?
    private var globalMonitor: Any?
    private var isDismissing = false

    deinit {
        stopMonitoring()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        ensureMonitoring()
    }

    func ensureMonitoring() {
        guard window != nil else {
            stopMonitoring()
            return
        }
        guard localMonitor == nil else {
            return
        }

        let mouseDownEvents: NSEvent.EventTypeMask = [.leftMouseDown, .rightMouseDown, .otherMouseDown]
        localMonitor = NSEvent.addLocalMonitorForEvents(matching: mouseDownEvents) { [weak self] event in
            self?.handleLocalMouseDown(event) ?? event
        }
        globalMonitor = NSEvent.addGlobalMonitorForEvents(matching: mouseDownEvents) { [weak self] _ in
            self?.dismiss()
        }
    }

    func stopMonitoring() {
        if let localMonitor {
            NSEvent.removeMonitor(localMonitor)
            self.localMonitor = nil
        }
        if let globalMonitor {
            NSEvent.removeMonitor(globalMonitor)
            self.globalMonitor = nil
        }
    }

    private func handleLocalMouseDown(_ event: NSEvent) -> NSEvent? {
        guard let window else {
            return event
        }
        guard event.window === window else {
            dismiss()
            return nil
        }

        let localPoint = convert(event.locationInWindow, from: nil)
        guard bounds.contains(localPoint) else {
            dismiss()
            return nil
        }
        return event
    }

    private func dismiss() {
        guard !isDismissing else {
            return
        }
        isDismissing = true
        DispatchQueue.main.async { [weak self] in
            self?.action?()
            self?.isDismissing = false
        }
    }
}
#endif

var irisToolbarTrailingPlacement: ToolbarItemPlacement {
    #if canImport(UIKit)
    .topBarTrailing
    #else
    .automatic
    #endif
}

protocol DesktopNotificationPosting {
    func post(title: String, body: String)
}

final class NoopDesktopNotificationPoster: DesktopNotificationPosting {
    func post(title: String, body: String) {
        _ = title
        _ = body
    }
}

final class SystemDesktopNotificationPoster: DesktopNotificationPosting {
    private let center = UNUserNotificationCenter.current()
    private let environment: [String: String]

    init(environment: [String: String] = ProcessInfo.processInfo.environment) {
        self.environment = environment
    }

    func post(title: String, body: String) {
        guard !AppPaths.notificationsDisabledForAutomation(environment: environment) else {
            return
        }
        center.getNotificationSettings { [center] settings in
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                Self.enqueue(title: title, body: body, center: center)
            case .notDetermined:
                center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                    guard granted else {
                        return
                    }
                    Self.enqueue(title: title, body: body, center: center)
                }
            case .denied:
                break
            @unknown default:
                break
            }
        }
    }

    private static func enqueue(title: String, body: String, center: UNUserNotificationCenter) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(
            identifier: "iris-chat-\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        center.add(request)
    }
}

enum PlatformStartupAtLogin {
    static let backgroundLaunchArgument = "--background"

    static var isSupported: Bool {
        #if os(macOS)
        true
        #else
        false
        #endif
    }

    static func setEnabled(_ enabled: Bool) throws {
        #if os(macOS)
        if enabled {
            try writeLaunchAgent()
        } else {
            try removeLaunchAgent()
        }
        #endif
    }

    #if os(macOS)
    private static let launchAgentLabel = "to.iris.chat.login"

    private static var launchAgentURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents", isDirectory: true)
            .appendingPathComponent("\(launchAgentLabel).plist")
    }

    private static func writeLaunchAgent() throws {
        guard let executable = Bundle.main.executableURL?.path else {
            throw CocoaError(.fileNoSuchFile)
        }
        let url = launchAgentURL
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try launchAgentPlist(executable: executable).write(
            to: url,
            atomically: true,
            encoding: .utf8
        )
    }

    private static func removeLaunchAgent() throws {
        let url = launchAgentURL
        guard FileManager.default.fileExists(atPath: url.path) else {
            return
        }
        try FileManager.default.removeItem(at: url)
    }

    private static func launchAgentPlist(executable: String) -> String {
        """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
          <key>Label</key>
          <string>\(xmlEscape(launchAgentLabel))</string>
          <key>ProgramArguments</key>
          <array>
            <string>\(xmlEscape(executable))</string>
            <string>\(xmlEscape(backgroundLaunchArgument))</string>
          </array>
          <key>RunAtLoad</key>
          <true/>
          <key>LimitLoadToSessionType</key>
          <string>Aqua</string>
        </dict>
        </plist>
        """
    }

    private static func xmlEscape(_ value: String) -> String {
        value
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "'", with: "&apos;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
    }
    #endif
}
