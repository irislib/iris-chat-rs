#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import IrisChatMac

@MainActor
final class GroupAvatarColorTests: XCTestCase {
    func testPlaceholderMatchesSenderNameInBothThemes() throws {
        for dark in [false, true] {
            let avatar = try render(dark: dark, key: "Example Sender")
            let swatch = ImageRenderer(content:
                irisGroupSenderNameColor(for: "Example Sender", isDarkMode: dark)
                    .frame(width: 48, height: 48))
            swatch.scale = 2
            let reference = try bitmap(swatch)
            assertSamePixel(avatar, reference, x: 48, y: 14)
        }
    }

    func testColorDoesNotTintAnExistingPhoto() throws {
        let image = NSImage(size: NSSize(width: 48, height: 48))
        image.lockFocus()
        NSColor.systemPink.setFill()
        NSRect(x: 0, y: 0, width: 48, height: 48).fill()
        image.unlockFocus()
        IrisAvatarImageCache.store(image, for: "htree:group-avatar-color-fixture")
        for dark in [false, true] {
            let ordinary = try render(dark: dark, key: nil, picture: "htree://group-avatar-color-fixture")
            let colored = try render(dark: dark, key: "Example Sender", picture: "htree://group-avatar-color-fixture")
            for y in 0..<ordinary.pixelsHigh {
                for x in 0..<ordinary.pixelsWide {
                    assertSamePixel(ordinary, colored, x: x, y: y)
                }
            }
        }
    }

    func testInitialContrastAcrossEntireSenderPalette() {
        func luminance(_ hex: UInt32) -> Double {
            let channels = [16, 8, 0].map { shift -> Double in
                let value = Double((hex >> shift) & 0xff) / 255
                return value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
            }
            return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
        }
        for color in irisGroupSenderNameLightColorHexes {
            XCTAssertGreaterThanOrEqual(1.05 / (luminance(color) + 0.05), 4.5)
        }
        for color in irisGroupSenderNameDarkColorHexes {
            XCTAssertGreaterThanOrEqual((luminance(color) + 0.05) / 0.05, 4.5)
        }
    }

    private func render(dark: Bool, key: String?, picture: String? = nil) throws -> NSBitmapImageRep {
        let view = IrisAvatar(label: "Example Sender", size: 48, pictureUrl: picture,
                              showsNearbyBadge: false, groupSenderColorKey: key)
            .environment(\.irisPalette, dark ? .dark : .light)
            .environment(\.colorScheme, dark ? .dark : .light)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        return try bitmap(renderer)
    }

    private func bitmap<V: View>(_ renderer: ImageRenderer<V>) throws -> NSBitmapImageRep {
        let image = try XCTUnwrap(renderer.nsImage)
        return try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
    }

    private func assertSamePixel(_ a: NSBitmapImageRep, _ b: NSBitmapImageRep, x: Int, y: Int,
                                 file: StaticString = #filePath, line: UInt = #line) {
        let lhs = a.colorAt(x: x, y: y)!.usingColorSpace(.sRGB)!
        let rhs = b.colorAt(x: x, y: y)!.usingColorSpace(.sRGB)!
        for (left, right) in [(lhs.redComponent, rhs.redComponent),
                              (lhs.greenComponent, rhs.greenComponent),
                              (lhs.blueComponent, rhs.blueComponent),
                              (lhs.alphaComponent, rhs.alphaComponent)] {
            XCTAssertEqual(left, right, accuracy: 1.0 / 255, file: file, line: line)
        }
    }
}
#endif
