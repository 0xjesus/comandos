package app.comandos.comandos_mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Restarts the watcher after a reboot or an app update, if it was on. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED ->
                if (WatchService.isEnabled(context)) {
                    try {
                        context.startForegroundService(Intent(context, WatchService::class.java))
                    } catch (_: Exception) {
                        // Not allowed right now; the next app start restarts it.
                    }
                }
        }
    }
}
