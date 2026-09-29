import XCTest
import SwiftUI
#if os(iOS)
import UIKit
#endif
#if os(macOS)
@testable import IrisChatMac
#else
@testable import IrisChat
#endif

final class CallAudioDevicesTests: XCTestCase {
#if os(iOS)
    @MainActor
    func testRouteRefreshTracksExternalChoiceWithoutReapplyingSpeaker() throws {
        var actual = IrisCallAudioRoute(speaker: true, name: "Speaker")
        var overrides: [Bool] = []
        let routing = IrisCallAudioRouting(read: { actual }, overrideSpeaker: { overrides.append($0) })
        var displayed = routing.route
        routing.onChange = { displayed = $0 }
        actual = .init(speaker: false, external: true, name: "Headphones")
        routing.refresh()
        XCTAssertEqual(displayed, actual)
        XCTAssertTrue(overrides.isEmpty, "CallKit activation and route notifications must never replace the user's route")
    }

    @MainActor
    func testSpeakerToggleReadsTheCurrentRouteAndReportsFailure() throws {
        var actual = IrisCallAudioRoute(speaker: false, name: "Phone")
        var requested: [Bool] = []
        var fails = false
        let routing = IrisCallAudioRouting(read: { actual }, overrideSpeaker: { enabled in
            requested.append(enabled)
            if fails { throw NSError(domain: "test", code: 1) }
            actual.speaker = enabled
        })
        // The route can change before the asynchronous notification arrives.
        actual.speaker = true
        try routing.toggleSpeaker()
        XCTAssertEqual(requested, [false])
        XCTAssertFalse(routing.route.speaker)
        fails = true
        XCTAssertThrowsError(try routing.toggleSpeaker())
        XCTAssertFalse(routing.route.speaker)
    }

    @MainActor
    func testNativeAudioPickerRemainsAccessibleAndRenders() async throws {
        let controller = UIHostingController(rootView:
            VStack(spacing: 8) {
                IrisCallRoutePicker(routeName: "Headphones")
                    .frame(width: 58, height: 58)
                    .background(.white.opacity(0.16), in: Circle())
                Text("Audio").font(.caption).foregroundStyle(.white)
            }
            .frame(width: 160, height: 130)
            .background(Color(red: 0.08, green: 0.075, blue: 0.12))
            .ignoresSafeArea())
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 160, height: 130)
        window.rootViewController = controller
        window.isHidden = false
        defer { window.isHidden = true }
        controller.view.frame = window.bounds
        window.layoutIfNeeded()
        await Task.yield()
        controller.view.layoutIfNeeded()
        func picker(_ view: UIView) -> UIView? {
            if view.accessibilityIdentifier == "callAudioRouteButton" { return view }
            return view.subviews.lazy.compactMap(picker).first
        }
        let routePicker = try XCTUnwrap(picker(controller.view))
        XCTAssertEqual(routePicker.accessibilityLabel, "Audio output")
        XCTAssertEqual(routePicker.accessibilityValue, "Headphones")
        let image = UIGraphicsImageRenderer(bounds: controller.view.bounds).image { context in
            controller.view.layer.render(in: context.cgContext)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = "ios-call-audio-picker"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
#elseif os(macOS)
    private let mic = IrisMacAudioDevice(id: "usb-mic", name: "USB microphone", deviceID: 30)
    private let speaker = IrisMacAudioDevice(id: "headphones", name: "Headphones", deviceID: 40)

    private var available: IrisMacAudioDeviceSnapshot {
        .init(inputs: [mic], outputs: [speaker], defaultInput: 10, defaultOutput: 20)
    }

    @MainActor
    func testIndependentSelectionsAndSystemDefault() {
        var applied: [IrisMacAudioDeviceSelection] = []
        let devices = IrisMacCallAudioDevices(defaults: nil) { selection, done in applied.append(selection); done(true) }
        devices.update(available)
        XCTAssertEqual(applied.last, .init())
        devices.select(input: mic.id)
        XCTAssertEqual(applied.last, .init(input: 30, output: 20))
        devices.select(output: speaker.id)
        XCTAssertEqual(applied.last, .init(input: 30, output: 40))
        devices.select(input: "")
        XCTAssertEqual(applied.last, .init(input: 10, output: 40))
        devices.select(output: "")
        XCTAssertEqual(applied.last, .init())
    }

    @MainActor
    func testUnpluggedDeviceReturnsToDefaultWithoutReselectingOnReconnect() {
        var applied: [IrisMacAudioDeviceSelection] = []
        let devices = IrisMacCallAudioDevices(defaults: nil) { selection, done in applied.append(selection); done(true) }
        devices.update(available)
        devices.select(input: mic.id, output: speaker.id)
        devices.update(.init(inputs: [], outputs: [speaker], defaultInput: 11, defaultOutput: 20))
        XCTAssertEqual(devices.input, "")
        XCTAssertEqual(devices.output, speaker.id)
        XCTAssertEqual(applied.last, .init(input: 11, output: 40))
        devices.update(available)
        XCTAssertEqual(devices.input, "")
        XCTAssertEqual(applied.last, .init(input: 10, output: 40))
    }

    @MainActor
    func testFailedQueuedChoicesRevertToLastSuccessfullyAppliedDevices() {
        var completions: [(Bool) -> Void] = []
        let devices = IrisMacCallAudioDevices(defaults: nil) { _, done in completions.append(done) }
        devices.update(available)
        completions.removeFirst()(true)
        devices.select(input: mic.id)
        devices.select(output: speaker.id)
        completions.removeFirst()(false)
        XCTAssertEqual(devices.output, speaker.id, "A stale failure must not overwrite the current choice")
        completions.removeFirst()(false)
        XCTAssertEqual(devices.input, "")
        XCTAssertEqual(devices.output, "")
        devices.select(input: mic.id)
        devices.select(output: speaker.id)
        completions.removeFirst()(true)
        completions.removeFirst()(false)
        XCTAssertEqual(devices.input, mic.id)
        XCTAssertEqual(devices.output, "")
    }

    func testAudioUnitSelectionChecksBothIndependentDevices() throws {
        let selection = IrisMacAudioDeviceSelection(input: 10, output: 20)
        var input: UInt32 = 0, output: UInt32 = 0
        try selection.apply(write: { isInput, value in
            if isInput { input = value } else { output = value }
        }, read: { $0 ? input : output })
        XCTAssertEqual(input, 10)
        XCTAssertEqual(output, 20)
        var shared: UInt32 = 0
        XCTAssertThrowsError(try selection.apply(write: { _, value in shared = value }, read: { _ in shared }),
                             "A coupled unit cannot silently select the wrong microphone")
    }

    @MainActor
    func testDeviceSheetRendersBothSelectors() throws {
        let devices = IrisMacCallAudioDevices(defaults: nil) { _, done in done(true) }
        devices.update(available)
        devices.select(input: mic.id, output: speaker.id)
        let renderer = ImageRenderer(content: IrisCallAudioDevicesSheet(devices: devices)
            .environment(\.colorScheme, .dark))
        renderer.scale = 2
        let image = try XCTUnwrap(renderer.nsImage)
        let attachment = XCTAttachment(image: image)
        attachment.name = "mac-call-audio-devices"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
#endif
}
