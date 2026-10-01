import Foundation
import CoreTransferable
import UniformTypeIdentifiers
import SwiftUI

// A staged direct file must never switch to an upload when adding more files.
func irisDirectFileSendMode(current: Bool, hasFiles: Bool, selectedDirectly: Bool) -> Bool {
    selectedDirectly || (current && hasFiles)
}

func irisAttachmentSendAction(
    chatId: String, attachments: [StagedAttachment], caption: String, sendDirectly: Bool
) -> AppAction {
    let files = attachments.map { OutgoingAttachment(filePath: $0.path, filename: $0.filename) }
    if sendDirectly {
        return .sendDirectFiles(chatId: chatId, attachments: files, caption: caption)
    }
    return .sendAttachments(chatId: chatId, attachments: files, caption: caption)
}

extension DirectFileTransferSnapshot {
    var canAcceptOnThisDevice: Bool { status == .offered && !isSender }
    var canCancelOnThisDevice: Bool {
        (status == .offered && isSender) || status == .connecting || status == .transferring
    }

    var displayStatus: String {
        switch status {
        case .offered: return isSender ? "Waiting for acceptance" : "Ready to receive"
        case .connecting: return "Connecting…"
        case .transferring: return isSender ? "Sending…" : "Receiving…"
        case .completed: return isSender ? "Sent" : "Received"
        case .declined: return "Declined"
        case .cancelled: return "Cancelled"
        case .failed: return "Transfer failed"
        case .unavailable: return "Files unavailable"
        }
    }
}

struct ChatDirectFileTransferView: View {
    let transfer: DirectFileTransferSnapshot
    let chatId: String
    let dispatch: (AppAction) -> Void
    @State private var openFailed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Direct files", systemImage: "arrow.up.arrow.down")
                .font(.system(.subheadline, design: .rounded, weight: .semibold))
            ForEach(Array(transfer.files.enumerated()), id: \.offset) { index, file in
                HStack(spacing: 8) {
                    if let path = file.localPath, transfer.status == .completed {
                        Button {
                            Task {
                                do {
                                    let url = try await irisDirectFileExportURL(path: path, filename: file.filename)
                                    openFailed = !PlatformDocumentOpener.open(url)
                                } catch { openFailed = true }
                            }
                        } label: { fileLabel(file) }
                        .buttonStyle(.irisPlain)
                        .accessibilityIdentifier("chatDirectTransferOpen-\(transfer.id)-\(index)")
                        ShareLink(item: IrisDirectFileExport(path: path, filename: file.filename), preview: SharePreview(file.filename)) {
                            Image(systemName: "square.and.arrow.up")
                        }
                        .buttonStyle(.irisPlain)
                        .accessibilityLabel("Share \(file.filename)")
                    } else {
                        fileLabel(file)
                    }
                }
            }
            Text(transfer.displayStatus)
                .font(.system(.caption, design: .rounded))
                .opacity(0.78)
            if transfer.status == .connecting || transfer.status == .transferring {
                ProgressView(value: transfer.totalBytes == 0 ? 0 : min(1, Double(transfer.transferredBytes) / Double(transfer.totalBytes)))
                    .progressViewStyle(.linear)
                Text("\(formattedSize(transfer.transferredBytes)) of \(formattedSize(transfer.totalBytes))")
                    .font(.caption2)
                    .opacity(0.78)
            }
            if transfer.canAcceptOnThisDevice {
                HStack(spacing: 18) {
                    Button("Accept") { dispatch(.acceptDirectFiles(chatId: chatId, transferId: transfer.id)) }
                        .accessibilityIdentifier("chatDirectTransferAccept-\(transfer.id)")
                    Button("Decline") { dispatch(.declineDirectFiles(chatId: chatId, transferId: transfer.id)) }
                        .accessibilityIdentifier("chatDirectTransferDecline-\(transfer.id)")
                }
                .buttonStyle(.bordered)
            } else if transfer.canCancelOnThisDevice {
                Button("Cancel") { dispatch(.cancelDirectFiles(chatId: chatId, transferId: transfer.id)) }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("chatDirectTransferCancel-\(transfer.id)")
            }
        }
        .frame(maxWidth: 260, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatDirectTransfer-\(transfer.id)")
        .alert("Couldn’t open file", isPresented: $openFailed) { Button("OK", role: .cancel) {} }
    }

    private func fileLabel(_ file: DirectFileSnapshot) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "doc.fill")
            VStack(alignment: .leading, spacing: 2) {
                Text(file.filename).font(.subheadline).lineLimit(2)
                Text(formattedSize(file.sizeBytes)).font(.caption2).opacity(0.78)
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
    }

    private func formattedSize(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }
}

// Prepare an original-named local copy only when the user opens or shares it.
// Transfer storage prefixes remain private and no upload/cache downloader runs.
func irisDirectFileExportURL(path: String, filename: String) async throws -> URL {
    try await Task.detached(priority: .userInitiated) {
        let source = URL(fileURLWithPath: path)
        let name = URL(fileURLWithPath: filename.replacingOccurrences(of: "\\", with: "/")).lastPathComponent
        guard !name.isEmpty, name != ".", name != ".." else { throw CocoaError(.fileReadInvalidFileName) }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("iris-direct-file-exports", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let output = directory.appendingPathComponent(name)
        do {
            try FileManager.default.copyItem(at: source, to: output)
            return output
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }.value
}

private struct IrisDirectFileExport: Transferable {
    let path: String
    let filename: String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .data) { file in
            SentTransferredFile(try await irisDirectFileExportURL(path: file.path, filename: file.filename))
        }
    }
}
