import Foundation
import SwiftUI
#if canImport(AppKit)
import AppKit
#endif
#if canImport(UIKit)
import UIKit
#endif

/// Shows a pointing-hand cursor while hovered. No-op outside macOS.
struct IrisHoverPointerModifier: ViewModifier {
    let enabled: Bool
    @Environment(\.isEnabled) private var isEnabled

    func body(content: Content) -> some View {
#if canImport(AppKit)
        if #available(macOS 15.0, *) {
            content.pointerStyle(enabled && isEnabled ? .link : nil)
        } else {
            content.modifier(IrisLegacyHoverPointerModifier(enabled: enabled && isEnabled))
        }
#else
        content
#endif
    }
}

#if canImport(AppKit)
/// macOS 14 has no SwiftUI pointerStyle. Share one cursor-stack entry across
/// nested controls, and release it when a hovered control disappears or disables.
struct IrisLegacyHoverPointerModifier: ViewModifier {
    let enabled: Bool
    @State private var id = UUID()
    @State private var hovered = false

    func body(content: Content) -> some View {
        content
            .onContinuousHover { phase in
                if case .active = phase { hovered = true } else { hovered = false }
                IrisLegacyPointerOwners.update(id, active: enabled && hovered)
            }
            .onChange(of: enabled) { _, enabled in
                IrisLegacyPointerOwners.update(id, active: enabled && hovered)
            }
            .onDisappear { IrisLegacyPointerOwners.update(id, active: false) }
    }
}

@MainActor
private enum IrisLegacyPointerOwners {
    static var owners = Set<UUID>()

    static func update(_ id: UUID, active: Bool) {
        let wasEmpty = owners.isEmpty
        if active { owners.insert(id) } else { owners.remove(id) }
        if wasEmpty && !owners.isEmpty { NSCursor.pointingHand.push() }
        else if !wasEmpty && owners.isEmpty { NSCursor.pop() }
        if !owners.isEmpty { NSCursor.pointingHand.set() }
    }
}
#endif

extension View {
    /// Shows a pointing-hand cursor while hovered. Inert on iOS.
    func irisHoverPointer(enabled: Bool = true) -> some View {
        modifier(IrisHoverPointerModifier(enabled: enabled))
    }
}

