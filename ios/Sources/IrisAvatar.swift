import Foundation
import ImageIO
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

func irisHtreeNhash(from rawURL: String?) -> String? {
    guard let rawURL else { return nil }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    let prefix: String
    if trimmed.hasPrefix("htree://") {
        prefix = "htree://"
    } else if trimmed.hasPrefix("nhash://") {
        prefix = "nhash://"
    } else {
        return nil
    }
    let remainder = trimmed.dropFirst(prefix.count)
    return remainder.split(separator: "/", maxSplits: 1).first.map(String.init)
}

func irisCanOpenProfilePicture(_ rawURL: String?) -> Bool {
    guard let rawURL else { return false }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return false }
    return irisHtreeNhash(from: trimmed) != nil
        || trimmed.hasPrefix("http://")
        || trimmed.hasPrefix("https://")
}

private enum IrisAvatarProxyURLCache {
    private static let cache: NSCache<NSString, NSArray> = {
        let cache = NSCache<NSString, NSArray>()
        cache.countLimit = 1_000
        return cache
    }()

    static func key(originalSrc: String, preferences: PreferencesSnapshot, pixelSize: UInt32) -> NSString {
        var hasher = Hasher()
        hasher.combine(preferences)
        return "\(pixelSize)|\(hasher.finalize())|\(originalSrc)" as NSString
    }

    static func value(for key: NSString) -> [String]? {
        cache.object(forKey: key) as? [String]
    }

    static func store(_ value: [String], for key: NSString) {
        cache.setObject(value as NSArray, forKey: key)
    }
}

func irisHttpAvatarURLs(
    _ rawURL: String?,
    preferences: PreferencesSnapshot,
    pixelSize: CGFloat
) -> [String]? {
    guard let rawURL else { return nil }
    let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") else {
        return nil
    }
    let dim = UInt32(max(1, pixelSize.rounded()))
    let cacheKey = IrisAvatarProxyURLCache.key(
        originalSrc: trimmed,
        preferences: preferences,
        pixelSize: dim
    )
    if let cached = IrisAvatarProxyURLCache.value(for: cacheKey) {
        return cached
    }
    let urls = imageLoadUrls(
        originalSrc: trimmed,
        preferences: preferences,
        width: dim,
        height: dim,
        square: true
    )
    IrisAvatarProxyURLCache.store(urls, for: cacheKey)
    return urls
}

enum IrisAvatarImageSource: Equatable {
    case hashtree(String)
    case http([String], originalURL: String, allowOriginalRedirects: Bool)

    var cacheKey: String {
        switch self {
        case .hashtree(let nhash): return "htree:\(nhash)"
        case .http(let urls, _, let redirects): return "http:\(redirects)|\(urls.joined(separator: "\u{1F}"))"
        }
    }
}

enum IrisAvatarImageCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 160
        cache.totalCostLimit = 24 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: irisAvatarImageCost(image))
    }
}

func makeIrisAvatarImage(data: Data, maxPixelSize: Int) -> PlatformImage? {
    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions as CFDictionary) else {
        return nil
    }

    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: max(1, maxPixelSize)
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) else {
        return nil
    }

    #if os(iOS)
    return PlatformImage(cgImage: cgImage)
    #elseif os(macOS)
    return PlatformImage(
        cgImage: cgImage,
        size: NSSize(width: cgImage.width, height: cgImage.height)
    )
    #else
    return nil
    #endif
}

func loadIrisHttpAvatarImage(
    urls: [String],
    originalURL: String,
    allowOriginalRedirects: Bool = false,
    maxPixelSize: Int,
    session: URLSession = .shared
) async -> PlatformImage? {
    for urlString in urls {
        guard !Task.isCancelled else { return nil }
        guard let url = URL(string: urlString) else { continue }
        do {
            let redirectDelegate = allowOriginalRedirects && urlString == originalURL
                ? nil : IrisImageProxyRedirectDelegate(origin: url)
            let (data, response) = try await session.data(for: URLRequest(url: url), delegate: redirectDelegate)
            guard !Task.isCancelled else { return nil }
            guard let response = response as? HTTPURLResponse,
                  (200..<300).contains(response.statusCode),
                  let image = makeIrisAvatarImage(data: data, maxPixelSize: maxPixelSize) else {
                continue
            }
            return image
        } catch {
            if Task.isCancelled || (error as? URLError)?.code == .cancelled {
                return nil
            }
        }
    }
    return nil
}

private final class IrisImageProxyRedirectDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    private let origin: URL
    private var redirectCount = 0

    init(origin: URL) { self.origin = origin }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        guard redirectCount < 5, let target = request.url,
              target.scheme?.lowercased() == origin.scheme?.lowercased(),
              target.host?.lowercased() == origin.host?.lowercased(),
              effectivePort(target) == effectivePort(origin) else {
            completionHandler(nil)
            return
        }
        redirectCount += 1
        completionHandler(request)
    }

    private func effectivePort(_ url: URL) -> Int? {
        url.port ?? (url.scheme?.lowercased() == "https" ? 443 : 80)
    }
}

func irisAvatarImageCost(_ image: PlatformImage) -> Int {
    #if os(iOS)
    let width = max(1, Int(image.size.width * image.scale))
    let height = max(1, Int(image.size.height * image.scale))
    return width * height * 4
    #elseif os(macOS)
    let width = max(1, Int(image.size.width))
    let height = max(1, Int(image.size.height))
    return width * height * 4
    #else
    return 1
    #endif
}

func irisAvatarImageSource(
    pictureUrl: String?,
    preferences: PreferencesSnapshot?,
    pixelSize: CGFloat
) -> IrisAvatarImageSource? {
    guard let pictureUrl else { return nil }
    if let nhash = irisHtreeNhash(from: pictureUrl) {
        return .hashtree(nhash)
    }
    guard let preferences,
          let httpURLs = irisHttpAvatarURLs(pictureUrl, preferences: preferences, pixelSize: pixelSize),
          !httpURLs.isEmpty else {
        return nil
    }
    return .http(
        httpURLs,
        originalURL: pictureUrl.trimmingCharacters(in: .whitespacesAndNewlines),
        allowOriginalRedirects: !preferences.imageProxyEnabled || preferences.imageProxyFallbackEnabled
    )
}

struct IrisAvatar: View {
    @Environment(\.irisPalette) private var palette

    let label: String
    let size: CGFloat
    let emphasize: Bool
    let pictureUrl: String?
    let preferences: PreferencesSnapshot?
    let manager: AppManager?
    let loadedImageIdentifier: String?

    @State private var avatarImage: PlatformImage?

    init(
        label: String,
        size: CGFloat = 42,
        emphasize: Bool = false,
        pictureUrl: String? = nil,
        preferences: PreferencesSnapshot? = nil,
        manager: AppManager? = nil,
        loadedImageIdentifier: String? = nil
    ) {
        self.label = label
        self.size = size
        self.emphasize = emphasize
        self.pictureUrl = pictureUrl
        self.preferences = preferences
        self.manager = manager
        self.loadedImageIdentifier = loadedImageIdentifier
        let source = irisAvatarImageSource(
            pictureUrl: pictureUrl,
            preferences: preferences,
            pixelSize: size * 2
        )
        _avatarImage = State(initialValue: source.flatMap { IrisAvatarImageCache.image(for: $0.cacheKey) })
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(emphasize ? palette.accent : palette.panelAlt)
                .overlay(Circle().stroke(palette.border, lineWidth: 1))

            if let avatarImage {
                Image(platformImage: avatarImage)
                    .resizable()
                    .scaledToFill()
                    .clipShape(Circle())
                if let loadedImageIdentifier {
                    Color.clear
                        .frame(width: 1, height: 1)
                        .accessibilityIdentifier(loadedImageIdentifier)
                        .allowsHitTesting(false)
                }
            } else {
                avatarInitial
            }
        }
        .frame(width: size, height: size)
        .task(id: imageSourceKey) {
            await loadAvatarImage()
        }
    }

    private var imageSource: IrisAvatarImageSource? {
        irisAvatarImageSource(
            pictureUrl: pictureUrl,
            preferences: preferences,
            pixelSize: size * 2
        )
    }

    private var imageSourceKey: String? {
        imageSource?.cacheKey
    }

    private func loadAvatarImage() async {
        guard let source = imageSource else {
            avatarImage = nil
            return
        }
        let key = source.cacheKey
        if let cached = IrisAvatarImageCache.image(for: key) {
            avatarImage = cached
            return
        }

        let image: PlatformImage?
        let maxPixelSize = Int(ceil(size * 3))
        switch source {
        case .hashtree(let nhash):
            if let manager, let data = await manager.resolveHashtreePictureBytes(nhash: nhash) {
                image = makeIrisAvatarImage(data: data, maxPixelSize: maxPixelSize)
            } else {
                image = nil
            }
        case .http(let urls, let originalURL, let allowOriginalRedirects):
            image = await loadIrisHttpAvatarImage(
                urls: urls, originalURL: originalURL,
                allowOriginalRedirects: allowOriginalRedirects, maxPixelSize: maxPixelSize
            )
        }

        guard !Task.isCancelled, imageSourceKey == key else { return }
        guard let image else {
            avatarImage = nil
            return
        }
        IrisAvatarImageCache.store(image, for: key)
        avatarImage = image
    }

    private var avatarInitial: some View {
        Text(String((label.trimmingCharacters(in: .whitespacesAndNewlines).first ?? "?")).uppercased())
            .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
            .foregroundStyle(emphasize ? palette.onAccent : palette.textPrimary)
    }
}
