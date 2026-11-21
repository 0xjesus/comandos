@file:Suppress("DEPRECATION") // android.hardware.Camera: no extra library, works offline everywhere.

package app.comandos.comandos_mobile

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.hardware.Camera
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.TypedValue
import android.view.Gravity
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.TextView
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.InvertedLuminanceSource
import com.google.zxing.LuminanceSource
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import kotlin.math.abs
import kotlin.math.min

/**
 * Full-screen QR scanner: camera preview + ZXing's QR reader on a background
 * thread. No Play Services, no model download, works offline. Returns the
 * text in [EXTRA_TEXT], or [EXTRA_ERROR] = "denied" | "failed" | "cancelled".
 */
class ScanActivity : Activity(), SurfaceHolder.Callback, Camera.PreviewCallback {
    companion object {
        const val EXTRA_TEXT = "text"
        const val EXTRA_ERROR = "error"
        private const val REQ_CAMERA = 1
    }

    private lateinit var root: FrameLayout
    private lateinit var surface: SurfaceView
    private var hasSurface = false
    private var camera: Camera? = null
    private var previewW = 0
    private var previewH = 0
    private var worker: HandlerThread? = null
    private var workerHandler: Handler? = null
    @Volatile private var decoding = false
    @Volatile private var done = false
    private var frame = 0
    private val reader = QRCodeReader()
    private val hints = mapOf(DecodeHintType.CHARACTER_SET to "UTF-8")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
        surface = SurfaceView(this)
        root.addView(surface, FrameLayout.LayoutParams(-1, -1, Gravity.CENTER))
        root.addView(Frame(this), FrameLayout.LayoutParams(-1, -1))
        val hint = TextView(this).apply {
            text = getString(R.string.scan_hint)
            setTextColor(0xFFEAF0FB.toInt())
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
            gravity = Gravity.CENTER
            val pad = (24 * resources.displayMetrics.density).toInt()
            setPadding(pad, pad, pad, pad * 2)
        }
        root.addView(hint, FrameLayout.LayoutParams(-1, -2, Gravity.BOTTOM))
        setContentView(root)
        surface.holder.addCallback(this)
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.CAMERA), REQ_CAMERA)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        if (requestCode != REQ_CAMERA) return
        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            if (hasSurface) openCamera()
        } else {
            finishWith(null, "denied")
        }
    }

    override fun onResume() {
        super.onResume()
        worker = HandlerThread("qr").also { it.start(); workerHandler = Handler(it.looper) }
        if (hasSurface) openCamera()
    }

    override fun onPause() {
        closeCamera()
        worker?.quitSafely()
        worker = null
        workerHandler = null
        super.onPause()
    }

    override fun onBackPressed() {
        finishWith(null, "cancelled")
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        hasSurface = true
        openCamera()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        hasSurface = false
        closeCamera()
    }

    private fun openCamera() {
        if (camera != null || done || workerHandler == null) return
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) return
        try {
            val id = backCameraId()
            val c = Camera.open(id)
            val info = Camera.CameraInfo().also { Camera.getCameraInfo(id, it) }
            val p = c.parameters
            val size = p.supportedPreviewSizes.minByOrNull { abs(it.width * it.height - 1280 * 720) }
            if (size != null) p.setPreviewSize(size.width, size.height)
            when {
                Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE in p.supportedFocusModes ->
                    p.focusMode = Camera.Parameters.FOCUS_MODE_CONTINUOUS_PICTURE
                Camera.Parameters.FOCUS_MODE_AUTO in p.supportedFocusModes ->
                    p.focusMode = Camera.Parameters.FOCUS_MODE_AUTO
            }
            c.parameters = p
            // The activity is portrait-only.
            c.setDisplayOrientation(info.orientation % 360)
            c.setPreviewDisplay(surface.holder)
            previewW = c.parameters.previewSize.width
            previewH = c.parameters.previewSize.height
            val buf = previewW * previewH * 3 / 2
            c.addCallbackBuffer(ByteArray(buf))
            c.addCallbackBuffer(ByteArray(buf))
            c.setPreviewCallbackWithBuffer(this)
            c.startPreview()
            camera = c
            fitPreview()
        } catch (e: Exception) {
            closeCamera()
            finishWith(null, "failed")
        }
    }

    /** Center-crops the preview so it is not stretched (the frame is landscape, the screen portrait). */
    private fun fitPreview() {
        root.post {
            val vw = root.width
            val vh = root.height
            if (vw == 0 || vh == 0 || previewW == 0) return@post
            val pw = previewH.toFloat()
            val ph = previewW.toFloat()
            val scale = maxOf(vw / pw, vh / ph)
            val lp = surface.layoutParams as FrameLayout.LayoutParams
            val w = (pw * scale).toInt()
            val h = (ph * scale).toInt()
            if (lp.width != w || lp.height != h) {
                lp.width = w
                lp.height = h
                surface.layoutParams = lp
            }
        }
    }

    private fun backCameraId(): Int {
        val info = Camera.CameraInfo()
        for (i in 0 until Camera.getNumberOfCameras()) {
            Camera.getCameraInfo(i, info)
            if (info.facing == Camera.CameraInfo.CAMERA_FACING_BACK) return i
        }
        return 0
    }

    private fun closeCamera() {
        val c = camera ?: return
        camera = null
        try {
            c.setPreviewCallbackWithBuffer(null)
            c.stopPreview()
        } catch (_: Exception) {
        }
        c.release()
    }

    override fun onPreviewFrame(data: ByteArray, cam: Camera) {
        val h = workerHandler
        if (done || decoding || h == null) {
            cam.addCallbackBuffer(data)
            return
        }
        decoding = true
        val w = previewW
        val ht = previewH
        val invert = (frame++ % 2) == 1
        h.post {
            val text = decode(data, w, ht, invert)
            decoding = false
            if (text != null) {
                runOnUiThread { finishWith(text, null) }
            } else {
                // The camera may have been released meanwhile; then the buffer is dropped.
                runOnUiThread { camera?.addCallbackBuffer(data) }
            }
        }
    }

    /** Decodes the central square; every other frame inverted (light-on-dark QR codes). */
    private fun decode(data: ByteArray, w: Int, h: Int, invert: Boolean): String? {
        val side = min(w, h) * 4 / 5
        return try {
            var src: LuminanceSource = PlanarYUVLuminanceSource(data, w, h, (w - side) / 2, (h - side) / 2, side, side, false)
            if (invert) src = InvertedLuminanceSource(src)
            reader.decode(BinaryBitmap(HybridBinarizer(src)), hints).text
        } catch (_: ReaderException) {
            null
        } catch (_: RuntimeException) {
            null
        } finally {
            reader.reset()
        }
    }

    private fun finishWith(text: String?, error: String?) {
        if (done) return
        done = true
        val data = Intent()
        if (text != null) data.putExtra(EXTRA_TEXT, text) else data.putExtra(EXTRA_ERROR, error)
        setResult(if (text != null) RESULT_OK else RESULT_CANCELED, data)
        finish()
    }

    /** Dims everything but the central square the decoder reads. */
    private class Frame(ctx: android.content.Context) : View(ctx) {
        private val dim = android.graphics.Paint().apply { color = 0x99000000.toInt() }
        private val edge = android.graphics.Paint().apply {
            color = 0xFF8B7CFF.toInt()
            style = android.graphics.Paint.Style.STROKE
            strokeWidth = 3 * ctx.resources.displayMetrics.density
        }

        override fun onDraw(canvas: android.graphics.Canvas) {
            val side = min(width, height) * 4 / 5f
            val l = (width - side) / 2
            val t = (height - side) / 2
            canvas.drawRect(0f, 0f, width.toFloat(), t, dim)
            canvas.drawRect(0f, t + side, width.toFloat(), height.toFloat(), dim)
            canvas.drawRect(0f, t, l, t + side, dim)
            canvas.drawRect(l + side, t, width.toFloat(), t + side, dim)
            canvas.drawRect(l, t, l + side, t + side, edge)
        }
    }
}
