import XCTest
#if os(iOS)
@testable import IrisChat

final class AppStoreUpdateTests: XCTestCase {
    private var defaults: UserDefaults!
    private var suiteName: String!

    override func setUp() {
        super.setUp()
        suiteName = "AppStoreUpdateTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        suiteName = nil
        super.tearDown()
    }

    func testNumericVersionComparison() {
        XCTAssertTrue(appStoreVersionIsNewer("2026.8.19", than: "2026.8.18"))
        XCTAssertTrue(appStoreVersionIsNewer("2.0.0.1", than: "2"))
        XCTAssertFalse(appStoreVersionIsNewer("2.0", than: "2.0.0"))
        XCTAssertFalse(appStoreVersionIsNewer("2026.8.19", than: "2026.8.19.1"))
        XCTAssertFalse(appStoreVersionIsNewer("1.beta", than: "1.0"))
        XCTAssertFalse(appStoreVersionIsNewer("1..2", than: "1.0"))
        XCTAssertFalse(appStoreVersionIsNewer("+2", than: "1"))
        XCTAssertFalse(appStoreVersionIsNewer("２", than: "1"))
        XCTAssertTrue(appStoreVersionIsNewer("18446744073709551616", than: "1"))
        XCTAssertTrue(appStoreVersionIsNewer("18446744073709551617", than: "18446744073709551616"))
        XCTAssertFalse(appStoreVersionIsNewer("2.000", than: "2"))
        XCTAssertTrue(appStoreUpdatesEnabled(for: "0002000.1"))
        XCTAssertTrue(appStoreUpdatesEnabled(for: "18446744073709551616.1"))
        XCTAssertFalse(appStoreUpdatesEnabled(for: "1999.99"))
    }

    func testFloatingHeaderInsetIncludesMeasuredStatusBannersHeight() {
        XCTAssertEqual(
            IrisNavigationHeaderMetrics.contentTopInset(
                topSafeArea: 59,
                isChatHeader: true,
                statusBannersHeight: 52
            ),
            163
        )
    }

    @MainActor
    func testValidResponsePublishesReturnedURLAndBuildsExpectedRequest() async {
        let storeURL = "https://apps.apple.com/fi/app/iris-chat/id6785411684?uo=4"
        let stub = TransportStub(data: lookupJSON(version: "2026.8.19", url: storeURL))
        let controller = makeController(stub: stub, installed: "2026.8.18", country: { "fi" })

        let task = controller.checkIfNeeded()
        await task?.value
        XCTAssertEqual(controller.notice?.storeURL.absoluteString, storeURL)
        let request = await stub.firstRequest()
        let items = URLComponents(url: request!.url!, resolvingAgainstBaseURL: false)?.queryItems
        XCTAssertEqual(items?.first(where: { $0.name == "id" })?.value, "6785411684")
        XCTAssertEqual(items?.first(where: { $0.name == "country" })?.value, "FI")
    }

    @MainActor
    func testInvalidStatusIdentityVersionAndURLStaySilent() async {
        let cases: [(Int, Data)] = [
            (500, lookupJSON()),
            (200, lookupJSON(trackID: 1)),
            (200, lookupJSON(bundleID: "example.wrong")),
            (200, lookupJSON(version: "2026.beta")),
            (200, lookupJSON(url: "http://apps.apple.com/app/id6785411684")),
            (200, lookupJSON(url: "https://apps.apple.com.evil.example/app/id6785411684")),
        ]
        for (index, testCase) in cases.enumerated() {
            let localDefaults = UserDefaults(suiteName: "\(suiteName!).\(index)")!
            defer { localDefaults.removePersistentDomain(forName: "\(suiteName!).\(index)") }
            let stub = TransportStub(data: testCase.1, status: testCase.0)
            let controller = makeController(stub: stub, defaults: localDefaults)
            let task = controller.checkIfNeeded()
            await task?.value
            XCTAssertNil(controller.notice)
        }
    }

    @MainActor
    func testNoUpdateSuccessIsCachedAcrossControllerRecreation() async {
        let first = TransportStub(data: Data(#"{"results":[]}"#.utf8))
        let controller = makeController(stub: first)
        let task = controller.checkIfNeeded()
        await task?.value

        let second = TransportStub(data: lookupJSON())
        let recreated = makeController(stub: second)
        let cachedTask = recreated.checkIfNeeded()

        XCTAssertNil(recreated.notice)
        XCTAssertNil(cachedTask)
        let secondRequests = await second.requestCount()
        XCTAssertEqual(secondRequests, 0)
    }

    @MainActor
    func testDevelopmentVersionSuppressesCachedNoticeAndLookup() async {
        let releaseStub = TransportStub(data: lookupJSON())
        let releaseController = makeController(stub: releaseStub, installed: "2026.8.18")
        let releaseTask = releaseController.checkIfNeeded()
        await releaseTask?.value

        let developmentStub = TransportStub(data: lookupJSON())
        let developmentController = makeController(stub: developmentStub, installed: "0.1.0")
        XCTAssertNil(developmentController.notice)
        let developmentTask = developmentController.checkIfNeeded()
        XCTAssertNil(developmentTask)
        let developmentRequests = await developmentStub.requestCount()
        XCTAssertEqual(developmentRequests, 0)
    }

    @MainActor
    func testCacheTTLAndControllerRecreation() async {
        var clock = Date(timeIntervalSince1970: 1_000_000)
        let first = TransportStub(data: lookupJSON())
        let controller = makeController(stub: first, now: { clock })
        let firstTask = controller.checkIfNeeded()
        await firstTask?.value

        clock.addTimeInterval(21_599)
        let freshStub = TransportStub(data: lookupJSON())
        let fresh = makeController(stub: freshStub, now: { clock })
        XCTAssertNotNil(fresh.notice)
        let freshTask = fresh.checkIfNeeded()
        XCTAssertNil(freshTask)
        let freshRequests = await freshStub.requestCount()
        XCTAssertEqual(freshRequests, 0)

        clock.addTimeInterval(1)
        let staleStub = TransportStub(data: lookupJSON(version: "2026.8.20"))
        let stale = makeController(stub: staleStub, now: { clock })
        XCTAssertNil(stale.notice)
        let staleTask = stale.checkIfNeeded()
        await staleTask?.value
        XCTAssertEqual(stale.notice?.version, "2026.8.20")
    }

    @MainActor
    func testFailureUsesFiveMinuteBackoffButRetriesAfterClockRollback() async {
        var clock = Date(timeIntervalSince1970: 1_000_000)
        let stub = TransportStub(error: URLError(.notConnectedToInternet))
        let controller = makeController(stub: stub, now: { clock })
        let firstTask = controller.checkIfNeeded()
        await firstTask?.value

        clock.addTimeInterval(-60)
        let rollbackTask = controller.checkIfNeeded()
        await rollbackTask?.value

        let backoffTask = controller.checkIfNeeded()
        XCTAssertNil(backoffTask)
        let requestCount = await stub.requestCount()
        XCTAssertEqual(requestCount, 2)

        clock.addTimeInterval(300)
        let retryTask = controller.checkIfNeeded()
        await retryTask?.value

        let newProcess = TransportStub(data: lookupJSON())
        let recreated = makeController(stub: newProcess, now: { clock })
        let recreatedTask = recreated.checkIfNeeded()
        await recreatedTask?.value
        XCTAssertNotNil(recreated.notice)
    }

    @MainActor
    func testConcurrentChecksShareOneInFlightTask() async {
        let stub = TransportStub(data: lookupJSON(), blocked: true)
        let controller = makeController(stub: stub)
        let task = controller.checkIfNeeded()
        let duplicateTask = controller.checkIfNeeded()
        XCTAssertNotNil(duplicateTask)
        controller.checkIfNeeded()
        await stub.waitForRequest()

        await stub.release()
        await task?.value
        XCTAssertNotNil(controller.notice)
        let requestCount = await stub.requestCount()
        XCTAssertEqual(requestCount, 1)
    }

    @MainActor
    func testCountryChangeDiscardsInFlightResult() async {
        var country = "FI"
        let stub = TransportStub(data: lookupJSON(), blocked: true)
        let controller = makeController(stub: stub, country: { country })
        let task = controller.checkIfNeeded()
        await stub.waitForRequest()

        country = "US"
        await stub.release()
        await task?.value
        XCTAssertNotNil(controller.notice)
        let requestCount = await stub.requestCount()
        XCTAssertEqual(requestCount, 2)
        let completedCount = await stub.completedCount()
        XCTAssertEqual(completedCount, 2)
    }

    @MainActor
    func testDismissalPersistsForExactPublishedVersion() async {
        var clock = Date(timeIntervalSince1970: 1_000_000)
        let first = TransportStub(data: lookupJSON())
        let controller = makeController(stub: first, now: { clock })
        let task = controller.checkIfNeeded()
        await task?.value
        XCTAssertNotNil(controller.notice)
        controller.dismissNotice()

        let cachedStub = TransportStub(data: lookupJSON())
        let recreated = makeController(stub: cachedStub, now: { clock })
        XCTAssertNil(recreated.notice)
        let cachedTask = recreated.checkIfNeeded()
        XCTAssertNil(cachedTask)
        let cachedRequests = await cachedStub.requestCount()
        XCTAssertEqual(cachedRequests, 0)

        clock.addTimeInterval(21_600)
        let newerStub = TransportStub(data: lookupJSON(version: "2026.8.20"))
        let newer = makeController(stub: newerStub, now: { clock })
        let newerTask = newer.checkIfNeeded()
        await newerTask?.value
        XCTAssertEqual(newer.notice?.version, "2026.8.20")
    }

    @MainActor
    private func makeController(
        stub: TransportStub,
        installed: String = "2026.8.18",
        defaults: UserDefaults? = nil,
        now: @escaping () -> Date = Date.init,
        country: @escaping () -> String? = { "FI" }
    ) -> AppStoreUpdateController {
        AppStoreUpdateController(
            installedVersion: installed,
            defaults: defaults ?? self.defaults,
            now: now,
            countryCode: country,
            transport: .init { try await stub.load($0) }
        )
    }

}

private actor TransportStub {
    private let data: Data
    private let status: Int
    private let error: Error?
    private var blocked: Bool
    private var continuation: CheckedContinuation<Void, Never>?
    private var requestContinuation: CheckedContinuation<Void, Never>?
    private var requests: [URLRequest] = []
    private var completions = 0

    init(data: Data = Data(), status: Int = 200, error: Error? = nil, blocked: Bool = false) {
        self.data = data
        self.status = status
        self.error = error
        self.blocked = blocked
    }

    func load(_ request: URLRequest) async throws -> (Data, URLResponse) {
        requests.append(request)
        requestContinuation?.resume()
        requestContinuation = nil
        if blocked {
            await withCheckedContinuation { continuation = $0 }
        }
        completions += 1
        if let error { throw error }
        return (data, HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!)
    }

    func release() {
        blocked = false
        continuation?.resume()
        continuation = nil
    }

    func requestCount() -> Int { requests.count }
    func completedCount() -> Int { completions }
    func firstRequest() -> URLRequest? { requests.first }
    func waitForRequest() async {
        guard requests.isEmpty else { return }
        await withCheckedContinuation { requestContinuation = $0 }
    }
}

private func lookupJSON(
    trackID: Int = 6_785_411_684,
    bundleID: String = "fi.siriusbusiness.irischat",
    version: String = "2026.8.19",
    url: String = "https://apps.apple.com/fi/app/iris-chat/id6785411684?uo=4"
) -> Data {
    try! JSONSerialization.data(withJSONObject: [
        "resultCount": 1,
        "results": [[
            "trackId": trackID,
            "bundleId": bundleID,
            "version": version,
            "trackViewUrl": url,
        ]],
    ])
}
#endif
