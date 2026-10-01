#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Sizing only needs the visible five lines. Share the text storage, not the
/// editor's unbounded scrolling container, and reuse TextKit's layout cache.
final class IrisComposerTextMeasurement {
    let layoutManager = NSLayoutManager()
    private let container = NSTextContainer(size: .zero)
    private weak var storage: NSTextStorage?

    init() {
        container.lineFragmentPadding = 0
        container.maximumNumberOfLines = 5
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
        let size = CGSize(width: width, height: maximum)
        if container.size != size { container.size = size }
        layoutManager.ensureLayout(for: container)
        if layoutManager.firstUnlaidCharacterIndex() < textStorage.length { return maximum }
        let used = layoutManager.usedRect(for: container)
        let extra = layoutManager.extraLineFragmentRect
        return min(max(ceil(max(used.maxY, extra.maxY)), lineHeight), maximum)
    }

    deinit { storage?.removeLayoutManager(layoutManager) }
}
