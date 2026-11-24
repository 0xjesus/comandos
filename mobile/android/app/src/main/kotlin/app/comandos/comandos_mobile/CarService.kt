package app.comandos.comandos_mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.view.KeyEvent

/**
 * Modo auto in a call: keeps ComandOS alive and listening with the screen off
 * or another app (Maps, Waze) in front, and receives the steering-wheel and
 * headset buttons through a media session:
 * ⏯ talk / cut the reading / confirm · ⏭ next split · ⏮ repeat.
 * Started by Dart when the call starts (the app is on screen then, as Android
 * requires for the microphone) and stopped when it ends.
 */
class CarService : Service() {
    companion object {
        private const val ID = 7002
        private const val CHANNEL = "car"
        const val ACTION_HANG_UP = "app.comandos.CAR_HANG_UP"

        @Volatile
        var running: Boolean = false
            private set

        fun start(context: Context): Boolean = try {
            context.startForegroundService(Intent(context, CarService::class.java))
            true
        } catch (_: Exception) {
            false
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, CarService::class.java))
        }
    }

    private var session: MediaSession? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_HANG_UP) {
            NativeBridge.carButton("hangup")
            return START_NOT_STICKY
        }
        val n = notification()
        try {
            if (Build.VERSION.SDK_INT >= 30) {
                // The microphone type needs RECORD_AUDIO granted; without it the
                // call still keeps playing (and the buttons work).
                try {
                    startForeground(
                        ID, n,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
                    )
                } catch (_: SecurityException) {
                    startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
                }
            } else {
                startForeground(ID, n)
            }
        } catch (_: Exception) {
            stopSelf()
            return START_NOT_STICKY
        }
        running = true
        if (session == null) session = mediaSession()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        running = false
        session?.run {
            isActive = false
            release()
        }
        session = null
        super.onDestroy()
    }

    /** The phone routes media buttons to the active session that plays. */
    private fun mediaSession(): MediaSession = MediaSession(this, "ComandOS · Modo auto").apply {
        setCallback(object : MediaSession.Callback() {
            override fun onMediaButtonEvent(mediaButtonIntent: Intent): Boolean {
                val ev: KeyEvent = (
                    if (Build.VERSION.SDK_INT >= 33) {
                        mediaButtonIntent.getParcelableExtra(Intent.EXTRA_KEY_EVENT, KeyEvent::class.java)
                    } else {
                        @Suppress("DEPRECATION")
                        mediaButtonIntent.getParcelableExtra(Intent.EXTRA_KEY_EVENT)
                    }
                    ) ?: return super.onMediaButtonEvent(mediaButtonIntent)
                val button = when (ev.keyCode) {
                    KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, KeyEvent.KEYCODE_MEDIA_PLAY,
                    KeyEvent.KEYCODE_MEDIA_PAUSE, KeyEvent.KEYCODE_HEADSETHOOK,
                    -> "talk"
                    KeyEvent.KEYCODE_MEDIA_NEXT -> "next"
                    KeyEvent.KEYCODE_MEDIA_PREVIOUS -> "previous"
                    else -> return super.onMediaButtonEvent(mediaButtonIntent)
                }
                // One press, one action (no repeats while held).
                if (ev.action == KeyEvent.ACTION_DOWN && ev.repeatCount == 0) NativeBridge.carButton(button)
                return true
            }

            // Head units that call the transport controls directly.
            override fun onPlay() = NativeBridge.carButton("talk")
            override fun onPause() = NativeBridge.carButton("talk")
            override fun onSkipToNext() = NativeBridge.carButton("next")
            override fun onSkipToPrevious() = NativeBridge.carButton("previous")
        })
        setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
                        PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS,
                )
                .setState(PlaybackState.STATE_PLAYING, 0, 1f)
                .build(),
        )
        isActive = true
    }

    private fun notification(): Notification {
        val nm = getSystemService(NotificationManager::class.java)
        if (nm?.getNotificationChannel(CHANNEL) == null) {
            nm?.createNotificationChannel(
                NotificationChannel(CHANNEL, getString(R.string.car_channel), NotificationManager.IMPORTANCE_LOW)
                    .apply {
                        description = getString(R.string.car_channel_desc)
                        setShowBadge(false)
                        setSound(null, null)
                        enableVibration(false)
                    },
            )
        }
        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val hangUp = PendingIntent.getService(
            this, 1,
            Intent(this, CarService::class.java).setAction(ACTION_HANG_UP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_comandos)
            .setColor(Notifier.ACCENT)
            .setContentTitle(getString(R.string.car_title))
            .setContentText(getString(R.string.car_text))
            .setOngoing(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_CALL)
            .setContentIntent(open)
            .addAction(
                Notification.Action.Builder(
                    android.graphics.drawable.Icon.createWithResource(this, R.drawable.ic_stat_comandos),
                    getString(R.string.car_hang_up), hangUp,
                ).build(),
            )
            .apply {
                if (Build.VERSION.SDK_INT >= 31) setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }
            .build()
    }
}
