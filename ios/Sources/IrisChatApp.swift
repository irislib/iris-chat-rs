import SwiftUI

@main
struct IrisChatApp: App {
#if os(iOS)
    @UIApplicationDelegateAdaptor(IrisPushAppDelegate.self) private var appDelegate
#endif
    @StateObject private var manager = AppManager()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(manager: manager)
                .onAppear {
                    appDelegate.manager = manager
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
                    } else if phase == .background {
                        manager.appBackgrounded()
                    }
                }
        }
    }
}
