import Foundation

#if canImport(IrisChatCoreFFI)
import IrisChatCoreFFI
#endif

private let ffiCallSuccess: Int8 = 0
private let ffiCallError: Int8 = 1
private let ffiCallUnexpectedError: Int8 = 2
private let ffiCallCancelled: Int8 = 3

enum SafeFfiDispatchError: LocalizedError {
    case callError(String)
    case rustPanic(String)
    case cancelled
    case unknownStatus(Int8)

    var errorDescription: String? {
        switch self {
        case .callError(let message):
            return message.isEmpty ? "FFI call failed" : message
        case .rustPanic(let message):
            return message.isEmpty ? "Rust panic" : message
        case .cancelled:
            return "FFI call cancelled"
        case .unknownStatus(let code):
            return "Unexpected FFI status \(code)"
        }
    }
}

extension FfiApp {
    func stateSafely() -> AppState {
        do {
            uniffiEnsureIrisChatCoreInitialized()
            var callStatus = makeRustCallStatus()
            let buffer = uniffi_iris_chat_core_fn_method_ffiapp_state(
                self.uniffiClonePointer(),
                &callStatus
            )
            try checkRustCallStatus(callStatus)
            return try FfiConverterTypeAppState_lift(buffer)
        } catch {
            logSafeFfiFailure("ffiapp.state", error)
            return fallbackAppState(toast: "Iris needs restart. Copy support bundle in Settings.")
        }
    }

    func dispatchSafely(action: AppAction) throws {
        uniffiEnsureIrisChatCoreInitialized()

        var callStatus = makeRustCallStatus()
        uniffi_iris_chat_core_fn_method_ffiapp_dispatch(
            self.uniffiClonePointer(),
            FfiConverterTypeAppAction_lower(action),
            &callStatus
        )
        try checkRustCallStatus(callStatus)
    }

    func searchSafely(
        query: String,
        scopeChatId: String?,
        limit: UInt32
    ) -> SearchResultSnapshot {
        do {
            uniffiEnsureIrisChatCoreInitialized()
            var callStatus = makeRustCallStatus()
            let buffer = uniffi_iris_chat_core_fn_method_ffiapp_search(
                self.uniffiClonePointer(),
                try lowerString(query),
                try lowerOptionalString(scopeChatId),
                limit,
                &callStatus
            )
            try checkRustCallStatus(callStatus)
            return try FfiConverterTypeSearchResultSnapshot_lift(buffer)
        } catch {
            logSafeFfiFailure("ffiapp.search", error)
            return SearchResultSnapshot(
                query: query,
                scopeChatId: scopeChatId,
                contacts: [],
                groups: [],
                messages: [],
                shortcut: nil
            )
        }
    }

    func buildNearbyPresenceEventJsonSafely(
        peerID: String,
        myNonce: String,
        theirNonce: String,
        profileEventID: String
    ) -> String {
        ffiString("ffiapp.buildNearbyPresenceEventJson") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_build_nearby_presence_event_json(
                self.uniffiClonePointer(),
                lowerString(peerID),
                lowerString(myNonce),
                lowerString(theirNonce),
                lowerString(profileEventID),
                status
            )
        }
    }

    func exportSupportBundleJsonSafely() -> String {
        ffiString("ffiapp.exportSupportBundleJson", fallback: "{}") { status in
            uniffi_iris_chat_core_fn_method_ffiapp_export_support_bundle_json(
                self.uniffiClonePointer(),
                status
            )
        }
    }

    func prepareForSuspendSafely() {
        ffiVoid("ffiapp.prepareForSuspend") { status in
            uniffi_iris_chat_core_fn_method_ffiapp_prepare_for_suspend(
                self.uniffiClonePointer(),
                status
            )
        }
    }

    func ingestNearbyEventJsonSafely(eventJson: String) -> Bool {
        ffiBool("ffiapp.ingestNearbyEventJson") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_ingest_nearby_event_json(
                self.uniffiClonePointer(),
                lowerString(eventJson),
                status
            )
        }
    }

    func ingestNearbyEventJsonWithTransportSafely(eventJson: String, transport: String) -> Bool {
        ffiBool("ffiapp.ingestNearbyEventJsonWithTransport") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_ingest_nearby_event_json_with_transport(
                self.uniffiClonePointer(),
                lowerString(eventJson),
                lowerString(transport),
                status
            )
        }
    }

    @discardableResult
    func listenForUpdatesSafely(reconciler: AppReconciler) -> Bool {
        ffiVoid("ffiapp.listenForUpdates") { status in
            uniffi_iris_chat_core_fn_method_ffiapp_listen_for_updates(
                self.uniffiClonePointer(),
                FfiConverterCallbackInterfaceAppReconciler_lower(reconciler),
                status
            )
        }
    }

    func nearbyDecodeFrameSafely(frame: Data) -> String {
        ffiString("ffiapp.nearbyDecodeFrame") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_nearby_decode_frame(
                self.uniffiClonePointer(),
                lowerData(frame),
                status
            )
        }
    }

    func nearbyEncodeFrameSafely(envelopeJson: String) -> Data {
        ffiData("ffiapp.nearbyEncodeFrame") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_nearby_encode_frame(
                self.uniffiClonePointer(),
                lowerString(envelopeJson),
                status
            )
        }
    }

    func nearbyFrameBodyLenFromHeaderSafely(header: Data) -> Int {
        Int(ffiInt32("ffiapp.nearbyFrameBodyLenFromHeader", fallback: -1) { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_nearby_frame_body_len_from_header(
                self.uniffiClonePointer(),
                lowerData(header),
                status
            )
        })
    }

    func verifyNearbyPresenceEventJsonSafely(
        eventJson: String,
        peerID: String,
        myNonce: String,
        theirNonce: String
    ) -> String {
        ffiString("ffiapp.verifyNearbyPresenceEventJson") { status in
            try uniffi_iris_chat_core_fn_method_ffiapp_verify_nearby_presence_event_json(
                self.uniffiClonePointer(),
                lowerString(eventJson),
                lowerString(peerID),
                lowerString(myNonce),
                lowerString(theirNonce),
                status
            )
        }
    }

    func shutdownSafely() {
        ffiVoid("ffiapp.shutdown") { status in
            uniffi_iris_chat_core_fn_method_ffiapp_shutdown(self.uniffiClonePointer(), status)
        }
    }
}

private func makeRustCallStatus() -> RustCallStatus {
    RustCallStatus(
        code: ffiCallSuccess,
        errorBuf: RustBuffer(capacity: 0, len: 0, data: nil)
    )
}

private func checkRustCallStatus(_ callStatus: RustCallStatus) throws {
    switch callStatus.code {
    case ffiCallSuccess:
        return
    case ffiCallError:
        throw SafeFfiDispatchError.callError(errorMessage(from: callStatus.errorBuf))
    case ffiCallUnexpectedError:
        throw SafeFfiDispatchError.rustPanic(errorMessage(from: callStatus.errorBuf))
    case ffiCallCancelled:
        freeRustBuffer(callStatus.errorBuf)
        throw SafeFfiDispatchError.cancelled
    default:
        freeRustBuffer(callStatus.errorBuf)
        throw SafeFfiDispatchError.unknownStatus(callStatus.code)
    }
}

private func errorMessage(from buffer: RustBuffer) -> String {
    defer { freeRustBuffer(buffer) }
    guard let data = buffer.data, buffer.len > 0 else {
        return ""
    }
    let bytes = UnsafeBufferPointer<UInt8>(start: data, count: Int(buffer.len))
    return String(bytes: bytes, encoding: .utf8) ?? ""
}

private func freeRustBuffer(_ buffer: RustBuffer) {
    guard buffer.data != nil || buffer.len > 0 || buffer.capacity > 0 else {
        return
    }
    var status = makeRustCallStatus()
    ffi_iris_chat_core_rustbuffer_free(buffer, &status)
}

private func ffiString(
    _ label: String,
    fallback: String = "",
    _ body: (UnsafeMutablePointer<RustCallStatus>) throws -> RustBuffer
) -> String {
    do {
        var status = makeRustCallStatus()
        let buffer = try body(&status)
        try checkRustCallStatus(status)
        return try liftString(buffer)
    } catch {
        logSafeFfiFailure(label, error)
        return fallback
    }
}

private func ffiData(
    _ label: String,
    fallback: Data = Data(),
    _ body: (UnsafeMutablePointer<RustCallStatus>) throws -> RustBuffer
) -> Data {
    do {
        var status = makeRustCallStatus()
        let buffer = try body(&status)
        try checkRustCallStatus(status)
        return try liftData(buffer)
    } catch {
        logSafeFfiFailure(label, error)
        return fallback
    }
}

private func ffiBool(
    _ label: String,
    fallback: Bool = false,
    _ body: (UnsafeMutablePointer<RustCallStatus>) throws -> Int8
) -> Bool {
    do {
        var status = makeRustCallStatus()
        let value = try body(&status)
        try checkRustCallStatus(status)
        return value != 0
    } catch {
        logSafeFfiFailure(label, error)
        return fallback
    }
}

private func ffiInt32(
    _ label: String,
    fallback: Int32,
    _ body: (UnsafeMutablePointer<RustCallStatus>) throws -> Int32
) -> Int32 {
    do {
        var status = makeRustCallStatus()
        let value = try body(&status)
        try checkRustCallStatus(status)
        return value
    } catch {
        logSafeFfiFailure(label, error)
        return fallback
    }
}

private func fallbackAppState(toast: String?) -> AppState {
    AppState(
        rev: 0,
        router: Router(defaultScreen: .welcome, screenStack: []),
        account: nil,
        deviceRoster: nil,
        busy: BusyState(
            creatingAccount: false,
            restoringSession: false,
            linkingDevice: false,
            creatingChat: false,
            creatingGroup: false,
            sendingMessage: false,
            updatingRoster: false,
            updatingGroup: false,
            creatingInvite: false,
            acceptingInvite: false,
            syncingNetwork: false,
            uploadingAttachment: false
        ),
        chatList: [],
        currentChat: nil,
        groupDetails: nil,
        publicInvite: nil,
        linkDevice: nil,
        networkStatus: nil,
        mobilePush: MobilePushSyncSnapshot(
            ownerPubkeyHex: nil,
            messageAuthorPubkeys: [],
            inviteResponsePubkeys: [],
            sessions: []
        ),
        preferences: PreferencesSnapshot(
            sendTypingIndicators: true,
            sendReadReceipts: true,
            desktopNotificationsEnabled: true,
            inviteAcceptanceNotificationsEnabled: true,
            startupAtLoginEnabled: false,
            nearbyBluetoothEnabled: false,
            nearbyLanEnabled: false,
            nostrRelayUrls: [
                "wss://relay.damus.io",
                "wss://nos.lol",
                "wss://relay.primal.net",
                "wss://relay.snort.social",
                "wss://temp.iris.to"
            ],
            imageProxyEnabled: true,
            imageProxyUrl: "https://imgproxy.iris.to",
            imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
            imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
            mutedChatIds: [],
            pinnedChatIds: [],
            mobilePushServerUrl: ""
        ),
        toast: toast
    )
}

@discardableResult
private func ffiVoid(
    _ label: String,
    _ body: (UnsafeMutablePointer<RustCallStatus>) throws -> Void
) -> Bool {
    do {
        var status = makeRustCallStatus()
        try body(&status)
        try checkRustCallStatus(status)
        return true
    } catch {
        logSafeFfiFailure(label, error)
        return false
    }
}

private func lowerString(_ value: String) throws -> RustBuffer {
    try rustBuffer(from: Array(value.utf8))
}

private func lowerOptionalString(_ value: String?) throws -> RustBuffer {
    var bytes: [UInt8] = []
    if let value = value {
        let payload = Array(value.utf8)
        guard payload.count <= Int(Int32.max) else {
            throw SafeFfiDispatchError.callError("String too large")
        }
        bytes.append(1)
        appendInt32(Int32(payload.count), to: &bytes)
        bytes.append(contentsOf: payload)
    } else {
        bytes.append(0)
    }
    return try rustBuffer(from: bytes)
}

private func lowerData(_ value: Data) throws -> RustBuffer {
    guard value.count <= Int(Int32.max) else {
        throw SafeFfiDispatchError.callError("Data too large")
    }
    var bytes: [UInt8] = []
    appendInt32(Int32(value.count), to: &bytes)
    bytes.append(contentsOf: value)
    return try rustBuffer(from: bytes)
}

private func rustBuffer(from bytes: [UInt8]) throws -> RustBuffer {
    let copy = bytes
    return try copy.withUnsafeBufferPointer { pointer in
        var status = makeRustCallStatus()
        let buffer = ffi_iris_chat_core_rustbuffer_from_bytes(
            ForeignBytes(len: Int32(pointer.count), data: pointer.baseAddress),
            &status
        )
        try checkRustCallStatus(status)
        return buffer
    }
}

private func liftString(_ buffer: RustBuffer) throws -> String {
    defer { freeRustBuffer(buffer) }
    guard let data = buffer.data, buffer.len > 0 else {
        return ""
    }
    let bytes = UnsafeBufferPointer<UInt8>(start: data, count: Int(buffer.len))
    return String(bytes: bytes, encoding: .utf8) ?? ""
}

private func liftData(_ buffer: RustBuffer) throws -> Data {
    defer { freeRustBuffer(buffer) }
    guard let data = buffer.data, buffer.len > 0 else {
        return Data()
    }
    let bytes = Array(UnsafeBufferPointer<UInt8>(start: data, count: Int(buffer.len)))
    guard bytes.count >= 4 else {
        throw SafeFfiDispatchError.callError("Invalid data buffer")
    }
    let count = readInt32(from: bytes, offset: 0)
    guard count >= 0 else {
        throw SafeFfiDispatchError.callError("Invalid data length")
    }
    let end = 4 + Int(count)
    guard bytes.count >= end else {
        throw SafeFfiDispatchError.callError("Short data buffer")
    }
    return Data(bytes[4..<end])
}

private func logSafeFfiFailure(_ label: String, _ error: Error) {
    NSLog("%@", "Iris Chat FFI call failed (\(label)): \(error)")
}

private func appendInt32(_ value: Int32, to bytes: inout [UInt8]) {
    let raw = UInt32(bitPattern: value)
    bytes.append(UInt8((raw >> 24) & 0xff))
    bytes.append(UInt8((raw >> 16) & 0xff))
    bytes.append(UInt8((raw >> 8) & 0xff))
    bytes.append(UInt8(raw & 0xff))
}

private func readInt32(from bytes: [UInt8], offset: Int) -> Int32 {
    let raw = UInt32(bytes[offset]) << 24
        | UInt32(bytes[offset + 1]) << 16
        | UInt32(bytes[offset + 2]) << 8
        | UInt32(bytes[offset + 3])
    return Int32(bitPattern: raw)
}
