import AppKit
import SwiftUI

@main
struct IrisChatMacApp: App {
    @StateObject private var manager = AppManager()
    private let startInBackground = CommandLine.arguments.contains(PlatformStartupAtLogin.backgroundLaunchArgument)

    var body: some Scene {
        WindowGroup {
            RootView(manager: manager)
                .frame(minWidth: 980, minHeight: 640)
                .onAppear {
                    if startInBackground {
                        NSApplication.shared.hide(nil)
                    }
                }
                .onOpenURL { url in
                    _ = manager.handleShareURL(url)
                }
        }
        .defaultSize(width: 1280, height: 820)
        .windowResizability(.automatic)
    }
}
