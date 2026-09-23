package to.iris.chat.calls

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureFailure
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.TotalCaptureResult
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import android.view.WindowManager
import java.io.ByteArrayOutputStream

/** Small independent JPEG frames keep the baseline interoperable with desktop. */
internal class CallCamera(
    private val context: Context,
    private val send: (ByteArray) -> Unit,
    private val preview: (Bitmap) -> Unit,
    private val failed: () -> Unit,
) : AutoCloseable {
    private val thread = HandlerThread("Iris call camera").apply { start() }
    private val handler = Handler(thread.looper)
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    @Volatile private var closed = false

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    fun start() {
        val manager = context.getSystemService(CameraManager::class.java)
        val id = manager.cameraIdList.firstOrNull {
            manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        } ?: manager.cameraIdList.first()
        val characteristics = manager.getCameraCharacteristics(id)
        val size = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
            .getOutputSizes(ImageFormat.JPEG).minBy { it.width * it.height }
        val displayDegrees = when (context.getSystemService(WindowManager::class.java).defaultDisplay.rotation) {
            Surface.ROTATION_90 -> 90
            Surface.ROTATION_180 -> 180
            Surface.ROTATION_270 -> 270
            else -> 0
        }
        val front = characteristics.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        val rotation = ((characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0) +
            (if (front) displayDegrees else -displayDegrees) + 360) % 360
        val images = ImageReader.newInstance(size.width, size.height, ImageFormat.JPEG, 2)
        reader = images
        images.setOnImageAvailableListener({ source ->
            val image = source.acquireLatestImage() ?: return@setOnImageAvailableListener
            try {
                if (!closed) {
                    val bytes = ByteArray(image.planes[0].buffer.remaining())
                    image.planes[0].buffer.get(bytes)
                    val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                    if (decoded != null) {
                        val rotated = Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height,
                            Matrix().apply { postRotate(rotation.toFloat()) }, true)
                        val scale = minOf(320f / rotated.width, 240f / rotated.height, 1f)
                        val frame = Bitmap.createScaledBitmap(rotated, (rotated.width * scale).toInt(),
                            (rotated.height * scale).toInt(), true)
                        val encoded = ByteArrayOutputStream().use { out ->
                            frame.compress(Bitmap.CompressFormat.JPEG, 55, out)
                            out.toByteArray()
                        }
                        if (encoded.size <= 65_536) { send(encoded); preview(frame) }
                        if (decoded !== rotated && decoded !== frame) decoded.recycle()
                        if (rotated !== frame) rotated.recycle()
                    }
                }
            } catch (_: Exception) {
                if (!closed) failed()
            } finally { image.close() }
        }, handler)
        manager.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) {
                if (closed) { camera.close(); return }
                device = camera
                camera.createCaptureSession(listOf(images.surface), object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(captureSession: CameraCaptureSession) {
                        if (closed) { captureSession.close(); return }
                        session = captureSession
                        val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE).apply {
                            addTarget(images.surface)
                        }.build()
                        val capture = object : Runnable {
                            override fun run() {
                                if (closed) return
                                val next = this
                                try { captureSession.capture(request, object : CameraCaptureSession.CaptureCallback() {
                                    override fun onCaptureCompleted(session: CameraCaptureSession, request: CaptureRequest, result: TotalCaptureResult) {
                                        if (!closed) handler.postDelayed(next, 150L)
                                    }
                                    override fun onCaptureFailed(session: CameraCaptureSession, request: CaptureRequest, failure: CaptureFailure) {
                                        if (!closed) failed()
                                    }
                                }, handler) }
                                catch (_: Exception) { if (!closed) failed(); return }
                            }
                        }
                        handler.post(capture)
                    }
                    override fun onConfigureFailed(session: CameraCaptureSession) { if (!closed) failed() }
                }, handler)
            }
            override fun onDisconnected(camera: CameraDevice) { camera.close(); if (!closed) failed() }
            override fun onError(camera: CameraDevice, error: Int) { camera.close(); if (!closed) failed() }
        }, handler)
    }

    override fun close() {
        closed = true
        handler.removeCallbacksAndMessages(null)
        session?.close()
        device?.close()
        reader?.close()
        thread.quitSafely()
    }
}
