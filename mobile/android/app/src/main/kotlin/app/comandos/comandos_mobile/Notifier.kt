package app.comandos.comandos_mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

/** Notification channels and the per-session notifications. Framework API only (minSdk 26). */
object Notifier {
    const val CHANNEL_ATTENTION = "attention"
    const val CHANNEL_DONE = "done"
    const val CHANNEL_WATCH = "watch"
    const val ACCENT = 0xFF8B7CFF.toInt()
    const val ACTION_OPEN_CHAT = "app.comandos.OPEN_CHAT"
    const val EXTRA_SESSION = "session"
    const val EXTRA_PANE = "pane"

    /** One notification per tag (= session): a newer event replaces it. */
    private const val ID = 1

    fun ensureChannels(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(CHANNEL_ATTENTION) != null) return
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ATTENTION, context.getString(R.string.channel_attention), NotificationManager.IMPORTANCE_HIGH)
                .apply { description = context.getString(R.string.channel_attention_desc) },
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_DONE, context.getString(R.string.channel_done), NotificationManager.IMPORTANCE_DEFAULT)
                .apply { description = context.getString(R.string.channel_done_desc) },
        )
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_WATCH, context.getString(R.string.channel_watch), NotificationManager.IMPORTANCE_MIN)
                .apply {
                    description = context.getString(R.string.channel_watch_desc)
                    setShowBadge(false)
                    setSound(null, null)
                    enableVibration(false)
                },
        )
    }

    fun show(
        context: Context,
        tag: String,
        urgent: Boolean,
        title: String,
        body: String,
        session: String?,
        pane: String?,
        whenMs: Long,
    ) {
        ensureChannels(context)
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        val intent = Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_CHAT)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (session != null) intent.putExtra(EXTRA_SESSION, session)
        if (pane != null) intent.putExtra(EXTRA_PANE, pane)
        val tap = PendingIntent.getActivity(
            context, tag.hashCode(), intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(context, if (urgent) CHANNEL_ATTENTION else CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_stat_comandos)
            .setColor(ACCENT)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setCategory(if (urgent) Notification.CATEGORY_MESSAGE else Notification.CATEGORY_STATUS)
            .setAutoCancel(true)
            .setContentIntent(tap)
            .setWhen(if (whenMs > 0) whenMs else System.currentTimeMillis())
            .setShowWhen(true)
            .build()
        try {
            nm.notify(tag, ID, n)
        } catch (_: SecurityException) {
            // POST_NOTIFICATIONS revoked: nothing to show.
        }
    }

    fun cancel(context: Context, tag: String) {
        context.getSystemService(NotificationManager::class.java)?.cancel(tag, ID)
    }

    /** Removes every session notification (not the service's own). */
    fun cancelAll(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        for (sbn in nm.activeNotifications) {
            if (sbn.tag != null && sbn.id == ID) nm.cancel(sbn.tag, sbn.id)
        }
    }

    fun enabled(context: Context): Boolean =
        context.getSystemService(NotificationManager::class.java)?.areNotificationsEnabled() ?: false
}
