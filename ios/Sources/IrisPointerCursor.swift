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
    @Environment(\.isEnabled) private var isEnabled

    func body(content: Content) -> some View {
#if canImport(AppKit)
        content.background(IrisPointerCursorRegion(enabled: isEnabled))
#else
        content
#endif
    }
}

#if canImport(AppKit)
/// Cursor rectangles follow native view lifetime, clipping and window changes.
/// A global push/pop stack can leave a stale hand behind when a hovered button
/// disappears, and lets nested hover handlers pop each other's cursor.
private struct IrisPointerCursorRegion: NSViewRepresentable {
    let enabled: Bool

    func makeNSView(context: Context) -> IrisPointerCursorView { IrisPointerCursorView() }

    func updateNSView(_ view: IrisPointerCursorView, context: Context) {
        guard view.pointerEnabled != enabled else { return }
        view.pointerEnabled = enabled
        view.window?.invalidateCursorRects(for: view)
    }
}

final class IrisPointerCursorView: NSView {
    var pointerEnabled = true

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func resetCursorRects() {
        super.resetCursorRects()
        if pointerEnabled { addCursorRect(visibleRect, cursor: .pointingHand) }
    }
}
#endif

extension View {
    /// Shows a pointing-hand cursor while hovered. Inert on iOS.
    func irisHoverPointer() -> some View {
        modifier(IrisHoverPointerModifier())
    }
}

