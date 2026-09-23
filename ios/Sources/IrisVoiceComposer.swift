#if os(iOS)
import SwiftUI
import UIKit

struct IrisVoiceRecordingStatus: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var recorder: IrisVoiceMessageRecorder

    var body: some View {
        HStack(spacing: 10) {
            Circle().fill(.red).frame(width: 8, height: 8)
                .scaleEffect(1 + recorder.level * 0.6)
                .accessibilityHidden(true)
            Text(irisVoiceElapsed(recorder.duration))
                .font(.system(.body, design: .rounded).monospacedDigit())
                .accessibilityIdentifier("chatVoiceDuration")
            Spacer(minLength: 0)
            if recorder.phase == .locked {
                Button { recorder.cancel() } label: {
                    Image(systemName: "trash.fill").frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel("Cancel recording")
                .accessibilityIdentifier("chatVoiceCancelButton")
                Button { Task { _ = await recorder.finish() } } label: {
                    Image(systemName: "stop.fill").frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel("Stop recording")
                .accessibilityIdentifier("chatVoiceStopButton")
            } else {
                Text(recorder.phase == .requestingPermission ? "Starting…" : "‹ Slide to cancel")
                    .font(.system(.callout, design: .rounded))
                    .foregroundStyle(palette.muted)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
        .buttonStyle(.irisPlain)
        .foregroundStyle(palette.textPrimary)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: 44)
        .irisGlassSurface(in: Capsule())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatVoiceRecording")
    }
}

struct IrisVoiceRecordButton: View {
    @Environment(\.irisPalette) private var palette
    @ObservedObject var recorder: IrisVoiceMessageRecorder
    let enabled: Bool
    let onBegin: () -> Void
    let onSend: () -> Void
    @State private var showsHint = false

    private var locked: Bool { recorder.phase == .locked }
    private var capturing: Bool { recorder.phase == .recording }

    var body: some View {
        ZStack {
            Circle().fill(locked ? palette.accent : palette.panel)
            if recorder.phase == .requestingPermission || recorder.phase == .finishing {
                ProgressView().controlSize(.small)
            } else {
                Image(systemName: locked ? "arrow.up" : "mic.fill")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(locked ? palette.onAccent : (capturing ? .red : palette.textPrimary))
            }
        }
        .frame(width: 44, height: 44)
        .opacity(enabled ? 1 : 0.45)
        .accessibilityHidden(true)
        .overlay {
            IrisVoiceGestureControl(
                enabled: enabled,
                label: locked ? "Send voice message" : "Record voice message",
                identifier: locked ? "chatVoiceSendButton" : "chatVoiceRecordButton",
                onBegin: {
                    guard recorder.phase == .idle else { return }
                    onBegin()
                    recorder.begin()
                },
                onMove: { delta in
                    guard recorder.phase == .recording || recorder.phase == .requestingPermission else { return }
                    if delta.width < -100 {
                        recorder.cancel()
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    } else if delta.height < -100 {
                        recorder.lock()
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    }
                },
                onEnd: { cancelled in
                    if cancelled {
                        if recorder.phase != .locked { recorder.cancel() }
                    } else if recorder.phase == .recording {
                        onSend()
                    } else if recorder.phase == .requestingPermission {
                        recorder.cancel()
                    }
                },
                onTap: {
                    if locked { onSend() }
                    else if recorder.phase == .idle {
                        showsHint = true
                        Task { try? await Task.sleep(nanoseconds: 2_000_000_000); showsHint = false }
                    }
                },
                onAccessibleActivate: {
                    if locked { onSend() }
                    else if recorder.phase == .idle { onBegin(); recorder.begin(locked: true) }
                }
            )
        }
        .overlay(alignment: .bottomTrailing) {
            if capturing {
                VStack(spacing: 7) {
                    Image(systemName: "lock.fill")
                    Image(systemName: "chevron.up")
                }
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(palette.textPrimary)
                .frame(width: 44, height: 70)
                .irisGlassSurface(in: Capsule())
                .offset(y: -54)
                .accessibilityLabel("Slide up to lock recording")
                .accessibilityIdentifier("chatVoiceLockHint")
                .allowsHitTesting(false)
            } else if showsHint {
                Text("Hold to record")
                    .font(.system(.caption, design: .rounded))
                    .fixedSize()
                    .padding(10)
                    .background(palette.panel, in: Capsule())
                    .offset(y: -52)
                    .allowsHitTesting(false)
            }
        }
    }
}

private struct IrisVoiceGestureControl: UIViewRepresentable {
    let enabled: Bool
    let label: String
    let identifier: String
    let onBegin: () -> Void
    let onMove: (CGSize) -> Void
    let onEnd: (Bool) -> Void
    let onTap: () -> Void
    let onAccessibleActivate: () -> Void

    func makeUIView(context: Context) -> TouchView {
        let view = TouchView()
        let hold = UILongPressGestureRecognizer(target: view, action: #selector(TouchView.hold(_:)))
        hold.minimumPressDuration = 0.2
        hold.allowableMovement = .greatestFiniteMagnitude
        let tap = UITapGestureRecognizer(target: view, action: #selector(TouchView.tap))
        tap.require(toFail: hold)
        view.addGestureRecognizer(hold)
        view.addGestureRecognizer(tap)
        view.isAccessibilityElement = true
        view.accessibilityTraits = .button
        view.accessibilityHint = "Hold and release to send. Slide left to cancel or up to lock."
        return view
    }

    func updateUIView(_ view: TouchView, context: Context) {
        view.actions = self
        view.isUserInteractionEnabled = enabled
        view.accessibilityTraits = enabled ? .button : [.button, .notEnabled]
        view.accessibilityLabel = label
        view.accessibilityIdentifier = identifier
    }

    final class TouchView: UIView {
        var actions: IrisVoiceGestureControl?
        private var origin: CGPoint = .zero
        @objc func hold(_ gesture: UILongPressGestureRecognizer) {
            let point = gesture.location(in: window)
            switch gesture.state {
            case .began: origin = point; actions?.onBegin()
            case .changed: actions?.onMove(CGSize(width: point.x - origin.x, height: point.y - origin.y))
            case .ended: actions?.onEnd(false)
            case .cancelled, .failed: actions?.onEnd(true)
            default: break
            }
        }
        @objc func tap() { actions?.onTap() }
        override func accessibilityActivate() -> Bool { actions?.onAccessibleActivate(); return true }
    }
}

private func irisVoiceElapsed(_ seconds: TimeInterval) -> String {
    let value = max(0, Int(seconds))
    return String(format: "%d:%02d", value / 60, value % 60)
}
#endif
