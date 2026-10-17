#if os(macOS)
import XCTest
@testable import IrisChatMac

final class DesktopImageZoomTests: XCTestCase {
    func testFitUsesImageAspectRatioAndHasNoPanAtNormalSize() {
        let viewport = CGSize(width: 900, height: 600)
        let portrait = CGSize(width: 600, height: 1200)
        XCTAssertEqual(IrisDesktopImageZoomGeometry.fittedSize(image: portrait, viewport: viewport),
                       CGSize(width: 300, height: 600))
        XCTAssertEqual(IrisDesktopImageZoomGeometry.offset(at: .zero, image: portrait, viewport: viewport, scale: 1), .zero)
    }

    func testZoomedPanReachesImageEdgesWithoutExposingEmptySpace() {
        let viewport = CGSize(width: 900, height: 600)
        let image = CGSize(width: 1200, height: 800)
        let center = CGPoint(x: 450, y: 300)
        XCTAssertEqual(IrisDesktopImageZoomGeometry.offset(at: center, image: image, viewport: viewport, scale: 3), .zero)
        XCTAssertEqual(IrisDesktopImageZoomGeometry.offset(at: .zero, image: image, viewport: viewport, scale: 3),
                       CGSize(width: 900, height: 600))
        XCTAssertEqual(IrisDesktopImageZoomGeometry.offset(at: CGPoint(x: 10_000, y: 10_000), image: image, viewport: viewport, scale: 3),
                       CGSize(width: -900, height: -600))
    }

    func testNarrowImageStaysCenteredOnAxisThatDoesNotOverflow() {
        let offset = IrisDesktopImageZoomGeometry.offset(at: .zero,
            image: CGSize(width: 200, height: 1200), viewport: CGSize(width: 900, height: 600), scale: 3)
        XCTAssertEqual(offset.width, 0)
        XCTAssertEqual(offset.height, 600)
    }
}
#endif
