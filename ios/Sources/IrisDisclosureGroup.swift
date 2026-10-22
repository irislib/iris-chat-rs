import SwiftUI

#if os(macOS)
private struct IrisDisclosureGroupStyle: DisclosureGroupStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation { configuration.isExpanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                        .rotationEffect(.degrees(configuration.isExpanded ? 90 : 0))
                        .frame(width: 10)
                    configuration.label
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.irisPlain)
            .accessibilityValue(configuration.isExpanded ? "Expanded" : "Collapsed")
            if configuration.isExpanded {
                configuration.content.padding(.leading, 16)
            }
        }
    }
}
#endif

extension View {
    @ViewBuilder
    func irisDisclosurePointer() -> some View {
#if os(macOS)
        disclosureGroupStyle(IrisDisclosureGroupStyle())
#else
        self
#endif
    }
}
