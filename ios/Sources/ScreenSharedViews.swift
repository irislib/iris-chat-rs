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

struct BackgroundFill: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        // Solid palette.background — a previous gradient mixed in 28%
        // panelAlt at the bottom, which lifted the lower half of every
        // screen with no explicit .background of its own (e.g., the
        // chat screen) into a noticeably greyer tone than the near-
        // black palette value the rest of the app is tuned for.
        palette.background
            .ignoresSafeArea()
    }
}

struct ToastView: View {
    @Environment(\.irisPalette) private var palette
    let text: String

    var body: some View {
        Text(text)
            .font(.system(.subheadline, design: .rounded, weight: .semibold))
            .foregroundStyle(palette.textPrimary)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                    .fill(palette.panel)
                    .overlay(
                        RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                            .stroke(palette.border, lineWidth: 1)
                    )
            )
    }
}

// Tiny wrapper so RootView doesn't have to subscribe to ToastCenter via the
// fat AppManager — toasts pop on their own publisher and don't drag any
// other view into a re-render.
struct ToastOverlay: View {
    @ObservedObject var center: ToastCenter

    var body: some View {
        if let toast = center.message {
            ToastView(text: toast)
                .padding(.top, 14)
        }
    }
}

#if canImport(AppKit)
final class SecretKeyDraft: ObservableObject {
    @Published var text = ""
}

final class BindingSecureTextField: NSSecureTextField {
    var onTextChange: ((String) -> Void)?

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if event.type == .keyDown,
           flags.contains(.command),
           event.charactersIgnoringModifiers?.lowercased() == "v" {
            pasteFromGeneralPasteboard()
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    private func pasteFromGeneralPasteboard() {
        guard let pasted = NSPasteboard.general.string(forType: .string),
              !pasted.isEmpty else {
            return
        }
        if let editor = currentEditor() {
            editor.insertText(pasted)
            stringValue = editor.string
        } else {
            stringValue = pasted
        }
        onTextChange?(stringValue)
    }

    override func textDidChange(_ notification: Notification) {
        super.textDidChange(notification)
        onTextChange?(stringValue)
    }

    override func textDidEndEditing(_ notification: Notification) {
        super.textDidEndEditing(notification)
        onTextChange?(stringValue)
    }
}

struct MacSecretKeyField: NSViewRepresentable {
    @Binding var text: String

    func makeNSView(context: Context) -> NSSecureTextField {
        let field = BindingSecureTextField()
        field.delegate = context.coordinator
        field.target = context.coordinator
        field.action = #selector(Coordinator.textFieldAction(_:))
        field.isContinuous = true
        field.onTextChange = { value in
            context.coordinator.update(value)
        }
        field.identifier = NSUserInterfaceItemIdentifier("importKeyField")
        field.placeholderString = "Secret key"
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.textColor = .labelColor
        return field
    }

    func updateNSView(_ nsView: NSSecureTextField, context: Context) {
        if let field = nsView as? BindingSecureTextField {
            field.onTextChange = { value in
                context.coordinator.update(value)
            }
        }
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
        nsView.placeholderString = "Secret key"
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        private let text: Binding<String>

        init(text: Binding<String>) {
            self.text = text
        }

        func update(_ value: String) {
            text.wrappedValue = value
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else {
                return
            }
            update(field.stringValue)
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else {
                return
            }
            update(field.stringValue)
        }

        @objc func textFieldAction(_ sender: NSTextField) {
            update(sender.stringValue)
        }
    }
}
#endif

#if !canImport(AppKit)
final class SecretKeyDraft: ObservableObject {
    @Published var text = ""
}
#endif

struct SecretKeyField: View {
    @Binding var text: String

    var body: some View {
        #if canImport(AppKit)
        MacSecretKeyField(text: $text)
            .frame(height: 22)
        #else
        SecureField("Secret key", text: $text)
            .irisIdentifierInputModifiers()
            .textContentType(.password)
            .textFieldStyle(.plain)
            .accessibilityIdentifier("importKeyField")
        #endif
    }
}

struct LoadingOverlay: View {
    @Environment(\.irisPalette) private var palette

    var body: some View {
        ZStack {
            palette.background.opacity(0.4).ignoresSafeArea()
            Image("IrisLogo")
                .resizable()
                .scaledToFit()
                .frame(width: 112, height: 112)
                .accessibilityLabel("Iris")
        }
    }
}

struct CardHeader: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String?

    init(title: String, subtitle: String? = nil) {
        self.title = title
        self.subtitle = subtitle
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.system(.title3, design: .rounded, weight: .bold))
                .foregroundStyle(palette.textPrimary)
            if let subtitle {
                Text(subtitle)
                    .font(.system(.body, design: .rounded))
                    .foregroundStyle(palette.muted)
            }
        }
    }
}

struct MonoValue: View {
    @Environment(\.irisPalette) private var palette
    let label: String
    let value: String
    let identifier: String?

    init(label: String, value: String, identifier: String? = nil) {
        self.label = label
        self.value = value
        self.identifier = identifier
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.system(.caption, design: .rounded, weight: .semibold))
                .foregroundStyle(palette.muted)
            if let identifier {
                Text(value)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(palette.textPrimary)
                    .textSelection(.enabled)
                    .accessibilityIdentifier(identifier)
            } else {
                Text(value)
                    .font(.system(.footnote, design: .monospaced))
                    .foregroundStyle(palette.textPrimary)
                    .textSelection(.enabled)
            }
        }
    }
}

struct SelectedMemberChip: View {
    @Environment(\.irisPalette) private var palette
    let title: String
    let subtitle: String?
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(.caption, design: .rounded, weight: .semibold))
                    .lineLimit(1)
                if let subtitle {
                    Text(subtitle)
                        .font(.system(.caption2, design: .monospaced, weight: .medium))
                        .foregroundStyle(palette.muted)
                        .lineLimit(1)
                }
            }
            Button(action: onRemove) {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
            }
            .buttonStyle(.irisPlain)
            .accessibilityIdentifier("memberChipRemove")
        }
        .foregroundStyle(palette.textPrimary)
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                .fill(palette.panel)
                .overlay(
                    RoundedRectangle(cornerRadius: IrisLayout.pillCornerRadius, style: .continuous)
                        .stroke(palette.border, lineWidth: 1)
                )
        )
    }
}

struct FlowWrap<Content: View>: View {
    let spacing: CGFloat
    let lineSpacing: CGFloat
    let content: () -> Content

    init(
        spacing: CGFloat = 8,
        lineSpacing: CGFloat = 8,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.spacing = spacing
        self.lineSpacing = lineSpacing
        self.content = content
    }

    var body: some View {
        ViewThatFits {
            HStack(alignment: .top, spacing: spacing, content: content)
            VStack(alignment: .leading, spacing: lineSpacing, content: content)
        }
    }
}
