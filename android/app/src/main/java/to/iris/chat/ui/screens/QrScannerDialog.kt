package to.iris.chat.ui.screens

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

internal object QrScannerTestOverrides {
    @Volatile
    var nextScannedValue: String? = null

    fun consume(): String? {
        val value = nextScannedValue
        nextScannedValue = null
        return value
    }
}

@Composable
fun QrScannerDialog(
    onDismiss: () -> Unit,
    onScanned: (String) -> String?,
) {
    val injectedScan = remember { QrScannerTestOverrides.consume() }
    LaunchedEffect(injectedScan) {
        if (!injectedScan.isNullOrBlank()) {
            onScanned(injectedScan.trim())
        }
    }
    if (!injectedScan.isNullOrBlank()) {
        return
    }

    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    var error by remember { mutableStateOf<String?>(null) }
    var hasPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }

    val permissionLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            hasPermission = granted
            if (!granted) {
                error = "Camera permission is required to scan QR codes."
            }
        }

    LaunchedEffect(Unit) {
        if (!hasPermission) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    if (!hasPermission) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("Scan QR") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = error ?: "Waiting for camera permission…",
                        color = MaterialTheme.colorScheme.error,
                    )
                    Text(
                        text = "You can still paste a user ID manually.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = { permissionLauncher.launch(Manifest.permission.CAMERA) }) {
                    Text("Retry")
                }
            },
            dismissButton = {
                TextButton(onClick = onDismiss) {
                    Text("Close")
                }
            },
        )
        return
    }

    val previewView =
        remember {
            PreviewView(context).apply {
                scaleType = PreviewView.ScaleType.FILL_CENTER
            }
        }
    val scanner =
        remember {
            val options =
                BarcodeScannerOptions.Builder()
                    .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                    .build()
            BarcodeScanning.getClient(options)
        }
    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    val didEmit = remember { AtomicBoolean(false) }
    val inFlight = remember { AtomicBoolean(false) }
    var cameraProvider: ProcessCameraProvider? by remember { mutableStateOf(null) }

    DisposableEffect(Unit) {
        onDispose {
            runCatching { cameraProvider?.unbindAll() }
            runCatching { analysisExecutor.shutdown() }
        }
    }

    LaunchedEffect(Unit) {
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener(
            { cameraProvider = future.get() },
            ContextCompat.getMainExecutor(context),
        )
    }

    LaunchedEffect(cameraProvider) {
        val provider = cameraProvider ?: return@LaunchedEffect
        error = null
        didEmit.set(false)
        inFlight.set(false)

        val preview = Preview.Builder().build()
        preview.setSurfaceProvider(previewView.surfaceProvider)

        val analysis =
            ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()

        analysis.setAnalyzer(analysisExecutor) { imageProxy ->
            val mediaImage = imageProxy.image
            if (mediaImage == null) {
                imageProxy.close()
                return@setAnalyzer
            }
            if (!inFlight.compareAndSet(false, true)) {
                imageProxy.close()
                return@setAnalyzer
            }

            val image = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
            scanner.process(image)
                .addOnSuccessListener { barcodes ->
                    val raw = barcodes.firstOrNull()?.rawValue?.trim().orEmpty()
                    if (raw.isBlank()) {
                        return@addOnSuccessListener
                    }
                    if (!didEmit.compareAndSet(false, true)) {
                        return@addOnSuccessListener
                    }

                    val errorMessage = onScanned(raw)
                    if (errorMessage != null) {
                        didEmit.set(false)
                        error = errorMessage
                    }
                }.addOnFailureListener {
                    // Keep scanning unless the user dismisses.
                }.addOnCompleteListener {
                    inFlight.set(false)
                    imageProxy.close()
                }
        }

        runCatching { provider.unbindAll() }
        provider.bindToLifecycle(
            lifecycleOwner,
            CameraSelector.DEFAULT_BACK_CAMERA,
            preview,
            analysis,
        )
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Scan QR") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                error?.let {
                    Text(
                        text = it,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                Box(
                    modifier = Modifier.fillMaxWidth(),
                    contentAlignment = Alignment.Center,
                ) {
                    AndroidView(
                        factory = { previewView },
                        modifier = Modifier.size(280.dp),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Close")
            }
        },
    )
}
