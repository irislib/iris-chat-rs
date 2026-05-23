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

struct IrisProfilePictureViewer: View {
    let item: IrisProfilePictureViewerItem
    let preferences: PreferencesSnapshot
    let manager: AppManager
    let onClose: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let diameter = max(120, min(geometry.size.width, geometry.size.height) - 48)
            ZStack(alignment: .topTrailing) {
                Color.black
                    .ignoresSafeArea()
                    .onTapGesture(perform: onClose)

                IrisAvatar(
                    label: item.label,
                    size: diameter,
                    emphasize: false,
                    pictureUrl: item.pictureUrl,
                    preferences: preferences,
                    manager: manager,
                    loadedImageIdentifier: "\(item.accessibilityIdentifier)Image"
                )
                .overlay(
                    Circle()
                        .strokeBorder(Color.white.opacity(0.12), lineWidth: 1)
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)

                IrisModalCloseButton(
                    accessibilityLabel: "Close profile picture",
                    tone: .light,
                    iconSize: 30,
                    hitSize: 66,
                    action: onClose
                )
            }
        }
        .irisOnExitCommand(onClose)
        .irisOnEscapeKey(onClose)
        .accessibilityIdentifier(item.accessibilityIdentifier)
        .zIndex(20)
    }
}

extension View {
    @ViewBuilder
    func irisProfilePictureViewer(
        item: Binding<IrisProfilePictureViewerItem?>,
        preferences: PreferencesSnapshot,
        manager: AppManager
    ) -> some View {
#if os(iOS)
        fullScreenCover(item: item) { viewerItem in
            IrisProfilePictureViewer(
                item: viewerItem,
                preferences: preferences,
                manager: manager
            ) {
                item.wrappedValue = nil
            }
        }
#else
        overlay {
            if let viewerItem = item.wrappedValue {
                IrisProfilePictureViewer(
                    item: viewerItem,
                    preferences: preferences,
                    manager: manager
                ) {
                    item.wrappedValue = nil
                }
            }
        }
#endif
    }
}
