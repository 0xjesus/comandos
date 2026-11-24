package app.comandos.comandos_mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.os.Bundle
import java.util.concurrent.ConcurrentHashMap

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

    /**
     * A session as an Android Auto conversation: what the car read ([speech],
     * said by [sender]) and the replies sent from the car with their outcome.
     */
    private class Conversation(
        val title: String,
        val sender: String,
        val speech: String,
        val urgent: Boolean,
        val session: String?,
        val pane: String?,
        val askSig: String?,
        val whenMs: Long,
    ) {
        /** (reply, outcome) in order; the outcome replaces «Enviando…». */
        val replies: MutableList<Pair<String, String>> = mutableListOf()
    }

    private val conversations = ConcurrentHashMap<String, Conversation>()

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
        speech: String? = null,
        sender: String? = null,
        askSig: String? = null,
    ) {
        ensureChannels(context)
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        if (speech != null && Build.VERSION.SDK_INT >= 28) {
            val c = Conversation(
                title, sender ?: title, speech, urgent, session, pane, askSig,
                if (whenMs > 0) whenMs else System.currentTimeMillis(),
            )
            conversations[tag] = c
            post(context, nm, tag, conversation(context, tag, c, quiet = false))
            return
        }
        conversations.remove(tag)
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
        post(context, nm, tag, n)
    }

    private fun post(context: Context, nm: NotificationManager, tag: String, n: Notification) {
        try {
            nm.notify(tag, ID, n)
        } catch (_: SecurityException) {
            // POST_NOTIFICATIONS revoked: nothing to show.
        }
    }

    private fun tapIntent(context: Context, tag: String, session: String?, pane: String?): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .setAction(ACTION_OPEN_CHAT)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (session != null) intent.putExtra(EXTRA_SESSION, session)
        if (pane != null) intent.putExtra(EXTRA_PANE, pane)
        return PendingIntent.getActivity(
            context, tag.hashCode(), intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    /**
     * The MessagingStyle notification Android Auto reads aloud, with the two
     * actions it requires: reply (RemoteInput, dictated and confirmed by the
     * car's assistant) and mark as read. Neither opens the app.
     */
    @android.annotation.TargetApi(28)
    private fun conversation(context: Context, tag: String, c: Conversation, quiet: Boolean): Notification {
        val me = Person.Builder().setName(context.getString(R.string.car_me)).setKey("me").build()
        val who = Person.Builder().setName(c.sender).setKey("s:$tag").build()
        val style = Notification.MessagingStyle(me)
            .setConversationTitle(c.title)
            .setGroupConversation(false)
        style.addMessage(Notification.MessagingStyle.Message(c.speech, c.whenMs, who))
        var at = c.whenMs
        for ((text, outcome) in c.replies) {
            at = maxOf(at + 1, System.currentTimeMillis())
            style.addMessage(Notification.MessagingStyle.Message(text, at, null as Person?))
            style.addMessage(Notification.MessagingStyle.Message(outcome, at + 1, who))
        }
        val icon = Icon.createWithResource(context, R.drawable.ic_stat_comandos)
        val extras = Intent(context, CarReplyReceiver::class.java)
            .putExtra(CarReplyReceiver.EXTRA_TAG, tag)
            .putExtra(EXTRA_SESSION, c.session)
            .putExtra(EXTRA_PANE, c.pane)
            .putExtra(CarReplyReceiver.EXTRA_ASK, c.askSig)
        val replyPi = PendingIntent.getBroadcast(
            context, "reply:$tag".hashCode(),
            Intent(extras).setAction(CarReplyReceiver.ACTION_REPLY),
            // RemoteInput fills the intent: it must stay mutable.
            PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0),
        )
        val readPi = PendingIntent.getBroadcast(
            context, "read:$tag".hashCode(),
            Intent(extras).setAction(CarReplyReceiver.ACTION_READ),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val reply = Notification.Action.Builder(icon, context.getString(R.string.car_reply), replyPi)
            .addRemoteInput(
                RemoteInput.Builder(CarReplyReceiver.KEY_REPLY)
                    .setLabel(context.getString(R.string.car_reply))
                    .build(),
            )
            .setSemanticAction(Notification.Action.SEMANTIC_ACTION_REPLY)
            .addExtras(carExtras(Notification.Action.SEMANTIC_ACTION_REPLY))
            .setAllowGeneratedReplies(false)
            .build()
        val read = Notification.Action.Builder(icon, context.getString(R.string.car_read), readPi)
            .setSemanticAction(Notification.Action.SEMANTIC_ACTION_MARK_AS_READ)
            .addExtras(carExtras(Notification.Action.SEMANTIC_ACTION_MARK_AS_READ))
            .build()
        return Notification.Builder(context, if (c.urgent) CHANNEL_ATTENTION else CHANNEL_DONE)
            .setSmallIcon(R.drawable.ic_stat_comandos)
            .setColor(ACCENT)
            .setStyle(style)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setContentIntent(tapIntent(context, tag, c.session, c.pane))
            .setAutoCancel(true)
            .setOnlyAlertOnce(quiet)
            .setWhen(at)
            .setShowWhen(true)
            .addAction(reply)
            .addAction(read)
            .build()
    }

    /**
     * What AndroidX's NotificationCompat writes for an action that runs in the
     * background (Android Auto reads these keys): it opens no screen.
     */
    private fun carExtras(semantic: Int) = Bundle().apply {
        putBoolean("android.support.action.showsUserInterface", false)
        putInt("android.support.action.semanticAction", semantic)
    }

    /**
     * Shows a reply sent from the car and its [outcome] in the conversation
     * (quietly: no new alert). A later outcome for the same reply replaces
     * the earlier one («Enviando…» → «Enviado.»).
     */
    fun replied(context: Context, tag: String, text: String, outcome: String, session: String?, pane: String?) {
        if (Build.VERSION.SDK_INT < 28) return
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        // The process may have restarted since the notice: keep the thread going.
        val c = conversations.getOrPut(tag) {
            Conversation("ComandOS", "ComandOS", "…", false, session, pane, null, System.currentTimeMillis())
        }
        val last = c.replies.lastOrNull()
        if (last != null && last.first == text) {
            c.replies[c.replies.size - 1] = text to outcome
        } else {
            c.replies.add(text to outcome)
        }
        post(context, nm, tag, conversation(context, tag, c, quiet = true))
    }

    fun cancel(context: Context, tag: String) {
        conversations.remove(tag)
        context.getSystemService(NotificationManager::class.java)?.cancel(tag, ID)
    }

    /** Removes every session notification (not the service's own). */
    fun cancelAll(context: Context) {
        conversations.clear()
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        for (sbn in nm.activeNotifications) {
            if (sbn.tag != null && sbn.id == ID) nm.cancel(sbn.tag, sbn.id)
        }
    }

    fun enabled(context: Context): Boolean =
        context.getSystemService(NotificationManager::class.java)?.areNotificationsEnabled() ?: false
}
