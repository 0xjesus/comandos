package app.comandos.comandos_mobile

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.PluginRegistry

/**
 * The app's own platform channel `comandos/native`: notifications, the keep-alive
 * service, notification taps and the QR scanner. Replaces four plugins
 * (flutter_local_notifications, flutter_foreground_task, mobile_scanner + ML Kit).
 */
class NativeBridge :
    FlutterPlugin,
    ActivityAware,
    MethodChannel.MethodCallHandler,
    PluginRegistry.NewIntentListener,
    PluginRegistry.ActivityResultListener,
    PluginRegistry.RequestPermissionsResultListener {

    private companion object {
        const val REQ_SCAN = 7101
        const val REQ_NOTIF = 7102
    }

    private lateinit var context: Context
    private var channel: MethodChannel? = null
    private var binding: ActivityPluginBinding? = null
    private var pendingTap: Map<String, String>? = null
    private var scanResult: MethodChannel.Result? = null
    private var permResult: MethodChannel.Result? = null

    override fun onAttachedToEngine(b: FlutterPlugin.FlutterPluginBinding) {
        context = b.applicationContext
        channel = MethodChannel(b.binaryMessenger, "comandos/native").also { it.setMethodCallHandler(this) }
    }

    override fun onDetachedFromEngine(b: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    // ---- Activity ----------------------------------------------------------

    override fun onAttachedToActivity(b: ActivityPluginBinding) {
        binding = b
        EngineHolder.activityAttached = true
        b.addOnNewIntentListener(this)
        b.addActivityResultListener(this)
        b.addRequestPermissionsResultListener(this)
        if (takeIntent(b.activity.intent)) channel?.invokeMethod("tap", null)
    }

    override fun onDetachedFromActivity() {
        binding?.removeOnNewIntentListener(this)
        binding?.removeActivityResultListener(this)
        binding?.removeRequestPermissionsResultListener(this)
        binding = null
        EngineHolder.activityAttached = false
        scanResult?.success(mapOf("error" to "cancelled"))
        scanResult = null
        permResult?.success(false)
        permResult = null
    }

    override fun onReattachedToActivityForConfigChanges(b: ActivityPluginBinding) = onAttachedToActivity(b)

    override fun onDetachedFromActivityForConfigChanges() = onDetachedFromActivity()

    override fun onNewIntent(intent: Intent): Boolean {
        if (!takeIntent(intent)) return false
        channel?.invokeMethod("tap", null)
        return true
    }

    /** Keeps the session of a notification tap until Dart asks for it. */
    private fun takeIntent(intent: Intent?): Boolean {
        if (intent?.action != Notifier.ACTION_OPEN_CHAT) return false
        val tap = HashMap<String, String>()
        intent.getStringExtra(Notifier.EXTRA_SESSION)?.let { tap["session"] = it }
        intent.getStringExtra(Notifier.EXTRA_PANE)?.let { tap["pane"] = it }
        pendingTap = tap
        // Handled once: a recreated activity must not reopen the chat.
        intent.action = Intent.ACTION_MAIN
        return true
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?): Boolean {
        if (requestCode != REQ_SCAN) return false
        val r = scanResult ?: return true
        scanResult = null
        val text = data?.getStringExtra(ScanActivity.EXTRA_TEXT)
        if (resultCode == Activity.RESULT_OK && text != null) {
            r.success(mapOf("text" to text))
        } else {
            r.success(mapOf("error" to (data?.getStringExtra(ScanActivity.EXTRA_ERROR) ?: "cancelled")))
        }
        return true
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ): Boolean {
        if (requestCode != REQ_NOTIF) return false
        val r = permResult ?: return true
        permResult = null
        r.success(grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED)
        return true
    }

    // ---- Calls from Dart ---------------------------------------------------

    private fun notificationsAllowed(): Boolean {
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        return Notifier.enabled(context)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "notificationsAllowed" -> result.success(notificationsAllowed())
            "requestNotifications" -> {
                val activity = binding?.activity
                when {
                    notificationsAllowed() -> result.success(true)
                    Build.VERSION.SDK_INT < 33 || activity == null -> result.success(false)
                    permResult != null -> result.error("busy", "permission request in progress", null)
                    else -> {
                        permResult = result
                        activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQ_NOTIF)
                    }
                }
            }
            "watchStart" -> result.success(WatchService.start(context))
            "watchStop" -> {
                WatchService.stop(context)
                result.success(null)
            }
            "watchRunning" -> result.success(WatchService.isRunning)
            "watchTakeLost" -> result.success(WatchService.takeLost(context))
            "notify" -> {
                Notifier.show(
                    context,
                    tag = call.argument<String>("tag") ?: "comandos",
                    urgent = call.argument<Boolean>("urgent") ?: false,
                    title = call.argument<String>("title") ?: "ComandOS",
                    body = call.argument<String>("body") ?: "",
                    session = call.argument<String>("session"),
                    pane = call.argument<String>("pane"),
                    whenMs = (call.argument<Number>("when") ?: 0).toLong(),
                )
                result.success(null)
            }
            "cancel" -> {
                call.argument<String>("tag")?.let { Notifier.cancel(context, it) }
                result.success(null)
            }
            "cancelAll" -> {
                Notifier.cancelAll(context)
                result.success(null)
            }
            "takeTap" -> {
                result.success(pendingTap)
                pendingTap = null
            }
            "scanQr" -> {
                val activity = binding?.activity
                when {
                    activity == null -> result.success(mapOf("error" to "failed"))
                    scanResult != null -> result.success(mapOf("error" to "busy"))
                    else -> {
                        scanResult = result
                        activity.startActivityForResult(Intent(activity, ScanActivity::class.java), REQ_SCAN)
                    }
                }
            }
            "openSettings" -> {
                val notif = call.argument<Boolean>("notifications") ?: false
                val intent = if (notif) {
                    Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                } else {
                    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                }
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                try {
                    context.startActivity(intent)
                    result.success(true)
                } catch (_: Exception) {
                    result.success(false)
                }
            }
            else -> result.notImplemented()
        }
    }
}
