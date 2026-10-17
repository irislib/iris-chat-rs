#if os(macOS)
import AppKit
import SwiftUI

struct IrisDesktopImageZoomGeometry {
    static func fittedSize(image: CGSize, viewport: CGSize) -> CGSize {
        guard image.width > 0, image.height > 0 else { return .zero }
        let fit = min(viewport.width / image.width, viewport.height / image.height)
        return CGSize(width: image.width * fit, height: image.height * fit)
    }

    static func offset(at point: CGPoint, image: CGSize, viewport: CGSize, scale: CGFloat) -> CGSize {
        let fitted = fittedSize(image: image, viewport: viewport)
        let limitX = max(0, (fitted.width * scale - viewport.width) / 2)
        let limitY = max(0, (fitted.height * scale - viewport.height) / 2)
        return CGSize(
            width: min(limitX, max(-limitX, (viewport.width / 2 - point.x) * scale)),
            height: min(limitY, max(-limitY, (viewport.height / 2 - point.y) * scale))
        )
    }
}

/// Click to inspect a detail, then move the pointer to explore the enlarged image.
struct IrisDesktopImageZoom: ViewModifier {
    let imageSize: CGSize?
    @Binding var zoomed: Bool
    @State private var scale: CGFloat = 1
    @State private var pointer: CGPoint?
    @State private var pinchStart: CGFloat?

    func body(content: Content) -> some View {
        if let imageSize, imageSize.width > 0, imageSize.height > 0 {
            GeometryReader { geometry in
                let center = CGPoint(x: geometry.size.width / 2, y: geometry.size.height / 2)
                let offset = IrisDesktopImageZoomGeometry.offset(
                    at: pointer ?? center, image: imageSize, viewport: geometry.size, scale: scale
                )
                Button {
                    withAnimation(.easeInOut(duration: 0.16)) {
                        if zoomed { reset() }
                        else {
                            if NSApp.currentEvent?.type == .keyDown { pointer = center }
                            scale = 3
                            zoomed = true
                        }
                    }
                } label: {
                    content
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .scaleEffect(scale)
                        .offset(offset)
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .clipped()
                        .contentShape(Rectangle())
                }
                .buttonStyle(IrisKeyboardButtonStyle(visual: IrisImageZoomButtonVisualStyle()))
                .background(IrisImageZoomCursor(zoomed: zoomed))
                .accessibilityLabel(zoomed ? "Zoom out" : "Zoom in")
                .accessibilityValue(zoomed ? "Zoomed in" : "Fit to window")
                .accessibilityIdentifier("imageViewerZoom")
                .help(zoomed ? "Click to zoom out" : "Click to zoom in")
                .onContinuousHover { phase in
                    switch phase {
                    case .active(let location):
                        pointer = location
                    case .ended: break
                    }
                }
                .simultaneousGesture(MagnifyGesture()
                    .onChanged { value in
                        if pinchStart == nil { pinchStart = scale }
                        pointer = CGPoint(x: value.startAnchor.x * geometry.size.width,
                                          y: value.startAnchor.y * geometry.size.height)
                        scale = min(6, max(1, (pinchStart ?? 1) * value.magnification))
                        zoomed = scale > 1
                    }
                    .onEnded { _ in pinchStart = nil }
                )
                .onChange(of: zoomed) { _, value in if !value { scale = 1 } }
            }
        } else {
            content
        }
    }

    private func reset() {
        scale = 1
        zoomed = false
        pinchStart = nil
    }
}

private struct IrisImageZoomButtonVisualStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { configuration.label }
}

private struct IrisImageZoomCursor: NSViewRepresentable {
    let zoomed: Bool

    func makeNSView(context: Context) -> CursorView { CursorView() }

    func updateNSView(_ view: CursorView, context: Context) {
        guard view.zoomed != zoomed else { return }
        view.zoomed = zoomed
        view.window?.invalidateCursorRects(for: view)
    }

    final class CursorView: NSView {
        var zoomed = false
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func resetCursorRects() {
            super.resetCursorRects()
            if #available(macOS 15, *) {
                addCursorRect(visibleRect, cursor: zoomed ? .zoomOut : .zoomIn)
            } else {
                addCursorRect(visibleRect, cursor: .pointingHand)
            }
        }
    }
}
#endif
