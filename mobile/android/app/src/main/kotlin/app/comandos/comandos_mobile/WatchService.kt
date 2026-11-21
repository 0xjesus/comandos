package app.comandos.comandos_mobile

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/**
 * Foreground service that only keeps the process alive (and shows the quiet
 * "atento a tus sesiones" notification). It does no work itself: the long-poll
 * runs in the Dart main isolate. Type `remoteMessaging` (relays the agents'
 * messages to the phone): unlike `dataSync` it has no 6-hour daily limit on
 * Android 15+ and may start at boot.
 */
class WatchService : Service() {
    companion object {
        private const val ID = 7001
        private const val PREFS = "comandos_watch"
        private const val KEY_ENABLED = "enabled"

        @Volatile
        var isRunning: Boolean = false
            private set

        fun isEnabled(context: Context): Boolean =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY_ENABLED, false)

        private fun setEnabled(context: Context, on: Boolean) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putBoolean(KEY_ENABLED, on).apply()
        }

        /** Called from Dart (app in foreground). */
        fun start(context: Context): Boolean {
            setEnabled(context, true)
            return try {
                context.startForegroundService(Intent(context, WatchService::class.java))
                true
            } catch (e: Exception) {
                // ForegroundServiceStartNotAllowedException when not in foreground.
                false
            }
        }

        fun stop(context: Context) {
            setEnabled(context, false)
            context.stopService(Intent(context, WatchService::class.java))
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!isEnabled(this)) {
            stopSelf()
            return START_NOT_STICKY
        }
        try {
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(ID, buildNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING)
            } else {
                startForeground(ID, buildNotification())
            }
        } catch (e: Exception) {
            stopSelf()
            return START_NOT_STICKY
        }
        isRunning = true
        // Restarted by the system or at boot: no Dart is running yet.
        EngineHolder.ensureHeadless(this)
        return START_STICKY
    }

    override fun onDestroy() {
        isRunning = false
        super.onDestroy()
        // Without the service and without an activity nothing needs Dart.
        Handler(Looper.getMainLooper()).post { EngineHolder.releaseIfIdle() }
    }

    private fun buildNotification(): Notification {
        Notifier.ensureChannels(this)
        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, Notifier.CHANNEL_WATCH)
            .setSmallIcon(R.drawable.ic_stat_comandos)
            .setColor(Notifier.ACCENT)
            .setContentTitle(getString(R.string.watch_title))
            .setContentText(getString(R.string.watch_text))
            .setOngoing(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setContentIntent(open)
            .apply {
                if (Build.VERSION.SDK_INT >= 31) {
                    setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
                }
            }
            .build()
    }
}
