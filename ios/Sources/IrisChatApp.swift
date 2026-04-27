import SwiftUI

@main
struct IrisChatApp: App {
#if os(iOS)
    @UIApplicationDelegateAdaptor(IrisPushAppDelegate.self) private var appDelegate
#endif
    @StateObject private var manager = AppManager()

    var body: some Scene {
        WindowGroup {
            RootView(manager: manager)
                .onAppear {
                    appDelegate.manager = manager
                }
                .onOpenURL { url in
                    manager.handleChatLink(url)
                }
                .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
                    guard let url = activity.webpageURL else {
                        return
                    }
                    manager.handleChatLink(url)
                }
        }
    }
}
