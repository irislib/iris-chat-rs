import Foundation
import Combine
import SwiftUI
import UniformTypeIdentifiers
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(PhotosUI)
import PhotosUI
#endif

struct NavigationShell<Content: View>: View {
    let title: String
    let subtitle: String?
    let subtitleSystemImage: String?
    let isChatHeader: Bool
    let centerTitle: Bool
    let floatsHeader: Bool
    let canGoBack: Bool
    let onBack: () -> Void
    let backBadgeCount: UInt64
    let leading: AnyView
    let trailing: AnyView
    let titleAccessoryLeading: AnyView
    let onTitleTap: (() -> Void)?
    let offlineBanner: AnyView
    let content: () -> Content

    init(
        title: String,
        subtitle: String? = nil,
        subtitleSystemImage: String? = nil,
        isChatHeader: Bool = false,
        centerTitle: Bool = false,
        floatsHeader: Bool = false,
        canGoBack: Bool,
        onBack: @escaping () -> Void,
        backBadgeCount: UInt64 = 0,
        leading: AnyView = AnyView(EmptyView()),
        trailing: AnyView = AnyView(EmptyView()),
        titleAccessoryLeading: AnyView = AnyView(EmptyView()),
        onTitleTap: (() -> Void)? = nil,
        offlineBanner: AnyView = AnyView(EmptyView()),
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.subtitle = subtitle
        self.subtitleSystemImage = subtitleSystemImage
        self.isChatHeader = isChatHeader
        self.centerTitle = centerTitle
        self.floatsHeader = floatsHeader
        self.canGoBack = canGoBack
        self.onBack = onBack
        self.backBadgeCount = backBadgeCount
        self.leading = leading
        self.trailing = trailing
        self.titleAccessoryLeading = titleAccessoryLeading
        self.onTitleTap = onTitleTap
        self.offlineBanner = offlineBanner
        self.content = content
    }

    @Environment(\.irisPalette) private var palette

    @ViewBuilder
    var body: some View {
        if floatsHeader {
            floatingHeaderBody
        } else {
            insetHeaderBody
        }
    }

    private var insetHeaderBody: some View {
        screenContent
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .safeAreaInset(edge: .top, spacing: 0) {
                navigationHeader
                // Signal-style no-divider header: the title cluster
                // floats on a soft background fade that dissolves
                // into the scrolling chat/list content.
                .background(alignment: .top) {
                    IrisNavigationHeaderChrome(palette: palette)
                        .ignoresSafeArea(.all, edges: .top)
                }
            }
    }

    private var floatingHeaderBody: some View {
        GeometryReader { geometry in
            let topSafeArea = geometry.safeAreaInsets.top
            let contentTopInset = IrisNavigationHeaderMetrics.contentTopInset(
                topSafeArea: topSafeArea,
                isChatHeader: isChatHeader
            )
            let chromeHeight = IrisNavigationHeaderMetrics.chromeHeight(
                topSafeArea: topSafeArea,
                isChatHeader: isChatHeader
            )

            screenContent
                .environment(\.irisNavigationHeaderTopInset, contentTopInset)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .overlay(alignment: .top) {
                    navigationHeader
                        .padding(.top, topSafeArea)
                        .background(alignment: .top) {
                            IrisNavigationHeaderChrome(palette: palette, height: chromeHeight)
                                .ignoresSafeArea(.all, edges: .top)
                        }
                        .zIndex(20)
                }
        }
    }

    private var screenContent: some View {
        content()
    }

    private var navigationHeader: some View {
        VStack(spacing: 0) {
            IrisTopBar(
                title: title,
                subtitle: subtitle,
                subtitleSystemImage: subtitleSystemImage,
                isChatHeader: isChatHeader,
                centerTitle: centerTitle,
                canGoBack: canGoBack,
                onBack: onBack,
                backBadgeCount: backBadgeCount,
                leading: leading,
                trailing: trailing,
                titleAccessoryLeading: titleAccessoryLeading,
                onTitleTap: onTitleTap
            )
            offlineBanner
        }
    }
}

struct NavigationRoute: Equatable {
    let screen: Screen
    let depth: Int

    var identity: String {
        "\(key)|\(depth)"
    }

    var key: String {
        switch screen {
        case .welcome:
            return "welcome"
        case .createAccount:
            return "createAccount"
        case .restoreAccount:
            return "restoreAccount"
        case .addDevice:
            return "addDevice"
        case .chatList:
            return "chatList"
        case .newChat:
            return "newChat"
        case .newGroup:
            return "newGroup"
        case .createInvite:
            return "createInvite"
        case .joinInvite:
            return "joinInvite"
        case .settings:
            return "settings"
        case .chat(let chatId):
            return "chat:\(chatId)"
        case .directChatInfo(let chatId):
            return "directChatInfo:\(chatId)"
        case .groupDetails(let groupId):
            return "groupDetails:\(groupId)"
        case .deviceRoster:
            return "deviceRoster"
        case .awaitingDeviceApproval:
            return "awaitingDeviceApproval"
        case .deviceRevoked:
            return "deviceRevoked"
        }
    }

}

#if os(iOS)
struct UIKitRouteNavigationHost: UIViewControllerRepresentable {
    let routes: [NavigationRoute]
    let makeContent: (NavigationRoute) -> AnyView
    let onStackChanged: ([Screen]) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onStackChanged: onStackChanged)
    }

    func makeUIViewController(context: Context) -> UINavigationController {
        let navigationController = UINavigationController()
        configureNavigationController(navigationController)
        navigationController.setNavigationBarHidden(true, animated: false)
        navigationController.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.isEnabled = true
        context.coordinator.navigationController = navigationController
        return navigationController
    }

    func updateUIViewController(_ navigationController: UINavigationController, context: Context) {
        configureNavigationController(navigationController)
        navigationController.interactivePopGestureRecognizer?.delegate = context.coordinator
        navigationController.interactivePopGestureRecognizer?.isEnabled = true
        context.coordinator.update(
            navigationController: navigationController,
            routes: routes,
            makeContent: makeContent
        )
    }

    private func configureNavigationController(_ navigationController: UINavigationController) {
        navigationController.view.backgroundColor = .clear
        navigationController.navigationBar.prefersLargeTitles = false

        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.shadowColor = .clear
        navigationController.navigationBar.standardAppearance = appearance
        navigationController.navigationBar.scrollEdgeAppearance = appearance
        navigationController.navigationBar.compactAppearance = appearance
        if #available(iOS 15.0, *) {
            navigationController.navigationBar.compactScrollEdgeAppearance = appearance
        }
    }

    final class Coordinator: NSObject, UINavigationControllerDelegate, UIGestureRecognizerDelegate {
        private let onStackChanged: ([Screen]) -> Void
        private var currentRoutes: [NavigationRoute] = []
        private var applyingProgrammaticNavigation = false
        private var deferredUpdate: (routes: [NavigationRoute], makeContent: (NavigationRoute) -> AnyView)?
        weak var navigationController: UINavigationController?

        init(onStackChanged: @escaping ([Screen]) -> Void) {
            self.onStackChanged = onStackChanged
        }

        func update(
            navigationController: UINavigationController,
            routes: [NavigationRoute],
            makeContent: @escaping (NavigationRoute) -> AnyView
        ) {
            if isInteractivePopActive(in: navigationController)
                || isNavigationTransitionActive(in: navigationController) {
                deferredUpdate = (routes, makeContent)
                return
            }

            let existingControllers = routeControllers(in: navigationController)
            if existingControllers.map(\.route) == routes {
                refresh(controllers: existingControllers, makeContent: makeContent)
                currentRoutes = routes
                return
            }

            let nextControllers = routes.map { route in
                if let existing = existingControllers.first(where: { $0.route.identity == route.identity }) {
                    existing.route = route
                    existing.rootView = makeContent(route)
                    return existing
                }
                return RouteHostingController(route: route, rootView: makeContent(route))
            }

            let oldRoutes = existingControllers.map(\.route)
            let animated = shouldAnimate(from: oldRoutes, to: routes)
            applyingProgrammaticNavigation = animated
            currentRoutes = routes
            navigationController.setViewControllers(nextControllers, animated: animated)
            if !animated {
                applyingProgrammaticNavigation = false
            }
        }

        func navigationController(
            _ navigationController: UINavigationController,
            didShow viewController: UIViewController,
            animated: Bool
        ) {
            let visibleRoutes = routeControllers(in: navigationController).map(\.route)
            if applyingProgrammaticNavigation {
                applyingProgrammaticNavigation = false
                currentRoutes = visibleRoutes
                applyDeferredUpdateIfNeeded(navigationController: navigationController, visibleRoutes: visibleRoutes)
                return
            }
            guard visibleRoutes != currentRoutes else {
                applyDeferredUpdateIfNeeded(navigationController: navigationController, visibleRoutes: visibleRoutes)
                return
            }
            currentRoutes = visibleRoutes
            deferredUpdate = nil
            onStackChanged(visibleRoutes.dropFirst().map(\.screen))
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard gestureRecognizer === navigationController?.interactivePopGestureRecognizer else {
                return true
            }
            guard navigationController?.transitionCoordinator == nil else {
                return false
            }
            guard (navigationController?.viewControllers.count ?? currentRoutes.count) > 1 else {
                return false
            }
            let velocity = (gestureRecognizer as? UIPanGestureRecognizer)?.velocity(in: navigationController?.view)
            return (velocity?.x ?? 1) > 0
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            gestureRecognizer === navigationController?.interactivePopGestureRecognizer
                || otherGestureRecognizer === navigationController?.interactivePopGestureRecognizer
        }

        private func refresh(
            controllers: [RouteHostingController],
            makeContent: (NavigationRoute) -> AnyView
        ) {
            controllers.forEach { controller in
                controller.rootView = makeContent(controller.route)
            }
        }

        private func routeControllers(in navigationController: UINavigationController) -> [RouteHostingController] {
            navigationController.viewControllers.compactMap { $0 as? RouteHostingController }
        }

        private func isInteractivePopActive(in navigationController: UINavigationController) -> Bool {
            if navigationController.transitionCoordinator?.isInteractive == true {
                return true
            }
            switch navigationController.interactivePopGestureRecognizer?.state {
            case .began, .changed:
                return true
            default:
                return false
            }
        }

        private func isNavigationTransitionActive(in navigationController: UINavigationController) -> Bool {
            applyingProgrammaticNavigation || navigationController.transitionCoordinator != nil
        }

        private func applyDeferredUpdateIfNeeded(
            navigationController: UINavigationController,
            visibleRoutes: [NavigationRoute]
        ) {
            guard let deferredUpdate else { return }
            self.deferredUpdate = nil
            DispatchQueue.main.async { [weak self, weak navigationController] in
                guard let self, let navigationController else { return }
                guard self.routeControllers(in: navigationController).map(\.route) == visibleRoutes else {
                    return
                }
                self.update(
                    navigationController: navigationController,
                    routes: deferredUpdate.routes,
                    makeContent: deferredUpdate.makeContent
                )
            }
        }

        private func shouldAnimate(from oldRoutes: [NavigationRoute], to newRoutes: [NavigationRoute]) -> Bool {
            guard !oldRoutes.isEmpty, oldRoutes.first == newRoutes.first else {
                return false
            }
            if newRoutes.count == oldRoutes.count + 1 {
                return oldRoutes == Array(newRoutes.dropLast())
            }
            if newRoutes.count + 1 == oldRoutes.count {
                return newRoutes == Array(oldRoutes.dropLast())
            }
            return false
        }
    }
}

final class RouteHostingController: UIHostingController<AnyView> {
    var route: NavigationRoute

    init(route: NavigationRoute, rootView: AnyView) {
        self.route = route
        super.init(rootView: rootView)
        view.backgroundColor = .clear
        navigationItem.backButtonDisplayMode = .minimal
    }

    @MainActor
    @preconcurrency required dynamic init?(coder aDecoder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}
#endif

#if os(iOS)
struct OfflineStatusBanner: View {
    @Environment(\.irisPalette) private var palette

    let networkStatus: NetworkStatusSnapshot?
    @ObservedObject var nearbyService: IrisNearbyService
    let bluetoothEnabled: Bool
    let appSceneIsActive: Bool
    let foregroundedAt: Date
    let onTap: () -> Void
    @State private var now = Date()

    var body: some View {
        let text = bannerText(at: now)
        Button(action: onTap) {
            VStack(spacing: 0) {
                if let text {
                    // Glass capsule with a small accentAlt offline
                    // icon — the previous full-width orange bar
                    // screamed at the user every time a relay
                    // blipped. Carrying the warning in the icon
                    // alone keeps the banner readable without
                    // dominating the screen.
                    HStack(spacing: 6) {
                        Image(systemName: "wifi.slash")
                            .font(.system(size: 11, weight: .bold))
                            .foregroundStyle(palette.accentAlt)
                        Text(text)
                            .font(.system(.caption, design: .rounded, weight: .semibold))
                            .foregroundStyle(palette.textPrimary)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .irisGlassSurface(in: Capsule())
                    .overlay(
                        Capsule()
                            .strokeBorder(palette.border, lineWidth: 0.5)
                    )
                    .padding(.horizontal, 12)
                    .padding(.bottom, 4)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .accessibilityIdentifier("offlineStatusBanner")
                }
            }
            .clipped()
            .animation(.easeInOut(duration: 0.22), value: text)
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Open settings")
        .task(id: refreshToken) {
            await refreshBannerClockIfNeeded()
        }
    }

    private func bannerText(at date: Date) -> String? {
        offlineStatusBannerText(
            networkStatus: networkStatus,
            bluetoothOn: bluetoothEnabled,
            wifiOn: mobileWifiEnabled(nearbyService),
            appSceneIsActive: appSceneIsActive,
            foregroundedAt: foregroundedAt,
            now: date
        )
    }

    private var refreshToken: String {
        [
            appSceneIsActive ? "active" : "inactive",
            String(networkStatus?.connectedRelayCount ?? 0),
            String(networkStatus?.allRelaysOfflineSinceSecs ?? 0),
            networkStatus?.relayConnections.map { "\($0.url)=\($0.status)" }.joined(separator: ",") ?? "",
            String(foregroundedAt.timeIntervalSince1970),
            bluetoothEnabled ? "bt-on" : "bt-off",
            mobileWifiEnabled(nearbyService) ? "wifi-on" : "wifi-off",
        ].joined(separator: "|")
    }

    private func nextRefreshDate(at date: Date) -> Date? {
        guard appSceneIsActive,
              let status = networkStatus,
              offlineStatusBannerShouldConsiderOffline(status),
              let offlineSince = status.allRelaysOfflineSinceSecs else {
            return nil
        }
        let offlineDeadline = Date(
            timeIntervalSince1970: TimeInterval(offlineSince) + offlineBannerGraceInterval
        )
        let foregroundDeadline = foregroundedAt.addingTimeInterval(offlineBannerGraceInterval)
        let deadline = max(offlineDeadline, foregroundDeadline)
        return date < deadline ? deadline : nil
    }

    private func refreshBannerClockIfNeeded() async {
        let current = Date()
        await MainActor.run {
            now = current
        }
        guard let deadline = nextRefreshDate(at: current) else {
            return
        }
        let seconds = max(0, deadline.timeIntervalSince(current))
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        guard !Task.isCancelled else {
            return
        }
        await MainActor.run {
            now = Date()
        }
    }
}

func offlineStatusBannerText(
    networkStatus: NetworkStatusSnapshot?,
    bluetoothOn: Bool,
    wifiOn: Bool,
    appSceneIsActive: Bool,
    foregroundedAt: Date,
    now date: Date
) -> String? {
    guard appSceneIsActive,
          let status = networkStatus,
          offlineStatusBannerShouldConsiderOffline(status),
          let offlineSince = status.allRelaysOfflineSinceSecs,
          date.timeIntervalSince1970 - TimeInterval(offlineSince) >= offlineBannerGraceInterval,
          date.timeIntervalSince(foregroundedAt) >= offlineBannerGraceInterval else {
        return nil
    }
    return "Offline, Bluetooth \(bluetoothOn ? "on" : "off"), Wi-Fi \(wifiOn ? "on" : "off")"
}

private func offlineStatusBannerShouldConsiderOffline(_ status: NetworkStatusSnapshot) -> Bool {
    guard !status.relayUrls.isEmpty, status.connectedRelayCount == 0 else {
        return false
    }
    let relayStatuses = status.relayConnections
        .filter { status.relayUrls.contains($0.url) }
        .map(\.status)
    guard relayStatuses.count == status.relayUrls.count else {
        return false
    }
    return relayStatuses.allSatisfy { $0 == "offline" || $0 == "blocked" }
}

#endif
