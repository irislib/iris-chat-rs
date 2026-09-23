import Foundation
import ImageIO
import SwiftUI
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif

enum IrisAttachmentCategory: String {
    case image = "Image"
    case video = "Video"
    case audio = "Audio"
    case archive = "Archive"
    case document = "Document"
    case file = "File"

    var systemIcon: String {
        switch self {
        case .image:
            return "photo.fill"
        case .video:
            return "play.rectangle.fill"
        case .audio:
            return "waveform"
        case .archive:
            return "archivebox.fill"
        case .document:
            return "doc.text.fill"
        case .file:
            return "doc.fill"
        }
    }
}

let irisImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
let irisVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
let irisAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
let irisArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
let irisDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

func irisAttachmentCategory(from filename: String) -> IrisAttachmentCategory {
    let ext = filename
        .split(separator: ".")
        .last
        .map { String($0).lowercased() }

    guard let extensionValue = ext, !extensionValue.isEmpty else {
        return .file
    }

    if irisImageExtensions.contains(extensionValue) {
        return .image
    }
    if irisVideoExtensions.contains(extensionValue) {
        return .video
    }
    if irisAudioExtensions.contains(extensionValue) {
        return .audio
    }
    if irisArchiveExtensions.contains(extensionValue) {
        return .archive
    }
    if irisDocumentExtensions.contains(extensionValue) {
        return .document
    }
    return .file
}

struct IrisSelectedAttachmentChip: View {
    @Environment(\.irisPalette) private var palette
    let attachment: StagedAttachment
    let enabled: Bool
    let onRemove: () -> Void

    @State private var thumbnail: PlatformImage?

    private static let thumbSize: CGFloat = 56

    var body: some View {
        let category = irisAttachmentCategory(from: attachment.filename)

        if category == .image {
            imageChip
                .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
                .task(id: attachment.path) {
                    await loadThumbnailIfNeeded()
                }
        } else {
            fileChip(category: category)
        }
    }

    private var imageChip: some View {
        ZStack(alignment: .topTrailing) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(palette.panel)
                if let thumbnail {
                    Image(platformImage: thumbnail)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: "photo.fill")
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(palette.muted)
                }
            }
            .frame(width: Self.thumbSize, height: Self.thumbSize)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            Button(action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.palette)
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(palette.textPrimary, palette.panel)
                    .padding(4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .disabled(!enabled)
            .accessibilityIdentifier("chatSelectedAttachmentRemove")
        }
    }

    private func fileChip(category: IrisAttachmentCategory) -> some View {
        HStack(spacing: 7) {
            Image(systemName: category.systemIcon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(palette.muted)
            VStack(alignment: .leading, spacing: 2) {
                Text(attachment.filename)
                    .font(.system(.subheadline, design: .rounded, weight: .semibold))
                    .foregroundStyle(palette.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(category.rawValue)
                    .font(.system(.caption, design: .rounded, weight: .medium))
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
            }
            .frame(maxWidth: 220, alignment: .leading)
            Button(action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(enabled ? palette.muted : palette.muted.opacity(0.45))
            }
            .buttonStyle(.irisPlain)
            .disabled(!enabled)
            .accessibilityIdentifier("chatSelectedAttachmentRemove")
        }
        .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
        .padding(.leading, 11)
        .padding(.trailing, 7)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(palette.panel)
        )
    }

    @MainActor
    private func loadThumbnailIfNeeded() async {
        if thumbnail != nil { return }
        if let cached = IrisStagedThumbnailCache.image(for: attachment.path) {
            thumbnail = cached
            return
        }
        let path = attachment.path
        let image: PlatformImage? = await Task.detached(priority: .userInitiated) {
            irisLoadStagedThumbnail(path: path)
        }.value
        guard let image else { return }
        IrisStagedThumbnailCache.store(image, for: attachment.path)
        thumbnail = image
    }
}

enum IrisStagedThumbnailCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 32
        cache.totalCostLimit = 16 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: irisAvatarImageCost(image))
    }
}

func irisLoadStagedThumbnail(path: String) -> PlatformImage? {
    let url = URL(fileURLWithPath: path) as CFURL
    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithURL(url, sourceOptions as CFDictionary) else {
        return nil
    }
    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: 256
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(
        source,
        0,
        thumbnailOptions as CFDictionary
    ) else {
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
