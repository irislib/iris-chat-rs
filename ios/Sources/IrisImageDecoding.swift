import Foundation
import ImageIO
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// ImageIO's immediate-cache option decompresses pixels synchronously. Keep
// that work off the UI actor, including when the original bytes are cached.
func loadIrisDecodedImage(
    _ decode: @escaping @Sendable () -> PlatformImage?
) async -> PlatformImage? {
    guard !Task.isCancelled else { return nil }
    let task = Task.detached(priority: .userInitiated) {
        guard !Task.isCancelled else { return nil as PlatformImage? }
        return decode()
    }
    return await withTaskCancellationHandler {
        let image = await task.value
        return Task.isCancelled ? nil : image
    } onCancel: {
        task.cancel()
    }
}

func loadIrisAvatarImage(data: Data, maxPixelSize: Int) async -> PlatformImage? {
    await loadIrisDecodedImage {
        makeIrisDecodedImage(data: data, maxPixelSize: maxPixelSize)
    }
}

func makeIrisDecodedImage(data: Data, maxPixelSize: Int? = nil) -> PlatformImage? {
    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions as CFDictionary) else {
        return nil
    }

    let pixelSize: Int
    if let maxPixelSize {
        pixelSize = max(1, maxPixelSize)
    } else {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int else { return nil }
        pixelSize = max(1, max(width, height))
    }

    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: pixelSize
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

func loadChatAttachmentPreviewImage(data: Data, filename: String) async -> PlatformImage? {
    await loadIrisDecodedImage {
        makeChatAttachmentPreviewImage(data: data, filename: filename)
    }
}

func makeChatAttachmentPreviewImage(data: Data, filename: String) -> PlatformImage? {
    guard !isAnimatedImage(data: data, filename: filename) else {
        return nil
    }

    let sourceOptions: [CFString: Any] = [
        kCGImageSourceShouldCache: false
    ]
    guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions as CFDictionary) else {
        return nil
    }

    let maxPixelSize = 512
    let thumbnailOptions: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceShouldCacheImmediately: true,
        kCGImageSourceThumbnailMaxPixelSize: maxPixelSize
    ]
    let fullImageOptions: [CFString: Any] = [
        kCGImageSourceShouldCacheImmediately: true
    ]
    guard let cgImage = CGImageSourceCreateThumbnailAtIndex(
        source,
        0,
        thumbnailOptions as CFDictionary
    ) ?? CGImageSourceCreateImageAtIndex(source, 0, fullImageOptions as CFDictionary) else {
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
