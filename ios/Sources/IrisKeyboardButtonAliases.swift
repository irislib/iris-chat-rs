import SwiftUI

#if os(macOS)
typealias IrisPlainButtonStyle = IrisKeyboardButtonStyle<IrisPlainButtonVisualStyle>
typealias IrisUnpressedButtonStyle = IrisKeyboardButtonStyle<IrisUnpressedButtonVisualStyle>
typealias IrisPrimaryButtonStyle = IrisKeyboardButtonStyle<IrisPrimaryButtonVisualStyle>
typealias IrisSecondaryButtonStyle = IrisKeyboardButtonStyle<IrisSecondaryButtonVisualStyle>
typealias IrisHeaderButtonStyle = IrisKeyboardButtonStyle<IrisHeaderButtonVisualStyle>
typealias IrisPrimaryCircleButtonStyle = IrisKeyboardButtonStyle<IrisPrimaryCircleButtonVisualStyle>

extension IrisKeyboardButtonStyle where Visual == IrisPlainButtonVisualStyle {
    init() { self.init(visual: IrisPlainButtonVisualStyle()) }
}
extension IrisKeyboardButtonStyle where Visual == IrisUnpressedButtonVisualStyle {
    init() { self.init(visual: IrisUnpressedButtonVisualStyle()) }
}
extension IrisKeyboardButtonStyle where Visual == IrisPrimaryButtonVisualStyle {
    init(compact: Bool = false) { self.init(visual: IrisPrimaryButtonVisualStyle(compact: compact)) }
}
extension IrisKeyboardButtonStyle where Visual == IrisSecondaryButtonVisualStyle {
    init(compact: Bool = false) { self.init(visual: IrisSecondaryButtonVisualStyle(compact: compact)) }
}
extension IrisKeyboardButtonStyle where Visual == IrisHeaderButtonVisualStyle {
    init() { self.init(visual: IrisHeaderButtonVisualStyle()) }
}
extension IrisKeyboardButtonStyle where Visual == IrisPrimaryCircleButtonVisualStyle {
    init() { self.init(visual: IrisPrimaryCircleButtonVisualStyle()) }
}
extension PrimitiveButtonStyle where Self == IrisPlainButtonStyle {
    static var irisPlain: IrisPlainButtonStyle { IrisPlainButtonStyle() }
}
extension PrimitiveButtonStyle where Self == IrisUnpressedButtonStyle {
    static var irisUnpressed: IrisUnpressedButtonStyle { IrisUnpressedButtonStyle() }
}
#else
typealias IrisPlainButtonStyle = IrisPlainButtonVisualStyle
typealias IrisUnpressedButtonStyle = IrisUnpressedButtonVisualStyle
typealias IrisPrimaryButtonStyle = IrisPrimaryButtonVisualStyle
typealias IrisSecondaryButtonStyle = IrisSecondaryButtonVisualStyle
typealias IrisHeaderButtonStyle = IrisHeaderButtonVisualStyle
typealias IrisPrimaryCircleButtonStyle = IrisPrimaryCircleButtonVisualStyle

extension ButtonStyle where Self == IrisPlainButtonStyle {
    static var irisPlain: IrisPlainButtonStyle { IrisPlainButtonStyle() }
}
extension ButtonStyle where Self == IrisUnpressedButtonStyle {
    static var irisUnpressed: IrisUnpressedButtonStyle { IrisUnpressedButtonStyle() }
}
#endif
