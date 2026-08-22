#if os(iOS)
import Foundation
import SwiftUI
struct AppStoreUpdateNotice: Codable, Equatable, Sendable {
    let version: String
    let storeURL: URL
}
struct AppStoreLookupTransport: Sendable {
    let data: @Sendable (URLRequest) async throws -> (Data, URLResponse)
    static let live: Self = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 10
        let session = URLSession(configuration: config)
        return Self { try await session.data(for: $0) }
    }()
}
private struct SuccessfulAppStoreLookup: Codable {
    let checkedAt: Date
    let country: String
    let notice: AppStoreUpdateNotice?
}
private struct AppStoreLookupResponse: Decodable { let results: [AppStoreLookupItem] }
private struct AppStoreLookupItem: Decodable {
    let trackId: Int?
    let bundleId: String?
    let version: String?
    let trackViewUrl: URL?
}
@MainActor
final class AppStoreUpdateController: ObservableObject {
    private static let appID = 6_785_411_684
    private static let bundleID = "fi.siriusbusiness.irischat"
    private static let cacheKey = "ios.appStoreUpdate.successfulLookup.v1"
    private static let dismissedKey = "ios.appStoreUpdate.dismissedVersion"
    private static let successTTL: TimeInterval = 21_600
    private static let attemptBackoff: TimeInterval = 300
    @Published private(set) var notice: AppStoreUpdateNotice? = nil
    private let installedVersion: String
    private let defaults: UserDefaults
    private let now: () -> Date
    private let countryCode: () -> String?
    private let transport: AppStoreLookupTransport
    private var lookupTask: Task<Void, Never>?
    private var lastAttempt: (date: Date, country: String)?
    init(
        installedVersion: String = Bundle.main.object(
            forInfoDictionaryKey: "CFBundleShortVersionString"
        ) as? String ?? "0",
        defaults: UserDefaults = .standard,
        now: @escaping () -> Date = Date.init,
        countryCode: @escaping () -> String? = { Locale.current.region?.identifier },
        transport: AppStoreLookupTransport = .live
    ) {
        self.installedVersion = installedVersion
        self.defaults = defaults
        self.now = now
        self.countryCode = countryCode
        self.transport = transport
        if appStoreUpdatesEnabled(for: installedVersion),
           let country = currentCountry(), let cache = freshCache(for: country) {
            notice = visibleNotice(cache.notice)
        }
    }
    @discardableResult
    func checkIfNeeded() -> Task<Void, Never>? {
        guard appStoreUpdatesEnabled(for: installedVersion) else { notice = nil; return nil }
        guard let country = currentCountry() else { notice = nil; return nil }
        if let cache = freshCache(for: country) {
            notice = visibleNotice(cache.notice)
            return nil
        }
        notice = nil
        if let lookupTask { return lookupTask }
        let date = now()
        if let lastAttempt, lastAttempt.country == country {
            let age = date.timeIntervalSince(lastAttempt.date)
            if age >= 0 && age < Self.attemptBackoff { return nil }
        }
        lastAttempt = (date, country)
        lookupTask = Task { [weak self] in
            guard let self else { return }
            defer {
                lookupTask = nil
                checkIfNeeded()
            }
            var lookupCountry = country
            while true {
                await lookup(country: lookupCountry)
                guard let nextCountry = currentCountry(), nextCountry != lookupCountry else { break }
                notice = nil
                lookupCountry = nextCountry
                lastAttempt = (now(), nextCountry)
            }
        }
        return lookupTask
    }
    func dismissNotice() {
        guard let notice else { return }
        defaults.set(notice.version, forKey: Self.dismissedKey)
        self.notice = nil
    }
    private func lookup(country: String) async {
        var components = URLComponents(string: "https://itunes.apple.com/lookup")!
        components.queryItems = [.init(name: "id", value: String(Self.appID)), .init(name: "country", value: country)]
        guard let url = components.url else { return }
        let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 10)
        do {
            let (data, response) = try await transport.data(request)
            guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else { return }
            let results = try JSONDecoder().decode(AppStoreLookupResponse.self, from: data).results
            let candidate: AppStoreUpdateNotice?
            if results.isEmpty {
                candidate = nil
            } else {
                guard let item = results.first(where: {
                    $0.trackId == Self.appID && $0.bundleId == Self.bundleID
                }), let version = item.version, appStoreNumericVersion(version) != nil,
                    let url = item.trackViewUrl, Self.isAppleStoreURL(url) else { return }
                candidate = appStoreVersionIsNewer(version, than: installedVersion)
                    ? .init(version: version, storeURL: url) : nil
            }
            guard currentCountry() == country else { return }
            let cache = SuccessfulAppStoreLookup(checkedAt: now(), country: country, notice: candidate)
            if let data = try? JSONEncoder().encode(cache) { defaults.set(data, forKey: Self.cacheKey) }
            notice = visibleNotice(candidate)
        } catch {} // Advisory lookup failures are intentionally silent.
    }
    private func freshCache(for country: String) -> SuccessfulAppStoreLookup? {
        guard let data = defaults.data(forKey: Self.cacheKey),
              let cache = try? JSONDecoder().decode(SuccessfulAppStoreLookup.self, from: data),
              cache.country == country else { return nil }
        let age = now().timeIntervalSince(cache.checkedAt)
        return age >= 0 && age < Self.successTTL ? cache : nil
    }
    private func visibleNotice(_ candidate: AppStoreUpdateNotice?) -> AppStoreUpdateNotice? {
        guard let candidate, appStoreVersionIsNewer(candidate.version, than: installedVersion),
              defaults.string(forKey: Self.dismissedKey) != candidate.version else { return nil }
        return candidate
    }
    private func currentCountry() -> String? {
        guard let value = countryCode()?.uppercased(), value.utf8.count == 2,
              value.utf8.allSatisfy({ $0 >= 65 && $0 <= 90 }) else { return nil }
        return value
    }
    private static func isAppleStoreURL(_ url: URL) -> Bool {
        guard url.scheme?.lowercased() == "https", let host = url.host?.lowercased() else { return false }
        return host == "apps.apple.com" || host.hasSuffix(".apps.apple.com")
            || host == "itunes.apple.com" || host.hasSuffix(".itunes.apple.com")
    }
}
func appStoreVersionIsNewer(_ candidate: String, than installed: String) -> Bool {
    guard let lhs = appStoreNumericVersion(candidate), let rhs = appStoreNumericVersion(installed) else { return false }
    for index in 0..<max(lhs.count, rhs.count) {
        let left = index < lhs.count ? lhs[index] : "0"
        let right = index < rhs.count ? rhs[index] : "0"
        let comparison = compareAppStoreNumericComponent(left, right)
        if comparison != 0 { return comparison > 0 }
    }
    return false
}
func appStoreUpdatesEnabled(for installedVersion: String) -> Bool {
    guard let major = appStoreNumericVersion(installedVersion)?.first else { return false }
    return compareAppStoreNumericComponent(major, "2000") >= 0
}
private func appStoreNumericVersion(_ value: String) -> [String]? {
    let parts = value.split(separator: ".", omittingEmptySubsequences: false)
    guard !parts.isEmpty else { return nil }
    var numbers: [String] = []
    for part in parts {
        guard !part.isEmpty, part.utf8.allSatisfy({ (48...57).contains($0) }) else { return nil }
        let normalized = part.drop(while: { $0 == "0" })
        numbers.append(normalized.isEmpty ? "0" : String(normalized))
    }
    return numbers
}
private func compareAppStoreNumericComponent(_ lhs: String, _ rhs: String) -> Int {
    if lhs.count != rhs.count { return lhs.count > rhs.count ? 1 : -1 }
    if lhs == rhs { return 0 }
    return lhs > rhs ? 1 : -1
}
struct AppStoreUpdateBanner: View {
    @ObservedObject var updates: AppStoreUpdateController
    @Environment(\.openURL) private var openURL
    @Environment(\.irisPalette) private var palette
    var body: some View {
        if let notice = updates.notice {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) { message(notice); Spacer(minLength: 8); actions(notice) }
                VStack(alignment: .leading, spacing: 8) { message(notice); actions(notice) }
            }
            .padding(.leading, 14).padding(.trailing, 6).frame(maxWidth: .infinity, alignment: .leading)
            .background(palette.panel)
            .overlay(alignment: .bottom) { Divider().overlay(palette.border) }
            .accessibilityIdentifier("appStoreUpdateBanner")
            .irisReportNavigationStatusBannerHeight()
        }
    }
    private func message(_ notice: AppStoreUpdateNotice) -> some View {
        Label("Version \(notice.version) is available", systemImage: "arrow.down.circle.fill")
            .font(.callout.weight(.semibold)).foregroundStyle(palette.textPrimary)
            .fixedSize(horizontal: false, vertical: true)
    }
    private func actions(_ notice: AppStoreUpdateNotice) -> some View {
        HStack(spacing: 4) {
            Button("Update") { openURL(notice.storeURL) }
                .font(.callout.weight(.semibold)).frame(minHeight: 44)
                .accessibilityLabel("Update to version \(notice.version)")
                .accessibilityHint("Opens the App Store")
            Button { updates.dismissNotice() } label: {
                Image(systemName: "xmark").frame(width: 44, height: 44)
            }.accessibilityLabel("Dismiss version \(notice.version) update")
        }.buttonStyle(.irisPlain)
    }
}
#endif
