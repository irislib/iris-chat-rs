#if os(iOS)
import PhotosUI
import SwiftUI
import UIKit

enum IrisAttachmentSource {
    case camera
    case photos
    case files
}

struct IrisAttachmentPicker: View {
    @Environment(\.irisPalette) private var palette
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    let onSource: (IrisAttachmentSource) -> Void
    let onPhotos: ([PhotosPickerItem]) -> Void

    var body: some View {
        ScrollView(.vertical) {
            VStack(spacing: 18) {
                HStack {
                    Text(title)
                        .font(.system(.headline, design: .rounded))
                        .foregroundStyle(palette.textPrimary)
                    Spacer()
                    IrisModalCloseButton(accessibilityIdentifier: "chatAttachmentCloseButton") {
                        dismiss()
                    }
                }
                .padding(.horizontal, 22)

                if #available(iOS 17.0, *) {
                    IrisRecentPhotosPicker(onPhotos: onPhotos)
                        .frame(height: 148)
                        .accessibilityIdentifier("chatAttachmentRecentPhotos")
                }

                HStack(alignment: .top, spacing: 12) {
                    sourceButton("Camera", icon: "camera.fill", source: .camera,
                                 identifier: "chatAttachmentCameraButton")
                        .disabled(!UIImagePickerController.isSourceTypeAvailable(.camera))
                        .opacity(UIImagePickerController.isSourceTypeAvailable(.camera) ? 1 : 0.4)
                    sourceButton("Photos", icon: "photo.on.rectangle.angled", source: .photos,
                                 identifier: "chatAttachmentPhotosButton")
                    sourceButton("Files", icon: "doc.fill", source: .files,
                                 identifier: "chatAttachmentFilesButton")
                }
                .padding(.horizontal, 22)
            }
            .padding(.top, 16)
            .padding(.bottom, 18)
        }
        .scrollIndicators(.hidden)
        .background(palette.background)
        .accessibilityIdentifier("chatAttachmentPicker")
        .presentationDetents([dynamicTypeSize.isAccessibilitySize ? .large : .height(sheetHeight)])
        .presentationDragIndicator(.visible)
    }

    private var title: String {
        if #available(iOS 17.0, *) { return "Recent photos" }
        return "Add attachment"
    }

    private var sheetHeight: CGFloat {
        if #available(iOS 17.0, *) { return 330 }
        return 180
    }

    private func sourceButton(
        _ title: String,
        icon: String,
        source: IrisAttachmentSource,
        identifier: String
    ) -> some View {
        Button { onSource(source) } label: {
            VStack(spacing: 10) {
                Image(systemName: icon)
                    .font(.system(size: 24, weight: .medium))
                    .frame(width: 76, height: 54)
                    .background(palette.panelAlt.opacity(0.6), in: RoundedRectangle(cornerRadius: 20))
                Text(title)
                    .font(.system(.subheadline, design: .rounded, weight: .medium))
            }
            .foregroundStyle(palette.textPrimary)
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityLabel(title)
        .accessibilityIdentifier(identifier)
    }
}

@available(iOS 17.0, *)
private struct IrisRecentPhotosPicker: UIViewControllerRepresentable {
    let onPhotos: ([PhotosPickerItem]) -> Void

    func makeUIViewController(context: Context) -> UIHostingController<IrisRecentPhotosContent> {
        // Isolate the compact picker's presentation preferences; in the same
        // SwiftUI host they can override the containing sheet's presentation.
        let controller = UIHostingController(rootView: IrisRecentPhotosContent(onPhotos: onPhotos))
        controller.view.backgroundColor = .clear
        return controller
    }

    func updateUIViewController(_ controller: UIHostingController<IrisRecentPhotosContent>, context: Context) {
        controller.rootView = IrisRecentPhotosContent(onPhotos: onPhotos)
    }
}

@available(iOS 17.0, *)
private struct IrisRecentPhotosContent: View {
    @State private var selection: [PhotosPickerItem] = []
    let onPhotos: ([PhotosPickerItem]) -> Void

    var body: some View {
        // The system supplies thumbnails without giving Iris broad access to
        // the library. Only a photo the user selects is loaded into the app.
        PhotosPicker(
            selection: $selection,
            maxSelectionCount: 1,
            selectionBehavior: .continuous,
            matching: .any(of: [.images, .videos])
        ) {
            EmptyView()
        }
        .photosPickerStyle(.compact)
        .photosPickerAccessoryVisibility(.hidden)
        .irisOnChange(of: selection) { items in
            guard !items.isEmpty else { return }
            onPhotos(items)
        }
    }
}
#endif
