import Foundation

final class ResultBox: @unchecked Sendable {
    private let lock = NSLock()
    private var value: [String: Any] = [:]
    func set(_ value: [String: Any]) {
        lock.lock()
        defer { lock.unlock() }
        self.value = value
    }
    func get() -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

// Exercise the same system networking family used by stapler, without signing,
// submitting, changing trust settings, or logging signing identifiers.
let body = FileHandle.standardInput.readDataToEndOfFile()
let endpoint = URL(string: "https://api.apple-cloudkit.com/database/1/com.apple.gk.ticket-delivery/production/public/records/lookup")!
var request = URLRequest(url: endpoint)
request.httpMethod = "POST"
request.httpBody = body
request.setValue("application/json", forHTTPHeaderField: "Content-Type")
request.timeoutInterval = 40
let semaphore = DispatchSemaphore(value: 0)
let started = Date()
let box = ResultBox()
let task = URLSession.shared.dataTask(with: request) { data, response, error in
    var result: [String: Any] = [:]
    result["seconds"] = Date().timeIntervalSince(started)
    if let response = response as? HTTPURLResponse {
        result["httpStatus"] = response.statusCode
    }
    if let error = error as NSError? {
        result["errorDomain"] = error.domain
        result["errorCode"] = error.code
    }
    if let data,
       let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
       let records = object["records"] as? [[String: Any]] {
        result["recordCount"] = records.count
        result["signedTicketAvailable"] = records.contains { record in
            guard let fields = record["fields"] as? [String: Any],
                  let ticket = fields["signedTicket"] as? [String: Any],
                  let value = ticket["value"] as? String else { return false }
            return !value.isEmpty
        }
        result["serverErrorCodes"] = records.compactMap { $0["serverErrorCode"] as? String }
    }
    box.set(result)
    semaphore.signal()
}
task.resume()
let result: [String: Any]
if semaphore.wait(timeout: .now() + 50) == .timedOut {
    task.cancel()
    result = ["processDeadlineExceeded": true]
} else {
    result = box.get()
}
let output = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
print(String(decoding: output, as: UTF8.self))
