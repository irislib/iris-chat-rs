import UIKit
import SwiftUI

@main
struct ShareHarnessApp: App {
    var body: some Scene {
        WindowGroup { ShareHarnessRoot().ignoresSafeArea() }
    }
}

private struct ShareHarnessRoot: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> ShareHarnessHost { ShareHarnessHost() }
    func updateUIViewController(_ controller: ShareHarnessHost, context: Context) {}
}

final class ShareHarnessHost: UIViewController {
    private let status = UILabel()
    private let names = ["ada": "Ada Lovelace", "grace": "Grace Hopper", "friends": "Hiking group"]
    private lazy var directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("share-fixture", isDirectory: true)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let title = UILabel()
        title.text = "Iris share test"
        title.font = .preferredFont(forTextStyle: .title1)
        let share = UIButton(type: .system)
        share.setTitle("Share link", for: .normal)
        share.addTarget(self, action: #selector(openShare), for: .touchUpInside)
        status.text = "Synthetic recipients. No account is loaded."
        status.accessibilityIdentifier = "fixture-status"
        status.numberOfLines = 0
        let stack = UIStackView(arrangedSubviews: [title, share, status])
        stack.axis = .vertical
        stack.spacing = 24
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    @objc private func openShare() {
        do {
            // Only this test app's private fixture directory is replaced.
            if FileManager.default.fileExists(atPath: directory.path) { try FileManager.default.removeItem(at: directory) }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let entries = [
                ShareSuggestionEntry(chatId: "ada", displayName: names["ada"]!, subtitle: nil, pictureUrl: "fixture://ada", isGroup: false, lastMessageAtSecs: 3, avatarData: thumbnail(.systemPurple, symbol: "leaf.fill")),
                ShareSuggestionEntry(chatId: "grace", displayName: names["grace"]!, subtitle: nil, pictureUrl: "fixture://grace", isGroup: false, lastMessageAtSecs: 2, avatarData: thumbnail(.systemTeal, symbol: "sun.max.fill")),
                ShareSuggestionEntry(chatId: "friends", displayName: names["friends"]!, subtitle: nil, pictureUrl: nil, isGroup: true, lastMessageAtSecs: 1),
            ]
            try JSONEncoder().encode(entries).write(to: directory.appendingPathComponent("share-suggestions.json"), options: .atomic)
            let item = NSExtensionItem()
            item.attachments = [NSItemProvider(object: URL(string: "https://example.com/iris-share-test")! as NSURL)]
            let sheet = ShareViewController(fixtureContainer: directory, items: [item]) { [weak self] in
                guard let self else { return }
                self.dismiss(animated: true)
                self.readQueuedResult()
            }
            sheet.modalPresentationStyle = .pageSheet
            present(sheet, animated: true)
        } catch {
            status.text = "Fixture failed: \(error.localizedDescription)"
        }
    }

    private func thumbnail(_ color: UIColor, symbol: String) -> Data? {
        UIGraphicsImageRenderer(size: CGSize(width: 120, height: 120)).image { context in
            color.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 120, height: 120))
            UIImage(systemName: symbol)?.withTintColor(.white, renderingMode: .alwaysOriginal)
                .draw(in: CGRect(x: 24, y: 24, width: 72, height: 72))
        }.pngData()
    }

    private func readQueuedResult() {
        let inbox = directory.appendingPathComponent("pending-shares")
        guard let urls = try? FileManager.default.contentsOfDirectory(at: inbox, includingPropertiesForKeys: nil),
              let url = urls.first(where: { $0.pathExtension == "json" }),
              let data = try? Data(contentsOf: url),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            status.text = "Share cancelled"
            return
        }
        let recipients = payload["suggestedChatIds"] as? [String] ?? []
        let text = payload["text"] as? String ?? ""
        let autoSend = payload["autoSend"] as? Bool ?? false
        let attachments = payload["attachments"] as? [[String: Any]] ?? []
        status.text = "Queued: \(recipients.compactMap { names[$0] }.joined(separator: ", "))\n\(text)\nAttachments: \(attachments.count)\nSend requested: \(autoSend)"
    }
}
