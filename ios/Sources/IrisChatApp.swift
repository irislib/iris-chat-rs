import SwiftUI
#if os(iOS)
import UIKit
#endif

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
#if os(iOS)
                .background(IOSUserActivityMonitor(manager: manager))
#endif
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
                    } else if phase == .inactive {
                        manager.appInactive()
                    } else if phase == .background {
                        manager.appBackgrounded()
                    }
                }
        }
    }
}

#if os(iOS)
private struct IOSUserActivityMonitor: UIViewRepresentable {
    @ObservedObject var manager: AppManager

    func makeCoordinator() -> Coordinator {
        Coordinator(manager: manager)
    }

    func makeUIView(context: Context) -> MonitorView {
        let view = MonitorView()
        view.onWindowChanged = { [weak coordinator = context.coordinator] window in
            coordinator?.attach(to: window)
        }
        return view
    }

    func updateUIView(_ uiView: MonitorView, context: Context) {
        context.coordinator.manager = manager
        context.coordinator.attach(to: uiView.window)
    }

    final class MonitorView: UIView {
        var onWindowChanged: ((UIWindow?) -> Void)?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            onWindowChanged?(window)
        }

        override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
            false
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        weak var manager: AppManager?
        private weak var attachedWindow: UIWindow?
        private var recognizer: TouchActivityRecognizer?

        init(manager: AppManager) {
            self.manager = manager
        }

        func attach(to window: UIWindow?) {
            guard attachedWindow !== window else { return }
            if let recognizer, let attachedWindow {
                attachedWindow.removeGestureRecognizer(recognizer)
            }
            attachedWindow = window
            guard let window else {
                recognizer = nil
                return
            }
            let recognizer = TouchActivityRecognizer { [weak self] in
                self?.manager?.recordUserActivity()
            }
            recognizer.cancelsTouchesInView = false
            recognizer.delaysTouchesBegan = false
            recognizer.delaysTouchesEnded = false
            recognizer.delegate = self
            window.addGestureRecognizer(recognizer)
            self.recognizer = recognizer
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }
}

private final class TouchActivityRecognizer: UIGestureRecognizer {
    private let onActivity: () -> Void

    init(onActivity: @escaping () -> Void) {
        self.onActivity = onActivity
        super.init(target: nil, action: nil)
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        onActivity()
        state = .failed
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        onActivity()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        onActivity()
        state = .failed
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        state = .cancelled
    }
}
#endif
