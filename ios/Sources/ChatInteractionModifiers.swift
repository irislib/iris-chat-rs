import Foundation
import ImageIO
import SwiftUI
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

extension View {
    @ViewBuilder
    func observeChatTimelineScroll(
        coordinator: ChatTimelineInteractionCoordinator,
        onPan: @escaping (CGFloat, CGFloat) -> Void
    ) -> some View {
#if os(iOS)
        background(
            ChatTimelineScrollObserver(
                timelineCoordinator: coordinator,
                onPan: onPan
            )
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
        )
#else
        self
#endif
    }

    @ViewBuilder
    func applyMessageBubbleSwipe(offset: CGFloat) -> some View {
#if os(iOS)
        modifier(MessageBubbleSwipeActions(offset: offset))
#else
        self
#endif
    }
}

#if os(iOS)
struct TouchDownControl: UIViewRepresentable {
    let accessibilityIdentifier: String
    let accessibilityLabel: String
    let action: () -> Void

    func makeUIView(context: Context) -> TouchDownControlView {
        let view = TouchDownControlView()
        view.backgroundColor = .clear
        view.isAccessibilityElement = true
        view.accessibilityTraits = [.button]
        view.accessibilityIdentifier = accessibilityIdentifier
        view.accessibilityLabel = accessibilityLabel
        view.onTouchDown = action
        return view
    }

    func updateUIView(_ uiView: TouchDownControlView, context: Context) {
        uiView.accessibilityIdentifier = accessibilityIdentifier
        uiView.accessibilityLabel = accessibilityLabel
        uiView.onTouchDown = action
    }
}

final class TouchDownControlView: UIControl {
    var onTouchDown: (() -> Void)?
    private var lastFireAt: CFTimeInterval = 0
    private weak var observedWindow: UIWindow?
    private var windowRecognizer: WindowTouchDownGestureRecognizer?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        updateWindowRecognizer()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateWindowRecognizer()
    }

    deinit {
        if let windowRecognizer, let observedWindow {
            observedWindow.removeGestureRecognizer(windowRecognizer)
        }
    }

    override func beginTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        fireOnce()
        return true
    }

    override func continueTracking(_ touch: UITouch, with event: UIEvent?) -> Bool {
        true
    }

    func fireOnce() {
        let now = CACurrentMediaTime()
        guard now - lastFireAt > 0.15 else { return }
        lastFireAt = now
        onTouchDown?()
    }

    private func updateWindowRecognizer() {
        if observedWindow === window {
            windowRecognizer?.targetView = self
            return
        }
        if let windowRecognizer, let observedWindow {
            observedWindow.removeGestureRecognizer(windowRecognizer)
        }
        observedWindow = window
        guard let window else {
            windowRecognizer = nil
            return
        }
        // UIScrollView can consume the first touch while decelerating.
        // Watching the window lets the jump button interrupt that coast.
        let recognizer = WindowTouchDownGestureRecognizer()
        recognizer.targetView = self
        recognizer.cancelsTouchesInView = false
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
        window.addGestureRecognizer(recognizer)
        windowRecognizer = recognizer
    }
}

final class WindowTouchDownGestureRecognizer: UIGestureRecognizer, UIGestureRecognizerDelegate {
    weak var targetView: TouchDownControlView?

    override init(target: Any?, action: Selector?) {
        super.init(target: target, action: action)
        delegate = self
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let touch = touches.first,
              let window = view,
              let targetView,
              !targetView.isHidden,
              targetView.alpha > 0.01 else {
            state = .failed
            return
        }
        let frame = targetView.convert(targetView.bounds, to: window).insetBy(dx: -8, dy: -8)
        if frame.contains(touch.location(in: window)) {
            targetView.fireOnce()
            state = .recognized
        } else {
            state = .failed
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible {
            state = .failed
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible {
            state = .failed
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        state = .cancelled
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        true
    }
}

struct ChatTimelineScrollObserver: UIViewRepresentable {
    let timelineCoordinator: ChatTimelineInteractionCoordinator
    let onPan: (CGFloat, CGFloat) -> Void

    func makeUIView(context: Context) -> ChatTimelineScrollObserverView {
        let view = ChatTimelineScrollObserverView()
        view.timelineCoordinator = timelineCoordinator
        view.onPan = onPan
        return view
    }

    func updateUIView(_ uiView: ChatTimelineScrollObserverView, context: Context) {
        uiView.timelineCoordinator = timelineCoordinator
        uiView.onPan = onPan
        uiView.bindToEnclosingScrollView()
    }

    static func dismantleUIView(_ uiView: ChatTimelineScrollObserverView, coordinator: ()) {
        uiView.unbind()
    }
}

final class ChatTimelineScrollObserverView: UIView {
    weak var timelineCoordinator: ChatTimelineInteractionCoordinator?
    var onPan: ((CGFloat, CGFloat) -> Void)?

    private weak var observedScrollView: UIScrollView?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        DispatchQueue.main.async { [weak self] in
            self?.bindToEnclosingScrollView()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        bindToEnclosingScrollView()
    }

    func bindToEnclosingScrollView() {
        guard let scrollView = enclosingScrollView() else { return }
        if observedScrollView === scrollView {
            timelineCoordinator?.scrollView = scrollView
            return
        }

        unbind()
        observedScrollView = scrollView
        timelineCoordinator?.scrollView = scrollView
        scrollView.panGestureRecognizer.addTarget(self, action: #selector(handleScrollPan(_:)))
    }

    func unbind() {
        if let scrollView = observedScrollView {
            scrollView.panGestureRecognizer.removeTarget(self, action: #selector(handleScrollPan(_:)))
        }
        if timelineCoordinator?.scrollView === observedScrollView {
            timelineCoordinator?.scrollView = nil
        }
        observedScrollView = nil
    }

    @objc private func handleScrollPan(_ recognizer: UIPanGestureRecognizer) {
        guard let scrollView = observedScrollView else { return }
        let translation = recognizer.translation(in: scrollView)
        switch recognizer.state {
        case .began, .changed:
            let velocity = recognizer.velocity(in: scrollView)
            onPan?(translation.y, velocity.y)
        default:
            break
        }
    }

    private func enclosingScrollView() -> UIScrollView? {
        var view: UIView? = self
        while let current = view {
            if let scrollView = current as? UIScrollView {
                return scrollView
            }
            view = current.superview
        }
        return nil
    }
}

struct MessageBubbleSwipeActions: ViewModifier {
    let offset: CGFloat

    func body(content: Content) -> some View {
        content
            .offset(x: offset)
            .overlay(alignment: .leading) {
                Image(systemName: "arrowshape.turn.up.left.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .opacity(reveal(forwards: true))
                    .scaleEffect(0.7 + 0.3 * reveal(forwards: true))
                    .padding(.leading, 14)
                    .allowsHitTesting(false)
            }
            .overlay(alignment: .trailing) {
                Image(systemName: "info.circle.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .opacity(reveal(forwards: false))
                    .scaleEffect(0.7 + 0.3 * reveal(forwards: false))
                    .padding(.trailing, 14)
                    .allowsHitTesting(false)
            }
    }

    private func reveal(forwards: Bool) -> Double {
        let v = forwards ? offset : -offset
        return Double(min(1, max(0, v / ChatMessageBubbleSwipeMetrics.threshold)))
    }
}
#endif

struct EscDismissesReply: ViewModifier {
    @Binding var replyTarget: ChatMessageSnapshot?

    func body(content: Content) -> some View {
        if #available(iOS 17.0, macOS 14.0, *) {
            content.onKeyPress(.escape) {
                if replyTarget != nil {
                    replyTarget = nil
                    return .handled
                }
                return .ignored
            }
        } else {
            content
        }
    }
}

struct IrisReplyComposerStrip: View {
    @Environment(\.irisPalette) private var palette
    let message: ChatMessageSnapshot
    let onCancel: () -> Void

    private var authorName: String {
        message.isOutgoing ? "You" : message.author
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            HStack(spacing: 8) {
                Capsule()
                    .fill(palette.muted.opacity(0.55))
                    .frame(width: 4, height: 38)
                    .padding(.vertical, 8)

                VStack(alignment: .leading, spacing: 2) {
                    Text(authorName)
                        .font(.system(.caption, design: .rounded, weight: .semibold))
                        .foregroundStyle(palette.textPrimary)
                        .lineLimit(1)
                    Text(replySnippet(for: message))
                        .font(.system(.subheadline, design: .rounded, weight: .regular))
                        .foregroundStyle(palette.muted)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)

                if let attachment = message.attachments.first {
                    IrisReplyComposerAttachmentBadge(attachment: attachment)
                        .padding(.vertical, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            IrisReplyComposerCloseButton(action: onCancel)
                .padding(.top, 4)
        }
        .padding(.horizontal, 8)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(palette.panelAlt)
        )
        .padding(.horizontal, IrisLayout.usesDesktopChrome ? 14 : 8)
        .padding(.top, 6)
        .padding(.bottom, 2)
        .accessibilityIdentifier("chatReplyComposer")
    }
}

struct IrisReplyComposerCloseButton: View {
    @Environment(\.irisPalette) private var palette
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            ZStack {
                Circle()
                    .fill(palette.toolbar)
                    .frame(width: 24, height: 24)
                Image(systemName: "xmark")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(palette.textPrimary)
            }
            .frame(width: 32, height: 32)
            .contentShape(Rectangle())
        }
        .buttonStyle(.irisPlain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Close")
        .accessibilityIdentifier("chatReplyCancelButton")
    }
}

struct IrisReplyComposerAttachmentBadge: View {
    @Environment(\.irisPalette) private var palette
    let attachment: MessageAttachmentSnapshot

    var body: some View {
        let category = chatAttachmentCategory(for: attachment)
        Image(systemName: category.systemIcon)
            .font(.system(size: 18, weight: .semibold))
            .foregroundStyle(palette.muted)
            .frame(width: 46, height: 46)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(palette.toolbar)
            )
            .accessibilityLabel(category.rawValue)
    }
}

struct ReplyPreviewView: View {
    @Environment(\.irisPalette) private var palette
    let reply: ReplyPreview
    let isOutgoing: Bool
    let onTap: () -> Void

    private let collapsedLineLimit = 4

    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 8) {
                Rectangle()
                    // Match the quote text color (the bubble's foreground)
                    // so the rule reads as a margin marker, not an accent
                    // band — it was previously the app accent (purple) on
                    // incoming bubbles, which fought with the message.
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.6))
                    .frame(width: 3)
                    .clipShape(Capsule())
                VStack(alignment: .leading, spacing: 2) {
                    Text(reply.author)
                        .font(.system(.caption, design: .rounded, weight: .bold))
                    Text(reply.body)
                        .font(.system(.caption, design: .rounded, weight: .medium))
                        .lineLimit(collapsedLineLimit)
                        .multilineTextAlignment(.leading)
                        .opacity(0.82)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            // Stretch the rounded background to the bubble's full inner
            // width so the quote pill matches what the body text below
            // gets. Spacer above pushes the rule+text to leading; the
            // frame here just lets the surrounding fill grow.
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill((isOutgoing ? palette.onBubbleMine : palette.onBubbleTheirs).opacity(0.12))
            )
        }
        .buttonStyle(.irisPlain)
        .accessibilityHint("Tap to scroll to the quoted message")
        .accessibilityIdentifier("chatReplyPreview")
    }
}
