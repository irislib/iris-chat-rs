import XCTest
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class GroupedSearchSessionTests: XCTestCase {
    func testRequestKeyCanonicalizesQueryAndHidesStaleSnapshots() throws {
        var session = GroupedSearchSession()
        XCTAssertNil(session.request(for: " \n ", discoveryRevision: 1))

        let request = try XCTUnwrap(session.request(for: "  Needle  ", discoveryRevision: 1))
        XCTAssertEqual(request.query, "Needle")
        XCTAssertEqual(request, session.request(for: "Needle", discoveryRevision: 1))

        session.refresh(request) { query, limit in
            buildLargeTestSearchResult(
                query: query,
                personCount: 11,
                contactCount: 25,
                groupCount: 9,
                messageCount: limit
            )
        }
        XCTAssertEqual(session.snapshot(for: request)?.people.count, 11)
        XCTAssertNil(session.snapshot(for: session.request(for: "Other", discoveryRevision: 1)))
        XCTAssertNil(session.snapshot(for: session.request(for: "Needle", discoveryRevision: 2)))
        XCTAssertNil(session.snapshot(for: nil))
    }

    func testRefreshDeduplicatesAndDiscoveryRevisionRequeries() throws {
        var session = GroupedSearchSession()
        var calls: [(String, UInt32)] = []
        let search: (String, UInt32) -> SearchResultSnapshot = { query, limit in
            calls.append((query, limit))
            return buildLargeTestSearchResult(
                query: query,
                personCount: UInt32(calls.count),
                contactCount: 0,
                groupCount: 0,
                messageCount: 0
            )
        }

        let first = try XCTUnwrap(session.request(for: "needle", discoveryRevision: 7))
        session.refresh(first, using: search)
        session.refresh(first, using: search)
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(session.snapshot(for: first)?.people.count, 1)

        let refreshed = try XCTUnwrap(session.request(for: "needle", discoveryRevision: 8))
        XCTAssertNil(session.snapshot(for: refreshed))
        session.refresh(refreshed, using: search)
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(session.snapshot(for: refreshed)?.people.count, 2)

        session.refresh(nil, using: search)
        XCTAssertEqual(calls.map { $0.0 }, ["needle", "needle", ""])
        XCTAssertNil(session.snapshot(for: refreshed))
        session.refresh(refreshed, using: search)
        XCTAssertEqual(calls.count, 4)
    }

    func testQueryChangeResetsExpansionAndMessageLimit() throws {
        var session = GroupedSearchSession()
        let request = try XCTUnwrap(session.request(for: "needle", discoveryRevision: 1))
        session.refresh(request) { query, _ in
            buildLargeTestSearchResult(
                query: query,
                personCount: 0,
                contactCount: 0,
                groupCount: 0,
                messageCount: 0
            )
        }
        session.viewMore(.contacts)
        session.viewMore(.messages)
        session.viewMore(.messages)
        session.viewMore(.messages)
        XCTAssertEqual(session.messageLimit, 150)
        XCTAssertEqual(session.expandedSections, [.contacts, .messages])

        session.queryChanged(" needle ")
        XCTAssertEqual(session.messageLimit, 150)
        session.queryChanged("different")
        XCTAssertEqual(session.messageLimit, 50)
        XCTAssertTrue(session.expandedSections.isEmpty)
    }
}
