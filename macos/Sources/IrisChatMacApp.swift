import AppKit
import SwiftUI

@main
struct IrisChatMacApp: App {
    @StateObject private var manager = AppManager()
    @NSApplicationDelegateAdaptor(IrisChatAppDelegate.self) private var appDelegate
    @Environment(\.scenePhase) private var scenePhase
    private let startInBackground = CommandLine.arguments.contains(PlatformStartupAtLogin.backgroundLaunchArgument)

    var body: some Scene {
        WindowGroup {
            RootView(manager: manager)
                .frame(minWidth: 980, minHeight: 640)
                .modifier(MacUserActivityMonitor(manager: manager))
                .onAppear {
                    appDelegate.configure(manager: manager, startInBackground: startInBackground)
                    manager.updates.runStartupCheckIfNeeded()
                }
                .onOpenURL { url in
                    if !manager.handleShareURL(url) {
                        manager.handleChatLink(url)
                    }
                }
                .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
                    guard let url = activity.webpageURL else {
                        return
                    }
                    manager.handleChatLink(url)
                }
                .irisOnChange(of: scenePhase) { phase in
                    if phase == .active {
                        manager.appForegrounded()
                    }
                }
        }
        .defaultSize(width: 1280, height: 820)
        .windowResizability(.automatic)
    }
}

private struct MacUserActivityMonitor: ViewModifier {
    @ObservedObject var manager: AppManager
    @State private var monitor: Any?

    func body(content: Content) -> some View {
        content
            .onAppear {
                guard monitor == nil else { return }
                monitor = NSEvent.addLocalMonitorForEvents(matching: [
                    .leftMouseDown,
                    .leftMouseUp,
                    .rightMouseDown,
                    .rightMouseUp,
                    .otherMouseDown,
                    .otherMouseUp,
                    .mouseMoved,
                    .leftMouseDragged,
                    .rightMouseDragged,
                    .otherMouseDragged,
                    .scrollWheel,
                    .keyDown,
                    .keyUp,
                    .flagsChanged
                ]) { event in
                    manager.recordUserActivity()
                    return event
                }
            }
            .onDisappear {
                if let monitor {
                    NSEvent.removeMonitor(monitor)
                    self.monitor = nil
                }
            }
    }
}
