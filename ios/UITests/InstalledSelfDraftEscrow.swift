import CryptoKit
import Foundation

/// Private configuration comes from the independently retained native observation.
struct InstalledSelfDraftEscrow: Codable, Equatable {
    enum Refusal: Error { case invalidWitness, differentDraft, recoveryNotOwned }
    let run: UUID
    let accountHash: String
    let candidate: String
    let originalRow: String
    let saved: Data
    let body: String

    static func hash(_ bytes: Data) -> String {
        SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    init(environment e: [String: String], originalRow: String) throws {
        guard let old = e["IRIS_QUALITY_PENDING_RUN_ID"], let oldID = UUID(uuidString: old),
              let fresh = e["IRIS_QUALITY_RUN_ID"], let run = UUID(uuidString: fresh), run != oldID,
              let encoded = e["IRIS_QUALITY_SELF_ESCROW_BASE64"], let saved = Data(base64Encoded: encoded),
              saved == Data(("s quality check " + old + "riI").utf8),
              Self.hash(saved) == e["IRIS_QUALITY_SELF_ESCROW_SHA256"],
              let account = e["IRIS_QUALITY_ACCOUNT_QR_SHA256"], let candidate = e["IRIS_QUALITY_CANDIDATE_BINDING_SHA256"],
              [account, candidate].allSatisfy({ $0.utf8.count == 64 && $0.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) } })
        else { throw Refusal.invalidWitness }
        self.run = run
        self.saved = saved
        self.accountHash = account
        self.candidate = candidate
        self.originalRow = originalRow
        self.body = "Iris quality check " + fresh
    }

    func checkAccount(_ payloads: [String]) throws {
        guard payloads.count == 1, Self.hash(Data(payloads[0].utf8)) == accountHash else { throw Refusal.invalidWitness }
    }

    func checkSaved(_ value: String?) throws {
        guard let value, Data(value.utf8) == saved else { throw Refusal.differentDraft }
    }

    // The runner's ledger survives a failed test. Never reuse a send authorization.
    func file(_ phase: String, in directory: URL) -> URL {
        directory.appendingPathComponent(run.uuidString + "." + phase)
    }

    func record(_ phase: String, in directory: URL) throws {
        let destination = file(phase, in: directory)
        try JSONEncoder().encode(self).write(to: destination, options: .withoutOverwriting)
        let handle = try FileHandle(forWritingTo: destination)
        defer { try? handle.close() }
        try handle.synchronize()
        try requireRecord(phase, in: directory)
    }

    func requireRecord(_ phase: String, in directory: URL) throws {
        guard try JSONDecoder().decode(Self.self, from: Data(contentsOf: file(phase, in: directory))) == self
        else { throw Refusal.recoveryNotOwned }
    }

    enum Recovery { case alreadyRestored, empty, freshBody }

    func recovery(_ value: String, in directory: URL) throws -> Recovery {
        try requireRecord("escrow", in: directory)
        if Data(value.utf8) == saved { return .alreadyRestored }
        guard !FileManager.default.fileExists(atPath: file("restored", in: directory).path) else { throw Refusal.recoveryNotOwned }
        if value.isEmpty {
            try requireRecord("cleared", in: directory)
            return .empty
        }
        guard Data(value.utf8) == Data(body.utf8) else { throw Refusal.differentDraft }
        try requireRecord("typing", in: directory)
        return .freshBody
    }
}
