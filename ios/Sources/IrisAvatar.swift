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
    private static let cache: NSCache<NSString, NSString> = {
        let cache = NSCache<NSString, NSString>()
        cache.countLimit = 1_000
        return cache
    }()

    static func key(originalSrc: String, preferences: PreferencesSnapshot, pixelSize: UInt32) -> NSString {
        var hasher = Hasher()
        hasher.combine(preferences)
        return "\(pixelSize)|\(hasher.finalize())|\(originalSrc)" as NSString
    }

    static func value(for key: NSString) -> String? {
        cache.object(forKey: key).map(String.init)
    }

    static func store(_ value: String, for key: NSString) {
        cache.setObject(value as NSString, forKey: key)
    }
}

func irisHttpAvatarURL(
    _ rawURL: String?,
    preferences: PreferencesSnapshot,
    pixelSize: CGFloat
) -> String? {
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
    let proxied = proxiedImageUrl(
        originalSrc: trimmed,
        preferences: preferences,
        width: dim,
        height: dim,
        square: true
    )
    IrisAvatarProxyURLCache.store(proxied, for: cacheKey)
    return proxied
}

enum IrisAvatarImageSource: Equatable {
    case hashtree(String)
    case http(String)

    var cacheKey: String {
        switch self {
        case .hashtree(let nhash): return "htree:\(nhash)"
        case .http(let url): return "http:\(url)"
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
    if let nhash = irisHtreeNhash(from: pictureUrl) {
        return .hashtree(nhash)
    }
    guard let preferences,
          let httpURL = irisHttpAvatarURL(pictureUrl, preferences: preferences, pixelSize: pixelSize) else {
        return nil
    }
    return .http(httpURL)
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

        let loaded: Data?
        switch source {
        case .hashtree(let nhash):
            guard let manager else {
                avatarImage = nil
                return
            }
            loaded = await manager.resolveHashtreePictureBytes(nhash: nhash)
        case .http(let urlString):
            guard let url = URL(string: urlString) else {
                avatarImage = nil
                return
            }
            if let response = try? await URLSession.shared.data(from: url) {
                loaded = response.0
            } else {
                loaded = nil
            }
        }

        guard imageSourceKey == key else { return }
        guard let loaded, !loaded.isEmpty else {
            avatarImage = nil
            return
        }
        guard let image = makeIrisAvatarImage(data: loaded, maxPixelSize: Int(ceil(size * 3))) else {
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
