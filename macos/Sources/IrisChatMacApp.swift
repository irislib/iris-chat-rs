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
                .onAppear {
                    appDelegate.configure(manager: manager, startInBackground: startInBackground)
                    manager.updates.runStartupCheckIfNeeded()
                }
                .onOpenURL { url in
                    _ = manager.handleShareURL(url)
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
