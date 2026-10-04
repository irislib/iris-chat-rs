import SwiftUI
import ImageIO
import XCTest
#if os(macOS)
import AppKit
@testable import IrisChatMac
#else
import UIKit
@testable import IrisChat
#endif

@MainActor
final class FavoriteAvatarLayoutTests: XCTestCase {
    func testFavoriteChangesAvatarIndependentlyOfPublicBadge() throws {
        for badge: SocialBadge? in [nil, .following, .warning, .muted] {
            var connection = SocialConnectionSnapshot(badge: badge, followDistance: nil, followedByFriends: 0, description: "Public relationship")
            let ordinary = try render(connection, dark: false)
            connection.isFavorite = true
            XCTAssertGreaterThan(try pixelDifference(render(connection, dark: false), ordinary), 10,
                              "A private favorite must be visible alongside every public relationship")
            XCTAssertEqual(connection.badge, badge)
            connection.isFavorite = false
            XCTAssertLessThanOrEqual(try pixelDifference(render(connection, dark: false), ordinary), 1,
                           "Removing a favorite restores the same avatar and public badge")
        }
    }

    func testFavoriteCoexistsWithSocialAndNearbyInBothThemes() throws {
        let connection = SocialConnectionSnapshot(badge: .following, followDistance: 1, followedByFriends: 0,
                                                  description: "Followed by you", isFavorite: true)
        for dark in [false, true] {
            let png = try render(connection, dark: dark, nearby: true)
            let name = "favorite-avatar-\(dark ? "dark" : "light")"
            let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
            if let output = ProcessInfo.processInfo.environment["IRIS_VISUAL_OUTPUT"] {
                try png.write(to: URL(fileURLWithPath: output).appendingPathComponent(name + ".png"))
            }
        }
    }

    // Core Graphics can vary antialiasing by one channel level between identical renders.
    private func pixelDifference(_ lhs: Data, _ rhs: Data) throws -> Int {
        func pixels(_ png: Data) throws -> [UInt8] {
            let source = try XCTUnwrap(CGImageSourceCreateWithData(png as CFData, nil))
            let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
            var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
            try bytes.withUnsafeMutableBytes { buffer in
                let context = try XCTUnwrap(CGContext(data: buffer.baseAddress, width: image.width,
                    height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4,
                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            }
            return bytes
        }
        let a = try pixels(lhs), b = try pixels(rhs)
        XCTAssertEqual(a.count, b.count)
        return zip(a, b).map { abs(Int($0) - Int($1)) }.max() ?? 0
    }

    private func render(_ connection: SocialConnectionSnapshot, dark: Bool, nearby: Bool = false) throws -> Data {
        let view = HStack(spacing: 24) {
            ForEach([CGFloat(32), 48, 80], id: \.self) { size in
                IrisAvatar(socialConnection: connection, label: "Alice", size: size)
                    .overlay(alignment: .bottomTrailing) {
                        if nearby { IrisNearbyBadge(size: max(14, min(22, size * 0.36))).offset(x: 2, y: 2) }
                    }
            }
        }
        .padding(16)
        .background(dark ? Color.black : Color.white)
        .environment(\.colorScheme, dark ? .dark : .light)
        .environment(\.irisPalette, dark ? .dark : .light)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        #if os(macOS)
        let image = try XCTUnwrap(renderer.nsImage)
        let data = try XCTUnwrap(image.tiffRepresentation)
        return try XCTUnwrap(NSBitmapImageRep(data: data)?.representation(using: .png, properties: [:]))
        #else
        return try XCTUnwrap(renderer.uiImage?.pngData())
        #endif
    }
}
