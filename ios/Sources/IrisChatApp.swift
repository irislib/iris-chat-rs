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
        }
    }
}
