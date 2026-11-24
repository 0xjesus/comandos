package app.comandos.comandos_mobile

import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * The two actions of a session conversation, fired by Android Auto (or the
 * notification shade): a reply dictated and confirmed in the car, and «leído».
 * The reply goes to Dart, which re-reads the pane before doing anything; the
 * conversation shows «Enviando…» and then the outcome.
 */
class CarReplyReceiver : BroadcastReceiver() {
    companion object {
        const val ACTION_REPLY = "app.comandos.CAR_REPLY"
        const val ACTION_READ = "app.comandos.CAR_READ"
        const val KEY_REPLY = "reply"
        const val EXTRA_TAG = "tag"
        const val EXTRA_ASK = "askSig"
    }

    override fun onReceive(context: Context, intent: Intent) {
        val tag = intent.getStringExtra(EXTRA_TAG) ?: return
        val session = intent.getStringExtra(Notifier.EXTRA_SESSION)
        val pane = intent.getStringExtra(Notifier.EXTRA_PANE)
        when (intent.action) {
            ACTION_READ -> {
                Notifier.cancel(context, tag)
                NativeBridge.carRead(tag, session)
            }
            ACTION_REPLY -> {
                val text = RemoteInput.getResultsFromIntent(intent)
                    ?.getCharSequence(KEY_REPLY)?.toString()?.trim().orEmpty()
                if (text.isEmpty()) return
                Notifier.replied(context, tag, text, context.getString(R.string.car_sending), session, pane)
                val pending = goAsync()
                val args = mapOf(
                    "tag" to tag,
                    "text" to text,
                    "session" to session,
                    "pane" to pane,
                    "askSig" to intent.getStringExtra(EXTRA_ASK),
                )
                NativeBridge.carReply(context, args) { outcome ->
                    Notifier.replied(context, tag, text, outcome, session, pane)
                    pending.finish()
                }
            }
        }
    }
}
