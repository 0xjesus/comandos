package app.comandos.comandos_mobile

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import org.json.JSONArray
import org.json.JSONObject
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

    companion object {
        private const val REQ_SCAN = 7101
        private const val REQ_NOTIF = 7102
        private const val REQ_BLUETOOTH = 7103
        private const val PREFS = "comandos_car"
        private const val KEY_QUEUE = "replies"

        /** A car reply must be answered before the broadcast window closes. */
        private const val REPLY_MS = 9000L

        /** The bridge of the running engine (null while no Dart runs). */
        @Volatile
        private var live: NativeBridge? = null

        /**
         * Hands a car reply to Dart and calls [done] once with the outcome to
         * show. Without a running engine the reply waits on disk and Dart
         * (started now) handles it as soon as it boots.
         */
        fun carReply(context: Context, args: Map<String, String?>, done: (String) -> Unit) {
            val ch = live?.channel
            if (ch == null) {
                queue(context, args)
                try {
                    EngineHolder.ensureHeadless(context)
                } catch (_: Exception) {
                }
                done("Guardado; lo proceso en cuanto ComandOS arranque.")
                return
            }
            var finished = false
            val main = Handler(Looper.getMainLooper())
            fun finish(outcome: String) {
                if (finished) return
                finished = true
                done(outcome)
            }
            main.postDelayed({ finish("Sigue en proceso; revisa la sesión antes de repetirlo.") }, REPLY_MS)
            ch.invokeMethod("carReply", args, object : MethodChannel.Result {
                override fun success(result: Any?) = finish(result as? String ?: "Hecho.")
                override fun error(code: String, message: String?, details: Any?) =
                    finish("No pude procesar la respuesta; no envié nada.")
                override fun notImplemented() = finish("ComandOS aún está arrancando; no envié nada.")
            })
        }

        /** A steering-wheel / headset button, or «Colgar» on the call notice. */
        fun carButton(button: String) {
            Handler(Looper.getMainLooper()).post {
                live?.channel?.invokeMethod("carButton", button)
            }
        }

        fun carRead(tag: String, session: String?) {
            live?.channel?.invokeMethod("carRead", mapOf("tag" to tag, "session" to session))
        }

        private fun queue(context: Context, args: Map<String, String?>) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val list = try {
                JSONArray(prefs.getString(KEY_QUEUE, "[]"))
            } catch (_: Exception) {
                JSONArray()
            }
            val o = JSONObject()
            for ((k, v) in args) if (v != null) o.put(k, v)
            list.put(o)
            prefs.edit().putString(KEY_QUEUE, list.toString()).apply()
        }

        private fun takeQueue(context: Context): List<Map<String, String>> {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val raw = prefs.getString(KEY_QUEUE, "[]") ?: "[]"
            prefs.edit().remove(KEY_QUEUE).apply()
            val list = try {
                JSONArray(raw)
            } catch (_: Exception) {
                return emptyList()
            }
            return (0 until list.length()).mapNotNull { i ->
                val o = list.optJSONObject(i) ?: return@mapNotNull null
                o.keys().asSequence().associateWith { k -> o.optString(k) }
            }
        }
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
        live = this
    }

    override fun onDetachedFromEngine(b: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
        if (live === this) live = null
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
        if (requestCode != REQ_NOTIF && requestCode != REQ_BLUETOOTH) return false
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
            "carStart" -> result.success(CarService.start(context))
            "carStop" -> {
                Thread { WakeWord.stop() }.start()
                CarService.stop(context)
                result.success(null)
            }
            // «OK ComandOS»: offline listening while the Modo auto call sleeps.
            // Starting and stopping wait for the microphone, so off the main thread.
            "wakeStart" -> Thread {
                WakeWord.start(
                    context,
                    { text, final -> channel?.invokeMethod("wakeText", mapOf("text" to text, "final" to final)) },
                    { why -> channel?.invokeMethod("wakeError", why) },
                )
                Handler(Looper.getMainLooper()).post { result.success(true) }
            }.start()
            "wakeStop" -> Thread {
                WakeWord.stop()
                Handler(Looper.getMainLooper()).post { result.success(null) }
            }.start()
            "requestBluetooth" -> {
                // The car's microphone over Bluetooth (Android 12+ asks for it).
                val activity = binding?.activity
                when {
                    Build.VERSION.SDK_INT < 31 -> result.success(true)
                    context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED ->
                        result.success(true)
                    activity == null -> result.success(false)
                    permResult != null -> result.success(false)
                    else -> {
                        permResult = result
                        activity.requestPermissions(arrayOf(Manifest.permission.BLUETOOTH_CONNECT), REQ_BLUETOOTH)
                    }
                }
            }
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
                    speech = call.argument<String>("speech"),
                    sender = call.argument<String>("sender"),
                    askSig = call.argument<String>("askSig"),
                )
                result.success(null)
            }
            "takeCarReplies" -> result.success(takeQueue(context))
            "carReplied" -> {
                val tag = call.argument<String>("tag")
                if (tag != null) {
                    Notifier.replied(
                        context, tag,
                        call.argument<String>("text") ?: "",
                        call.argument<String>("say") ?: "Hecho.",
                        null, null,
                    )
                }
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
