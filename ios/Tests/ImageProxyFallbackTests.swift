import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class ImageProxyFallbackTests: XCTestCase {
    private let original = "https://original.example/image.png"

    func testProxyFailuresNeverContactOriginalByDefault() async {
        for failure in ["http-error", "decode-error", "network-error"] {
            let (session, requests) = makeSession()
            defer { session.invalidateAndCancel() }
            let image = await loadIrisHttpAvatarImage(
                urls: candidates(failure: failure, fallback: false), originalURL: original, maxPixelSize: 64, session: session
            )
            XCTAssertNil(image, failure)
            XCTAssertEqual(requests.urls.count, 1, failure)
            XCTAssertEqual(requests.urls.first?.host, "proxy.example", failure)
        }
    }

    func testOptInRetriesOriginalAfterHttpNetworkAndDecodeFailures() async {
        for failure in ["http-error", "decode-error", "network-error"] {
            let (session, requests) = makeSession()
            defer { session.invalidateAndCancel() }
            let image = await loadIrisHttpAvatarImage(
                urls: candidates(failure: failure, fallback: true), originalURL: original, maxPixelSize: 64, session: session
            )
            XCTAssertNotNil(image, failure)
            XCTAssertEqual(requests.urls.map(\.host), ["proxy.example", "original.example"], failure)
        }
    }

    func testSuccessfulProxyDoesNotContactOriginalWhenOptedIn() async {
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let image = await loadIrisHttpAvatarImage(
            urls: candidates(failure: "image", fallback: true), originalURL: original, maxPixelSize: 64, session: session
        )
        XCTAssertNotNil(image)
        XCTAssertEqual(requests.urls.count, 1)
        XCTAssertEqual(requests.urls.first?.host, "proxy.example")
    }

    func testCancelledRequestDoesNotTryOriginal() async {
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let image = await loadIrisHttpAvatarImage(
            urls: candidates(failure: "cancelled", fallback: true), originalURL: original, maxPixelSize: 64, session: session
        )
        XCTAssertNil(image)
        XCTAssertEqual(requests.urls.count, 1)
    }

    func testInvalidProxyConfigurationStaysOfflineUntilOptedIn() async {
        var preferences = makeAppState().preferences
        preferences.imageProxyUrl = "invalid"
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let blocked = irisHttpAvatarURLs(original, preferences: preferences, pixelSize: 64) ?? []
        XCTAssertTrue(blocked.isEmpty)
        let blockedImage = await loadIrisHttpAvatarImage(urls: blocked, originalURL: original, maxPixelSize: 64, session: session)
        XCTAssertNil(blockedImage)
        XCTAssertTrue(requests.urls.isEmpty)
        preferences.imageProxyFallbackEnabled = true
        let allowed = irisHttpAvatarURLs(original, preferences: preferences, pixelSize: 64) ?? []
        let image = await loadIrisHttpAvatarImage(urls: allowed, originalURL: original, maxPixelSize: 64, session: session)
        XCTAssertNotNil(image)
        XCTAssertEqual(requests.urls.map(\.absoluteString), [original])
    }

    func testCrossOriginProxyRedirectRequiresOptIn() async {
        for fallback in [false, true] {
            let (session, requests) = makeSession()
            defer { session.invalidateAndCancel() }
            let image = await loadIrisHttpAvatarImage(
                urls: candidates(failure: "redirect-cross-origin", fallback: fallback),
                originalURL: original, allowOriginalRedirects: fallback, maxPixelSize: 64, session: session
            )
            XCTAssertEqual(image != nil, fallback)
            XCTAssertEqual(requests.urls.map(\.host), fallback ? ["proxy.example", "original.example"] : ["proxy.example"])
        }
    }

    func testSameOriginProxyRedirectCanLoadImageWithoutFallback() async {
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let image = await loadIrisHttpAvatarImage(
            urls: candidates(failure: "redirect-same-origin", fallback: false),
            originalURL: original, maxPixelSize: 64, session: session
        )
        XCTAssertNotNil(image)
        XCTAssertEqual(requests.urls.map(\.host), ["proxy.example", "proxy.example"])
    }

    func testProxyRedirectLoopStopsWithoutContactingOriginal() async {
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let image = await loadIrisHttpAvatarImage(
            urls: candidates(failure: "redirect-loop", fallback: false),
            originalURL: original, maxPixelSize: 64, session: session
        )
        XCTAssertNil(image)
        XCTAssertEqual(requests.urls.count, 6)
        XCTAssertTrue(requests.urls.allSatisfy { $0.host == "proxy.example" })
    }

    func testOriginalImageCanFollowRedirectWhenProxyIsDisabled() async {
        let (session, requests) = makeSession()
        defer { session.invalidateAndCancel() }
        let url = "https://first-original.example/redirect-cross-origin"
        let image = await loadIrisHttpAvatarImage(
            urls: [url], originalURL: url, allowOriginalRedirects: true, maxPixelSize: 64, session: session
        )
        XCTAssertNotNil(image)
        XCTAssertEqual(requests.urls.map(\.host), ["first-original.example", "original.example"])
    }

    func testAlreadyProxiedImageCannotRedirectOffProxyWithoutOptIn() async {
        let original = "https://proxy.example/redirect-cross-origin"
        for fallback in [false, true] {
            var preferences = makeAppState().preferences
            preferences.imageProxyUrl = "https://proxy.example"
            preferences.imageProxyFallbackEnabled = fallback
            let source = irisAvatarImageSource(pictureUrl: original, preferences: preferences, pixelSize: 64)
            guard case .http(let urls, let originalURL, let allowRedirects) = source else {
                XCTFail("expected HTTP avatar source")
                return
            }
            XCTAssertEqual(urls, [original])
            let (session, requests) = makeSession()
            defer { session.invalidateAndCancel() }
            let image = await loadIrisHttpAvatarImage(
                urls: urls, originalURL: originalURL, allowOriginalRedirects: allowRedirects,
                maxPixelSize: 64, session: session
            )
            XCTAssertEqual(image != nil, fallback)
            XCTAssertEqual(requests.urls.map(\.host), fallback ? ["proxy.example", "original.example"] : ["proxy.example"])
        }
    }

    private func candidates(failure: String, fallback: Bool) -> [String] {
        var preferences = makeAppState().preferences
        preferences.imageProxyUrl = "https://proxy.example/\(failure)"
        preferences.imageProxyFallbackEnabled = fallback
        return irisHttpAvatarURLs(original, preferences: preferences, pixelSize: 64) ?? []
    }

    private func makeSession() -> (URLSession, ImageRequestRecorder) {
        let requests = ImageRequestRecorder()
        ImageStubURLProtocol.recorder = requests
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ImageStubURLProtocol.self]
        configuration.timeoutIntervalForRequest = 3
        configuration.timeoutIntervalForResource = 5
        return (URLSession(configuration: configuration), requests)
    }
}

private final class ImageRequestRecorder {
    private let lock = NSLock()
    private var recorded: [URL] = []

    var urls: [URL] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func append(_ url: URL) {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(url)
    }
}

private final class ImageStubURLProtocol: URLProtocol {
    static var recorder: ImageRequestRecorder?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url else { return }
        Self.recorder?.append(url)
        if url.path.contains("network-error") || url.path.contains("cancelled") {
            let code: URLError.Code = url.path.contains("cancelled") ? .cancelled : .cannotConnectToHost
            client?.urlProtocol(self, didFailWithError: URLError(code))
            return
        }
        if url.path.contains("redirect-") {
            let target: URL
            if url.path.contains("redirect-loop") {
                target = url
            } else if url.path.contains("redirect-same-origin") {
                target = URL(string: "/image", relativeTo: url)!.absoluteURL
            } else {
                target = URL(string: "https://original.example/image.png")!
            }
            let response = HTTPURLResponse(
                url: url, statusCode: 302, httpVersion: "HTTP/1.1", headerFields: ["Location": target.absoluteString]
            )!
            client?.urlProtocol(self, wasRedirectedTo: URLRequest(url: target), redirectResponse: response)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocolDidFinishLoading(self)
            return
        }
        let response = HTTPURLResponse(
            url: url, statusCode: url.path.contains("http-error") ? 502 : 200,
            httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "image/png"]
        )!
        let png = Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGOwmRb1HwAEagIsXv3rtwAAAABJRU5ErkJggg==")!
        let data = url.path.contains("decode-error") ? Data("not an image".utf8) : png
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}
