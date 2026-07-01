import XCTest
@testable import IrisChat

final class IrisChatNearbyQueueTests: XCTestCase {
    func testNearbyPeripheralWriteQueueDropsOldestChunks() {
        var queue = IrisNearbyPeripheralWriteQueue()
        for value in 0..<5 {
            queue.append(Data(repeating: UInt8(value), count: 1))
        }

        let dropped = queue.trimToLimits(maxChunks: 3, maxBytes: 64)

        XCTAssertEqual(dropped, 2)
        XCTAssertEqual(queue.count, 3)
        XCTAssertEqual(queue.pendingBytes, 3)
        XCTAssertEqual(queue.popFirst(), Data([2]))
        XCTAssertEqual(queue.popFirst(), Data([3]))
        XCTAssertEqual(queue.popFirst(), Data([4]))
        XCTAssertTrue(queue.isEmpty)
    }

    func testNearbyPeripheralWriteQueueDropsOldestBytes() {
        var queue = IrisNearbyPeripheralWriteQueue()
        queue.append(Data(repeating: 1, count: 100))
        queue.append(Data(repeating: 2, count: 100))
        queue.append(Data(repeating: 3, count: 100))

        let dropped = queue.trimToLimits(maxChunks: 10, maxBytes: 150)

        XCTAssertEqual(dropped, 2)
        XCTAssertEqual(queue.count, 1)
        XCTAssertEqual(queue.pendingBytes, 100)
        XCTAssertEqual(queue.popFirst(), Data(repeating: 3, count: 100))
        XCTAssertTrue(queue.isEmpty)
    }
}
