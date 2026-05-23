import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

enum ChatAttachmentCategory: String {
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

let chatImageExtensions: Set<String> = ["gif", "heic", "heif", "jpeg", "jpg", "png", "webp", "bmp", "tif", "tiff", "avif"]
let chatVideoExtensions: Set<String> = ["avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "webm", "wmv", "ts", "mts", "m2ts"]
let chatAudioExtensions: Set<String> = ["aac", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"]
let chatArchiveExtensions: Set<String> = ["7z", "apk", "arc", "arj", "bz2", "cpio", "gz", "jar", "rar", "tar", "xz", "zip"]
let chatDocumentExtensions: Set<String> = ["csv", "doc", "docm", "docx", "json", "key", "md", "odf", "odg", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "tex", "txt", "xhtml", "xls", "xlsx", "xml", "yaml", "yml"]

func chatAttachmentCategory(from filename: String) -> ChatAttachmentCategory {
    let ext = filename
        .split(separator: ".")
        .last
        .map { String($0).lowercased() }

    guard let extensionValue = ext, !extensionValue.isEmpty else {
        return .file
    }

    if chatImageExtensions.contains(extensionValue) {
        return .image
    }
    if chatVideoExtensions.contains(extensionValue) {
        return .video
    }
    if chatAudioExtensions.contains(extensionValue) {
        return .audio
    }
    if chatArchiveExtensions.contains(extensionValue) {
        return .archive
    }
    if chatDocumentExtensions.contains(extensionValue) {
        return .document
    }
    return .file
}

func chatAttachmentCategory(for attachment: MessageAttachmentSnapshot) -> ChatAttachmentCategory {
    if attachment.isImage {
        return .image
    }
    if attachment.isVideo {
        return .video
    }
    if attachment.isAudio {
        return .audio
    }
    return chatAttachmentCategory(from: attachment.filename)
}

enum ChatAttachmentPreviewImageCache {
    private static let cache: NSCache<NSString, PlatformImage> = {
        let cache = NSCache<NSString, PlatformImage>()
        cache.countLimit = 120
        cache.totalCostLimit = 48 * 1024 * 1024
        return cache
    }()

    static func image(for key: String) -> PlatformImage? {
        cache.object(forKey: key as NSString)
    }

    static func store(_ image: PlatformImage, for key: String) {
        cache.setObject(image, forKey: key as NSString, cost: imagePreviewCost(image))
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

func imagePreviewCost(_ image: PlatformImage) -> Int {
    #if os(iOS)
    let width = max(1, Int(image.size.width * image.scale))
    let height = max(1, Int(image.size.height * image.scale))
    let pixels = width * height
    return pixels * 4
    #elseif os(macOS)
    let width = max(1, Int(image.size.width))
    let height = max(1, Int(image.size.height))
    let pixels = width * height
    return pixels * 4
    #else
    return 1
    #endif
}

struct ChatImageAlbumView: View {
    let attachments: [MessageAttachmentSnapshot]
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: (MessageAttachmentSnapshot) -> Void

    private let albumWidth: CGFloat = 232
    private let gap: CGFloat = 2

    var body: some View {
        switch attachments.count {
        case 0:
            EmptyView()
        case 1:
            cell(attachments[0], width: 220, height: 150, contentMode: .fill)
        case 2:
            HStack(spacing: gap) {
                cell(attachments[0], width: (albumWidth - gap) / 2, height: 150, contentMode: .fill)
                cell(attachments[1], width: (albumWidth - gap) / 2, height: 150, contentMode: .fill)
            }
        case 3:
            HStack(spacing: gap) {
                cell(attachments[0], width: albumWidth * 0.58 - gap / 2, height: albumWidth * 0.86, contentMode: .fill)
                VStack(spacing: gap) {
                    cell(attachments[1], width: albumWidth * 0.42 - gap / 2, height: (albumWidth * 0.86 - gap) / 2, contentMode: .fill)
                    cell(attachments[2], width: albumWidth * 0.42 - gap / 2, height: (albumWidth * 0.86 - gap) / 2, contentMode: .fill)
                }
            }
        default:
            let cellSize = (albumWidth - gap) / 2
            VStack(spacing: gap) {
                HStack(spacing: gap) {
                    cell(attachments[0], width: cellSize, height: cellSize, contentMode: .fill)
                    cell(attachments[1], width: cellSize, height: cellSize, contentMode: .fill)
                }
                HStack(spacing: gap) {
                    cell(attachments[2], width: cellSize, height: cellSize, contentMode: .fill)
                    overflowCell(at: 3, width: cellSize, height: cellSize)
                }
            }
        }
    }

    @ViewBuilder
    private func cell(
        _ attachment: MessageAttachmentSnapshot,
        width: CGFloat,
        height: CGFloat,
        contentMode: ContentMode
    ) -> some View {
        ChatAlbumImageCell(
            attachment: attachment,
            isOutgoing: isOutgoing,
            width: width,
            height: height,
            downloadAttachment: downloadAttachment,
            onOpenImage: onOpenImage,
            onForward: { onForward(attachment) }
        )
    }

    @ViewBuilder
    private func overflowCell(at index: Int, width: CGFloat, height: CGFloat) -> some View {
        ChatAlbumImageCell(
            attachment: attachments[index],
            isOutgoing: isOutgoing,
            width: width,
            height: height,
            downloadAttachment: downloadAttachment,
            onOpenImage: onOpenImage,
            onForward: { onForward(attachments[index]) }
        )
        .overlay {
            if attachments.count > 4 {
                ZStack {
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(Color.black.opacity(0.45))
                    Text("+\(attachments.count - 4)")
                        .font(.system(size: 24, weight: .bold, design: .rounded))
                        .foregroundStyle(.white)
                }
                .frame(width: width, height: height)
                .allowsHitTesting(false)
            }
        }
    }
}

struct ChatAlbumImageCell: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let width: CGFloat
    let height: CGFloat
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: () -> Void

    @State private var localImageData: Data?
    @State private var localPreviewImage: PlatformImage?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false

    var body: some View {
        ZStack {
            Rectangle()
                .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
            if let localPreviewImage {
                Image(platformImage: localPreviewImage)
                    .resizable()
                    .scaledToFill()
            } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                IrisAnimatedImageDataView(data: localImageData)
                    .allowsHitTesting(false)
            } else if isLoadingImage {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: failedImageLoad ? "exclamationmark.triangle.fill" : "photo.fill")
                    .font(.system(size: 22, weight: .semibold))
                    .opacity(0.72)
            }
        }
        .frame(width: width, height: height)
        .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        .contentShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        .onTapGesture {
            if let localImageData {
                onOpenImage(localImageData, attachment)
            } else {
                Task {
                    await loadImageIfNeeded()
                    if let localImageData {
                        onOpenImage(localImageData, attachment)
                    }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(attachment.filename)
        .contextMenu {
            Button("Forward", action: onForward)
            Button("Copy link") {
                PlatformClipboard.setString(attachment.htreeUrl)
            }
        }
        .task(id: attachment.htreeUrl) {
            await loadImageIfNeeded()
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard localImageData == nil, !isLoadingImage else { return }
        isLoadingImage = true
        failedImageLoad = false
        if let cached = ChatAttachmentPreviewImageCache.image(for: attachment.htreeUrl) {
            localPreviewImage = cached
        }
        guard let data = await downloadAttachment(attachment) else {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        let isAnimated = isAnimatedImage(data: data, filename: attachment.filename)
        if !isAnimated, localPreviewImage == nil {
            if let preview = makeChatAttachmentPreviewImage(data: data, filename: attachment.filename) {
                ChatAttachmentPreviewImageCache.store(preview, for: attachment.htreeUrl)
                localPreviewImage = preview
            } else {
                isLoadingImage = false
                failedImageLoad = true
                return
            }
        }
        localImageData = data
        isLoadingImage = false
    }
}

struct ChatAttachmentView: View {
    @Environment(\.irisPalette) private var palette

    let attachment: MessageAttachmentSnapshot
    let isOutgoing: Bool
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let openAttachment: (MessageAttachmentSnapshot) async -> Void
    let onOpenImage: (Data, MessageAttachmentSnapshot) -> Void
    let onForward: () -> Void

    @State private var localImageData: Data?
    @State private var localPreviewImage: PlatformImage?
    @State private var isLoadingImage = false
    @State private var failedImageLoad = false
    @State private var isOpeningAttachment = false

    var body: some View {
        if attachment.isImage {
            ZStack {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                if let localPreviewImage {
                    Image(platformImage: localPreviewImage)
                        .resizable()
                        .scaledToFill()
                } else if let localImageData, isAnimatedImage(data: localImageData, filename: attachment.filename) {
                    IrisAnimatedImageDataView(data: localImageData)
                        .allowsHitTesting(false)
                } else if isLoadingImage {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: failedImageLoad ? "exclamationmark.triangle.fill" : "photo.fill")
                        .font(.system(size: 28, weight: .semibold))
                        .opacity(0.72)
                }
            }
            .frame(width: 220, height: 150)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .onTapGesture {
                if let localImageData {
                    onOpenImage(localImageData, attachment)
                } else {
                    Task {
                        await loadImageIfNeeded()
                        if let localImageData {
                            onOpenImage(localImageData, attachment)
                        }
                    }
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(attachment.filename)
            .contextMenu {
                Button("Forward", action: onForward)
                Button("Copy link") {
                    PlatformClipboard.setString(attachment.htreeUrl)
                }
            }
            .task(id: attachment.htreeUrl) {
                await loadImageIfNeeded()
            }
        } else {
            let category = chatAttachmentCategory(for: attachment)

            Button {
                Task {
                    guard !isOpeningAttachment else { return }
                    isOpeningAttachment = true
                    await openAttachment(attachment)
                    isOpeningAttachment = false
                }
            } label: {
                HStack(spacing: 8) {
                    if isOpeningAttachment {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 20, height: 20)
                    } else {
                        Image(systemName: category.systemIcon)
                            .font(.system(size: 15, weight: .semibold))
                            .frame(width: 20, height: 20)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(attachment.filename)
                            .font(.system(.subheadline, design: .rounded, weight: .semibold))
                            .lineLimit(1)
                        Text(category.rawValue)
                            .font(.system(.caption, design: .rounded, weight: .medium))
                            .foregroundStyle(isOutgoing ? palette.onBubbleMine.opacity(0.6) : palette.onBubbleTheirs.opacity(0.6))
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
                )
            }
            .buttonStyle(.irisPlain)
            .disabled(isOpeningAttachment)
            .contextMenu {
                Button("Forward", action: onForward)
                Button("Copy link") {
                    PlatformClipboard.setString(attachment.htreeUrl)
                }
            }
            .accessibilityLabel("\(category.rawValue), \(attachment.filename)")
        }
    }

    @MainActor
    private func loadImageIfNeeded() async {
        guard localImageData == nil, !isLoadingImage else {
            return
        }
        isLoadingImage = true
        failedImageLoad = false
        if let cached = ChatAttachmentPreviewImageCache.image(for: attachment.htreeUrl) {
            localPreviewImage = cached
        }
        guard let data = await downloadAttachment(attachment) else {
            isLoadingImage = false
            failedImageLoad = true
            return
        }
        let isAnimated = isAnimatedImage(data: data, filename: attachment.filename)
        if !isAnimated, localPreviewImage == nil {
            guard let preview = makeChatAttachmentPreviewImage(data: data, filename: attachment.filename) else {
                isLoadingImage = false
                failedImageLoad = true
                return
            }
            ChatAttachmentPreviewImageCache.store(preview, for: attachment.htreeUrl)
            localPreviewImage = preview
        }
        localImageData = data
        isLoadingImage = false
    }

}

struct ImageViewerItem: Identifiable, Equatable {
    let id = UUID()
    let attachments: [MessageAttachmentSnapshot]
    let initialIndex: Int
    let initialData: Data
    let senderName: String
    let createdAtSecs: UInt64
    let downloadAttachment: (MessageAttachmentSnapshot) async -> Data?
    let forwardableTextFor: (MessageAttachmentSnapshot) -> String

    static func == (lhs: ImageViewerItem, rhs: ImageViewerItem) -> Bool {
        lhs.id == rhs.id
    }
}

struct ChatImageViewerPresenter: ViewModifier {
    @Binding var item: ImageViewerItem?
    let onForwardText: (String) -> Void

    func body(content: Content) -> some View {
        #if os(iOS)
        content
            .fullScreenCover(item: $item) { viewerItem in
                IrisImageViewer(item: viewerItem, onForwardText: onForwardText) {
                    item = nil
                }
            }
        #else
        content
            .overlay {
                if let item {
                    IrisImageViewer(item: item, onForwardText: onForwardText) {
                        self.item = nil
                    }
                }
            }
        #endif
    }
}

struct IrisImageViewer: View {
    let item: ImageViewerItem
    let onForwardText: (String) -> Void
    let onClose: () -> Void

    @State private var currentIndex: Int
    @State private var loadedData: [String: Data]
    @State private var loadedImages: [String: PlatformImage]
    @State private var sharedFileURL: URL?
    @State private var dragTranslation: CGFloat = 0

    init(item: ImageViewerItem, onForwardText: @escaping (String) -> Void, onClose: @escaping () -> Void) {
        self.item = item
        self.onForwardText = onForwardText
        self.onClose = onClose
        _currentIndex = State(initialValue: item.initialIndex)
        var initial: [String: Data] = [:]
        var initialImages: [String: PlatformImage] = [:]
        if item.attachments.indices.contains(item.initialIndex) {
            let attachment = item.attachments[item.initialIndex]
            initial[attachment.htreeUrl] = item.initialData
            if !isAnimatedImage(data: item.initialData, filename: attachment.filename),
               let image = PlatformImage(data: item.initialData) {
                initialImages[attachment.htreeUrl] = image
            }
        }
        _loadedData = State(initialValue: initial)
        _loadedImages = State(initialValue: initialImages)
    }

    private var currentAttachment: MessageAttachmentSnapshot? {
        item.attachments.indices.contains(currentIndex) ? item.attachments[currentIndex] : nil
    }

    private var currentData: Data? {
        currentAttachment.flatMap { loadedData[$0.htreeUrl] }
    }

    private var loadTaskID: String {
        "\(item.id):\(currentIndex)"
    }

    var body: some View {
        GeometryReader { geometry in
            ZStack {
                Color.black
                    .opacity(backdropOpacity)
                    .ignoresSafeArea()
                    .onTapGesture(perform: onClose)

                carouselContent
                    .padding(.top, geometry.safeAreaInsets.top + 64)
                    .padding(.bottom, geometry.safeAreaInsets.bottom + 92)
                    .offset(y: dragTranslation)
                    #if os(iOS)
                    .simultaneousGesture(dismissDragGesture)
                    #endif

                VStack(spacing: 0) {
                    topChrome(topInset: geometry.safeAreaInsets.top)
                    Spacer(minLength: 0)
                    bottomChrome(bottomInset: geometry.safeAreaInsets.bottom)
                }
                .ignoresSafeArea()
                .opacity(chromeOpacity)
            }
        }
        .background(Color.black.opacity(backdropOpacity).ignoresSafeArea())
        .environment(\.colorScheme, .dark)
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .zIndex(10)
        .task(id: loadTaskID) {
            let index = currentIndex
            await ensureLoaded(index: index)
            updateSharedFile()
            await preloadAdjacent(index: index)
        }
    }

    private var backdropOpacity: Double {
        let fade = min(abs(dragTranslation) / 600, 0.55)
        return 1 - fade
    }

    private var chromeOpacity: Double {
        let fade = min(abs(dragTranslation) / 220, 1)
        return 1 - fade
    }

    #if os(iOS)
    private var dismissDragGesture: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                let translation = value.translation
                if abs(translation.height) > abs(translation.width) * 1.3 {
                    dragTranslation = translation.height
                }
            }
            .onEnded { value in
                let translation = value.translation.height
                let predicted = value.predictedEndTranslation.height
                if abs(translation) > 140 || abs(predicted) > 360 {
                    onClose()
                } else {
                    withAnimation(.interactiveSpring(response: 0.32, dampingFraction: 0.85)) {
                        dragTranslation = 0
                    }
                }
            }
    }
    #endif

    @ViewBuilder
    private var carouselContent: some View {
        #if os(iOS)
        TabView(selection: $currentIndex) {
            ForEach(Array(item.attachments.enumerated()), id: \.offset) { idx, attachment in
                IrisImageViewerPage(
                    data: loadedData[attachment.htreeUrl],
                    image: loadedImages[attachment.htreeUrl],
                    filename: attachment.filename
                )
                .tag(idx)
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        #else
        ZStack {
            if let attachment = currentAttachment {
                IrisImageViewerPage(
                    data: loadedData[attachment.htreeUrl],
                    image: loadedImages[attachment.htreeUrl],
                    filename: attachment.filename
                )
            }
            if item.attachments.count > 1 {
                HStack {
                    chevronButton(systemName: "chevron.left", disabled: currentIndex == 0) {
                        if currentIndex > 0 { currentIndex -= 1 }
                    }
                    Spacer()
                    chevronButton(systemName: "chevron.right", disabled: currentIndex >= item.attachments.count - 1) {
                        if currentIndex < item.attachments.count - 1 { currentIndex += 1 }
                    }
                }
                .padding(.horizontal, 18)
            }
        }
        .irisOnLeftArrowKey {
            if currentIndex > 0 { currentIndex -= 1 }
        }
        .irisOnRightArrowKey {
            if currentIndex < item.attachments.count - 1 { currentIndex += 1 }
        }
        #endif
    }

    private func chevronButton(systemName: String, disabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            IrisGlassCircleButtonLabel(
                systemName: systemName,
                iconSize: 18,
                hitSize: 44,
                tone: .dark,
                glyphColor: Color.white.opacity(disabled ? 0.4 : 1)
            )
            .opacity(disabled ? 0.55 : 1)
        }
        .buttonStyle(.irisPlain)
        .disabled(disabled)
    }

    @MainActor
    private func ensureLoaded(index: Int) async {
        guard item.attachments.indices.contains(index) else { return }
        let attachment = item.attachments[index]
        if loadedData[attachment.htreeUrl] != nil { return }
        guard let data = await item.downloadAttachment(attachment) else { return }
        loadedData[attachment.htreeUrl] = data
        let isAnimated = isAnimatedImage(data: data, filename: attachment.filename)
        if !isAnimated, loadedImages[attachment.htreeUrl] == nil {
            let bytes = data
            let image = await Task.detached(priority: .userInitiated) {
                PlatformImage(data: bytes)
            }.value
            if let image {
                loadedImages[attachment.htreeUrl] = image
            }
        }
    }

    @MainActor
    private func preloadAdjacent(index: Int) async {
        for neighbor in [index - 1, index + 1] where item.attachments.indices.contains(neighbor) {
            await ensureLoaded(index: neighbor)
        }
    }

    @MainActor
    private func updateSharedFile() {
        guard let attachment = currentAttachment, let data = loadedData[attachment.htreeUrl] else {
            sharedFileURL = nil
            return
        }
        sharedFileURL = writeTempImage(data: data, filename: attachment.filename)
    }

    private func topChrome(topInset: CGFloat) -> some View {
        ZStack {
            HStack {
                backButton
                Spacer(minLength: 0)
            }
            senderHeader
        }
        .padding(.top, topInset + 4)
        .padding(.horizontal, 12)
    }

    private var backButton: some View {
        Button(action: onClose) {
            IrisGlassCircleButtonLabel(
                systemName: "chevron.left",
                iconSize: 16,
                hitSize: 40,
                tone: .dark,
                glyphColor: .white
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Close image")
        .accessibilityIdentifier("imageViewerCloseButton")
    }

    private var senderHeader: some View {
        VStack(spacing: 2) {
            Text(item.senderName)
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
                .foregroundStyle(Color.white)
                .lineLimit(1)
            Text(imageViewerDate(item.createdAtSecs))
                .font(.system(.caption, design: .rounded, weight: .medium))
                .foregroundStyle(Color.white.opacity(0.72))
                .lineLimit(1)
        }
        .padding(.horizontal, 12)
    }

    private func bottomChrome(bottomInset: CGFloat) -> some View {
        VStack(spacing: 12) {
            pageIndicator
            HStack(alignment: .center, spacing: 0) {
                shareButton
                    .frame(width: 40, height: 40)
                Spacer(minLength: 12)
                forwardButton
                    .frame(width: 40, height: 40)
            }
            .frame(height: 40)
        }
        .padding(.horizontal, 20)
        .padding(.top, 10)
        .padding(.bottom, bottomInset + 14)
        .background(alignment: .bottom) {
            LinearGradient(
                colors: [
                    Color.black.opacity(0),
                    Color.black.opacity(0.42),
                    Color.black.opacity(0.68)
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .allowsHitTesting(false)
        }
    }

    @ViewBuilder
    private var pageIndicator: some View {
        if item.attachments.count > 1 {
            HStack(spacing: 6) {
                ForEach(0..<item.attachments.count, id: \.self) { idx in
                    Circle()
                        .fill(Color.white.opacity(idx == currentIndex ? 0.95 : 0.38))
                        .frame(width: 6, height: 6)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule(style: .continuous).fill(Color.black.opacity(0.42)))
            .accessibilityHidden(true)
        }
    }

    @ViewBuilder
    private var forwardButton: some View {
        if let attachment = currentAttachment {
            IrisImageViewerForwardButton(
                text: item.forwardableTextFor(attachment),
                onForwardText: onForwardText
            )
        }
    }

    private var shareButton: some View {
        IrisImageViewerShareButton(sharedFileURL: sharedFileURL)
    }
}

func writeTempImage(data: Data, filename: String) -> URL? {
    let safeName = safeImageShareFilename(data: data, filename: filename)
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString)
        .appendingPathComponent(safeName)
    do {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
        return url
    } catch {
        return nil
    }
}

struct IrisImageViewerShareButton: View {
    let sharedFileURL: URL?

    var body: some View {
        Group {
            if let sharedFileURL {
                ShareLink(item: sharedFileURL) {
                    IrisImageViewerIconButtonLabel(systemName: "square.and.arrow.up")
                }
            } else {
                Button(action: {}) {
                    IrisImageViewerIconButtonLabel(systemName: "square.and.arrow.up", isEnabled: false)
                }
                .disabled(true)
            }
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel("Share image")
        .accessibilityIdentifier("imageViewerShareButton")
    }
}

struct IrisImageViewerForwardButton: View {
    let text: String
    let onForwardText: (String) -> Void

    var body: some View {
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            Button {
                onForwardText(text)
            } label: {
                IrisImageViewerIconButtonLabel(systemName: "arrowshape.turn.up.right")
            }
            .buttonStyle(.irisPlain)
            .accessibilityLabel("Forward")
            .accessibilityIdentifier("imageViewerForwardButton")
        }
    }
}

struct IrisImageViewerPage: View {
    let data: Data?
    let image: PlatformImage?
    let filename: String

    var body: some View {
        Group {
            if let data, isAnimatedImage(data: data, filename: filename) {
                IrisAnimatedImageDataView(data: data)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .allowsHitTesting(false)
            } else if let image {
                Image(platformImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if data == nil {
                ProgressView()
                    .tint(.white)
            } else {
                ProgressView()
                    .tint(.white)
            }
        }
    }
}

struct IrisImageViewerIconButtonLabel: View {
    let systemName: String
    var isEnabled = true

    var body: some View {
        IrisGlassCircleButtonLabel(
            systemName: systemName,
            iconSize: 18,
            hitSize: 40,
            tone: .dark,
            glyphColor: Color.white.opacity(isEnabled ? 1 : 0.38)
        )
    }
}

func imageViewerDate(_ secs: UInt64) -> String {
    imageViewerDateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(secs)))
}

let imageViewerDateFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.doesRelativeDateFormatting = true
    formatter.dateStyle = .medium
    formatter.timeStyle = .short
    return formatter
}()

func uploadFraction(_ progress: UploadProgress?) -> Double? {
    guard let progress, progress.totalBytes > 0 else { return nil }
    let fraction = Double(progress.bytesUploaded) / Double(progress.totalBytes)
    return min(max(fraction, 0), 1)
}

func safeImageShareFilename(data: Data, filename: String) -> String {
    let trimmed = filename.trimmingCharacters(in: .whitespacesAndNewlines)
    var safeName = trimmed.isEmpty ? "image" : (trimmed as NSString).lastPathComponent
    if safeName.isEmpty || safeName == "." || safeName == "/" {
        safeName = "image"
    }

    let invalidScalars = CharacterSet(charactersIn: "/\\:")
    safeName = safeName.unicodeScalars
        .map { invalidScalars.contains($0) ? "-" : String($0) }
        .joined()
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if safeName.isEmpty {
        safeName = "image"
    }

    let currentExtension = (safeName as NSString).pathExtension
    if currentExtension.isEmpty {
        safeName += ".\(imageShareFileExtension(data: data, filename: filename))"
    }
    return safeName
}

func imageShareFileExtension(data: Data, filename: String) -> String {
    let originalExtension = (filename as NSString).pathExtension.lowercased()
    if chatImageExtensions.contains(originalExtension) {
        return originalExtension
    }
    let bytes = [UInt8](data.prefix(12))
    if bytes.starts(with: [0x89, 0x50, 0x4E, 0x47]) {
        return "png"
    }
    if bytes.starts(with: [0xFF, 0xD8, 0xFF]) {
        return "jpg"
    }
    if bytes.starts(with: Array("GIF87a".utf8)) || bytes.starts(with: Array("GIF89a".utf8)) {
        return "gif"
    }
    if bytes.count >= 12,
       bytes[0...3] == Array("RIFF".utf8)[0...3],
       bytes[8...11] == Array("WEBP".utf8)[0...3] {
        return "webp"
    }
    return "jpg"
}
