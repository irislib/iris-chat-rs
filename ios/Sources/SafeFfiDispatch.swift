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
    func dispatchSafely(action: AppAction) throws {
        uniffiEnsureIrisChatCoreInitialized()

        let pointer = self.uniffiClonePointer()
        let loweredAction = FfiConverterTypeAppAction_lower(action)
        var callStatus = makeRustCallStatus()
        uniffi_iris_chat_core_fn_method_ffiapp_dispatch(pointer, loweredAction, &callStatus)
        try checkRustCallStatus(callStatus)
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
