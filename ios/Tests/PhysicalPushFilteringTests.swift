#if os(iOS)
import XCTest
import UserNotifications
import UIKit
@testable import IrisChat

/// Opt-in APNs inspection on a selected physical phone. Keeps account, chats,
/// and notification preferences intact. Inspection should leave the usual
/// IRIS_ENABLE_NOTIFICATIONS_FOR_AUTOMATION override off, so app foregrounding
/// does not clear delivered notifications before they can be checked.
@MainActor
final class PhysicalPushFilteringTests: XCTestCase {
    func testPrepareAndInspectPhysicalPush() async throws {
        let env = ProcessInfo.processInfo.environment
        guard env["IRIS_PHYSICAL_PUSH_TEST"] == "1" else {
            throw XCTSkip("Requires an explicitly selected physical phone")
        }
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        XCTAssertTrue([.authorized, .provisional, .ephemeral].contains(settings.authorizationStatus))
        let root = try XCTUnwrap(FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: "group.fi.siriusbusiness.irischat"
        )).appendingPathComponent("Library", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let startedURL = root.appendingPathComponent("physical-push-started")
        if env["IRIS_PHYSICAL_PUSH_PREPARE"] == "1" {
            UIApplication.shared.registerForRemoteNotifications()
            let token = await MobilePushTokenCenter.shared.waitForApnsToken(timeoutNanoseconds: 15_000_000_000)
            try XCTUnwrap(token).write(to: root.appendingPathComponent("physical-push-token"), atomically: true, encoding: .utf8)
            try String(Date().timeIntervalSince1970).write(to: startedURL, atomically: true, encoding: .utf8)
        }
        let notifications = await center.deliveredNotifications()
        let prefix = env["IRIS_PHYSICAL_PUSH_PREFIX"] ?? "physical-filtering-"
        let selected = notifications.filter {
            ($0.request.content.userInfo["iris_push_e2e_id"] as? String)?.hasPrefix(prefix) == true
        }
        let rows = selected.map { notification in
            ["id": notification.request.content.userInfo["iris_push_e2e_id"] as? String ?? "",
             "body": notification.request.content.body,
             "title": notification.request.content.title]
        }
        try JSONSerialization.data(withJSONObject: rows).write(to: root.appendingPathComponent("physical-push-delivered.json"), options: .atomic)
        if let expected = env["IRIS_PHYSICAL_PUSH_EXPECTED_IDS"] {
            let started = try XCTUnwrap(Double(String(contentsOf: startedURL, encoding: .utf8)))
            let recent = notifications.filter { $0.date.timeIntervalSince1970 >= started }
            // Inspect every new notification, including one whose filtered
            // content has lost its test marker. This catches blank leftovers.
            let actual = recent.map { $0.request.content.userInfo["iris_push_e2e_id"] as? String ?? "<unmarked>" }
            XCTAssertEqual(actual.sorted(), expected.split(separator: ",").map(String.init).sorted())
            XCTAssertTrue(recent.allSatisfy { !$0.request.content.body.isEmpty })
        }
        if env["IRIS_PHYSICAL_PUSH_CLEANUP"] == "1" {
            center.removeDeliveredNotifications(withIdentifiers: selected.map { $0.request.identifier })
            try? FileManager.default.removeItem(at: root.appendingPathComponent("physical-push-token"))
            try? FileManager.default.removeItem(at: startedURL)
            MobilePushDeliveryProbe.clear()
        }
        print("PHYSICAL_PUSH_INSPECTION_COMPLETE count=\(rows.count)")
    }
}
#endif
