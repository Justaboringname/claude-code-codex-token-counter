// In-app QR scanning: the Mac's QR code is a web-app link with `#pair=…`,
// which the system Camera would open in Safari, not here.
import SwiftUI
import VisionKit

struct QRScanner: UIViewControllerRepresentable {
    var onCode: (String) -> Void

    static var isAvailable: Bool {
        DataScannerViewController.isSupported && DataScannerViewController.isAvailable
    }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let vc = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isHighlightingEnabled: true
        )
        vc.delegate = context.coordinator
        try? vc.startScanning()
        return vc
    }

    func updateUIViewController(_ vc: DataScannerViewController, context: Context) {}

    static func dismantleUIViewController(_ vc: DataScannerViewController, coordinator: Coordinator) {
        vc.stopScanning()
    }

    func makeCoordinator() -> Coordinator { Coordinator(onCode: onCode) }

    @MainActor final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onCode: (String) -> Void
        private var done = false
        init(onCode: @escaping (String) -> Void) { self.onCode = onCode }

        func dataScanner(_ s: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]) {
            for item in items {
                guard !done, case let .barcode(b) = item, let text = b.payloadStringValue else { continue }
                let code = SyncProtocol.extractCode(text)
                guard code.hasPrefix(SyncProtocol.codePrefix) else { continue }
                done = true
                s.stopScanning()
                UINotificationFeedbackGenerator().notificationOccurred(.success)
                onCode(code)
            }
        }
    }
}
