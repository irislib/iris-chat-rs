import CoreImage.CIFilterBuiltins
import SwiftUI
#if canImport(UIKit)
import AVFoundation
#endif

enum DeviceApprovalQr {
    static func encode(ownerInput: String, deviceInput: String) -> String {
        normalizeDeviceApprovalQr(
            encodeDeviceApprovalQr(
                ownerInput: ownerInput.trimmingCharacters(in: .whitespacesAndNewlines),
                deviceInput: deviceInput.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        )
    }

    static func decode(_ raw: String) -> DeviceApprovalQrPayload? {
        decodeDeviceApprovalQr(raw: raw)
    }
}

private func normalizeDeviceApprovalQr(_ raw: String) -> String {
    raw.trimmingCharacters(in: .whitespacesAndNewlines)
}

struct ResolvedDeviceAuthorizationInput: Equatable {
    let deviceInput: String
    let errorMessage: String?
}

func resolveDeviceAuthorizationInput(
    rawInput: String,
    ownerNpub: String,
    ownerPublicKeyHex: String
) -> ResolvedDeviceAuthorizationInput {
    let trimmed = rawInput.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty {
        return ResolvedDeviceAuthorizationInput(deviceInput: "", errorMessage: nil)
    }

    if let payload = DeviceApprovalQr.decode(trimmed) {
        let normalizedOwner = normalizePeerInput(input: payload.ownerInput)
        let acceptedOwnerInputs = Set([
            normalizePeerInput(input: ownerNpub),
            normalizePeerInput(input: ownerPublicKeyHex),
        ])
        if !acceptedOwnerInputs.contains(normalizedOwner) {
            return ResolvedDeviceAuthorizationInput(
                deviceInput: "",
                errorMessage: "This QR is for a different account."
            )
        }

        let normalizedDevice = normalizePeerInput(input: payload.deviceInput)
        if !isValidPeerInput(input: normalizedDevice) {
            return ResolvedDeviceAuthorizationInput(
                deviceInput: "",
                errorMessage: "That QR does not contain a valid device code."
            )
        }
        return ResolvedDeviceAuthorizationInput(deviceInput: normalizedDevice, errorMessage: nil)
    }

    let normalized = normalizePeerInput(input: trimmed)
    if isValidPeerInput(input: normalized) {
        return ResolvedDeviceAuthorizationInput(deviceInput: normalized, errorMessage: nil)
    }

    return ResolvedDeviceAuthorizationInput(
        deviceInput: "",
        errorMessage: "Not a valid device code."
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
                .overlay(Text("QR unavailable").font(.footnote))
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

    private let session = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        if let testValue = ProcessInfo.processInfo.environment["IRIS_QR_TEST_VALUE"], !testValue.isEmpty {
            DispatchQueue.main.async { [weak self] in
                self?.onCode?(testValue)
            }
            return
        }
        AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
            guard granted else { return }
            DispatchQueue.main.async {
                self?.configureSession()
            }
        }
    }

    private func configureSession() {
        guard previewLayer == nil,
              let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device)
        else {
            return
        }
        if session.canAddInput(input) {
            session.addInput(input)
        }

        let output = AVCaptureMetadataOutput()
        if session.canAddOutput(output) {
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.qr]
        }

        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        previewLayer = layer
        session.startRunning()
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
        guard let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              let value = object.stringValue
        else {
            return
        }
        session.stopRunning()
        onCode?(value)
    }
}
#else
struct QrScannerSheet: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var pastedCode = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("QR scanning is not wired for macOS yet.")
                .font(.system(.title3, design: .rounded, weight: .bold))
            Text("Paste the code instead.")
                .font(.system(.body, design: .rounded))
                .foregroundStyle(.secondary)

            TextField("Paste code", text: $pastedCode)
                .textFieldStyle(.roundedBorder)

            HStack(spacing: 10) {
                Button("Paste from clipboard") {
                    pastedCode = normalizePeerInput(input: PlatformClipboard.string() ?? "")
                }

                Button("Use code") {
                    onCode(pastedCode)
                    dismiss()
                }
                .disabled(pastedCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                Spacer()

                Button("Close") {
                    dismiss()
                }
            }
        }
        .padding(20)
        .frame(minWidth: 420)
    }
}
#endif
