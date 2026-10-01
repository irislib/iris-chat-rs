#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Fit five visible lines with one extra line to detect overflow. The separate
/// container reuses TextKit's cache; TextKit may still process additional text.
final class IrisComposerTextMeasurement {
    let layoutManager = NSLayoutManager()
    private let container = NSTextContainer(size: .zero)
    private weak var storage: NSTextStorage?
    #if DEBUG
    var onLayoutTiming: ((Double) -> Void)?
    #endif

    init() {
        container.lineFragmentPadding = 0
        container.maximumNumberOfLines = 6
        layoutManager.addTextContainer(container)
    }

    func height(for textStorage: NSTextStorage, width: CGFloat, lineHeight: CGFloat) -> CGFloat {
        guard width > 0, width.isFinite else { return lineHeight }
        if storage !== textStorage {
            storage?.removeLayoutManager(layoutManager)
            storage = textStorage
            textStorage.addLayoutManager(layoutManager)
        }
        let maximum = lineHeight * 5
        // Let all six line fragments fit even when emoji make a line taller
        // than the base font. Only the returned composer height is clamped.
        let size = CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        if container.size != size { container.size = size }
        #if DEBUG
        let timingStart = onLayoutTiming == nil ? nil : ProcessInfo.processInfo.systemUptime
        #endif
        layoutManager.ensureLayout(for: container)
        #if DEBUG
        if let timingStart { onLayoutTiming?((ProcessInfo.processInfo.systemUptime - timingStart) * 1_000) }
        #endif
        let used = layoutManager.usedRect(for: container)
        let extra = layoutManager.extraLineFragmentRect
        return min(max(ceil(max(used.maxY, extra.maxY)), lineHeight), maximum)
    }

    deinit { storage?.removeLayoutManager(layoutManager) }
}
