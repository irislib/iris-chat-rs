import Foundation

// The transfer worker retains this lease through commit/abort, even if the chat closes.
final class IrisDirectFileDestination: DirectFileDestination, @unchecked Sendable {
    private let url: URL
    private let accessed: Bool
    private let output: DirectFileDestination
    private var savedLocations: [String] = []

    init(folder: URL) {
        url = folder
        accessed = folder.startAccessingSecurityScopedResource()
        output = directFileDirectoryDestination(directory: folder.path)
    }
    deinit { if accessed { url.stopAccessingSecurityScopedResource() } }
    func prepare(transferId: String, files: [DirectFileSnapshot]) throws {
        // Persist the folder capability privately with each received file, not in events.
        // Resolve it lazily on Open/Share, avoiding permission or file I/O at startup.
        #if os(macOS)
        let options: URL.BookmarkCreationOptions = [.withSecurityScope]
        #else
        let options: URL.BookmarkCreationOptions = [.minimalBookmark]
        #endif
        let bookmark = try url.bookmarkData(options: options, includingResourceValuesForKeys: nil, relativeTo: nil)
        let locations = try files.enumerated().map { index, file in
            try IrisDirectFileLocation(bookmark: bookmark, relativePath: "Iris files \(transferId)/\(index + 1)-\(file.filename)").encoded()
        }
        try output.prepare(transferId: transferId, files: files)
        savedLocations = locations
    }
    func write(fileIndex: UInt32, bytes: Data) throws { try output.write(fileIndex: fileIndex, bytes: bytes) }
    func finishFile(fileIndex: UInt32) throws { try output.finishFile(fileIndex: fileIndex) }
    func commit() throws -> [String] {
        _ = try output.commit()
        return savedLocations
    }
    func abort() { output.abort() }
}

// Opaque local URI: the core stores it only in this device's private transfer record.
struct IrisDirectFileLocation: Codable {
    static let prefix = "iris-file-bookmark:"
    let bookmark: Data
    let relativePath: String

    func encoded() throws -> String { Self.prefix + (try JSONEncoder().encode(self)).base64EncodedString() }

    static func resolve(_ path: String) throws -> (file: URL, scope: URL?) {
        guard path.hasPrefix(prefix) else { return (URL(fileURLWithPath: path), nil) }
        guard let data = Data(base64Encoded: String(path.dropFirst(prefix.count))) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        let location = try JSONDecoder().decode(Self.self, from: data)
        let components = location.relativePath.split(separator: "/", omittingEmptySubsequences: false)
        guard components.count == 2, components.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }) else {
            throw CocoaError(.fileReadInvalidFileName)
        }
        var stale = false
        #if os(macOS)
        let options: URL.BookmarkResolutionOptions = [.withSecurityScope, .withoutUI]
        #else
        let options: URL.BookmarkResolutionOptions = [.withoutUI]
        #endif
        let folder = try URL(resolvingBookmarkData: location.bookmark, options: options, relativeTo: nil, bookmarkDataIsStale: &stale)
        return (folder.appendingPathComponent(location.relativePath), folder)
    }
}
