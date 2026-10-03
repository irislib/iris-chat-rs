import CoreImage.CIFilterBuiltins
import SwiftUI
#if canImport(UIKit)
import AVFoundation
#endif

enum DeviceApprovalQr {
    static func isValid(_ raw: String) -> Bool {
        isDeviceApprovalBootstrap(raw: raw.trimmingCharacters(in: .whitespacesAndNewlines))
    }
}

struct ResolvedDeviceAuthorizationInput: Equatable {
    let deviceInput: String
    let errorMessage: String?
    let requiresConfirmation: Bool
}

func resolveDeviceAuthorizationInput(rawInput: String) -> ResolvedDeviceAuthorizationInput {
    let trimmed = rawInput.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty {
        return ResolvedDeviceAuthorizationInput(
            deviceInput: "",
            errorMessage: nil,
            requiresConfirmation: false
        )
    }

    if DeviceApprovalQr.isValid(trimmed) || trimmed.lowercased().hasPrefix("nostrconnect://") {
        return ResolvedDeviceAuthorizationInput(
            deviceInput: trimmed,
            errorMessage: nil,
            requiresConfirmation: true
        )
    }

    return ResolvedDeviceAuthorizationInput(
        deviceInput: "",
        errorMessage: "Not a valid link code.",
        requiresConfirmation: false
    )
}

struct QrCodeImage: View {
    let text: String
    let size: CGFloat

    init(text: String, size: CGFloat = 260) {
        self.text = text
        self.size = size
    }

    var body: some View {
        if let image = qrImage(text: text) {
            Image(platformImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .frame(width: size, height: size)
                .background(Color.white)
        } else {
            Color.secondary.opacity(0.1)
                .frame(width: size, height: size)
                .overlay(Text("Code unavailable").font(.footnote))
        }
    }

    private func qrImage(text: String) -> PlatformImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(Data(text.utf8), forKey: "inputMessage")
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else {
            return nil
        }
        let transformed = output.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        let context = CIContext()
        guard let cgImage = context.createCGImage(transformed, from: transformed.extent) else {
            return nil
        }
        #if canImport(UIKit)
        return UIImage(cgImage: cgImage)
        #elseif canImport(AppKit)
        return NSImage(cgImage: cgImage, size: transformed.extent.size)
        #else
        return nil
        #endif
    }
}

#if canImport(UIKit)
struct QrScannerSheet: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let controller = ScannerViewController()
        controller.onCode = onCode
        return controller
    }

    func updateUIViewController(_ uiViewController: ScannerViewController, context: Context) {}
}

final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onCode: ((String) -> Void)?

    private lazy var capture = IrisQrCaptureSession(delegate: self)
    private lazy var lifecycle: IrisQrScannerLifecycle = {
        let capture = self.capture
        return IrisQrScannerLifecycle(
            requestAccess: { await AVCaptureDevice.requestAccess(for: .video) },
            startSession: { capture.start() },
            stopSession: { capture.stop() }
        )
    }()
    private var activationTask: Task<Void, Never>?
    private var deliveredCode = false
    private var previewLayer: AVCaptureVideoPreviewLayer?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        let layer = AVCaptureVideoPreviewLayer(session: capture.session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        previewLayer = layer
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        deliveredCode = false
        if let testValue = ProcessInfo.processInfo.environment["IRIS_QR_TEST_VALUE"], !testValue.isEmpty {
            DispatchQueue.main.async { [weak self] in
                self?.onCode?(testValue)
            }
            return
        }
        let lifecycle = self.lifecycle
        activationTask = Task {
            await lifecycle.activate()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        activationTask?.cancel()
        activationTask = nil
        lifecycle.deactivate()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard lifecycle.isActive, !deliveredCode,
              let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let value = object.stringValue
        else {
            return
        }
        deliveredCode = true
        lifecycle.deactivate()
        onCode?(value)
    }
}

@MainActor
final class IrisQrScannerLifecycle {
    private let queue: DispatchQueue
    private let requestAccess: @MainActor () async -> Bool
    private let startSession: @Sendable () -> Void
    private let stopSession: @Sendable () -> Void
    private var generation: UInt64 = 0
    private(set) var isActive = false

    init(
        queue: DispatchQueue = DispatchQueue(label: "to.iris.chat.qr-session"),
        requestAccess: @escaping @MainActor () async -> Bool,
        startSession: @escaping @Sendable () -> Void,
        stopSession: @escaping @Sendable () -> Void
    ) {
        self.queue = queue
        self.requestAccess = requestAccess
        self.startSession = startSession
        self.stopSession = stopSession
    }

    func activate() async {
        guard !Task.isCancelled else { return }
        generation &+= 1
        let requestGeneration = generation
        isActive = true
        let granted = await requestAccess()
        guard granted, isActive, generation == requestGeneration, !Task.isCancelled else { return }
        queue.async(execute: startSession)
    }

    func deactivate() {
        generation &+= 1
        isActive = false
        queue.async(execute: stopSession)
    }

    deinit {
        queue.async(execute: stopSession)
    }
}

// Configuration and start/stop run only on IrisQrScannerLifecycle's serial queue.
// The preview layer reads the immutable session reference on the main thread.
private final class IrisQrCaptureSession: @unchecked Sendable {
    let session = AVCaptureSession()
    private weak var delegate: AVCaptureMetadataOutputObjectsDelegate?
    private var configured = false

    init(delegate: AVCaptureMetadataOutputObjectsDelegate) {
        self.delegate = delegate
    }

    func start() {
        if !configured {
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device) else { return }
            let output = AVCaptureMetadataOutput()
            guard session.canAddInput(input), session.canAddOutput(output) else { return }
            session.beginConfiguration()
            session.addInput(input)
            session.addOutput(output)
            output.setMetadataObjectsDelegate(delegate, queue: .main)
            output.metadataObjectTypes = [.qr]
            session.commitConfiguration()
            configured = true
        }
        if !session.isRunning { session.startRunning() }
    }

    func stop() {
        if session.isRunning { session.stopRunning() }
    }
}
#else
struct QrScannerSheet: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var pastedCode = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                Text("Scanning is not available on macOS yet.")
                    .font(.system(.title3, design: .rounded, weight: .bold))
                Spacer()
                IrisModalCloseButton {
                    dismiss()
                }
                .accessibilityIdentifier("qrScannerCloseButton")
            }

            Text("Paste the code instead.")
                .font(.system(.body, design: .rounded))
                .foregroundStyle(.secondary)

            TextField("Paste code", text: $pastedCode)
                .textFieldStyle(.roundedBorder)

            HStack(spacing: 10) {
                Button("Paste from clipboard") {
                    pastedCode = (PlatformClipboard.string() ?? "")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                }

                Button("Use code") {
                    onCode(pastedCode)
                    dismiss()
                }
                .disabled(pastedCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(minWidth: 420)
    }
}
#endif
