package app.comandos.comandos_mobile

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import org.json.JSONObject
import org.vosk.LibVosk
import org.vosk.LogLevel
import org.vosk.Model
import org.vosk.Recognizer
import org.vosk.android.StorageService

/**
 * «OK ComandOS» for Modo auto: listens offline with Vosk's small Spanish model
 * (assets `vosk-es`, see build.gradle.kts) and hands every new piece of text
 * to Dart, which decides whether it was the wake phrase
 * (`lib/features/car/wake_phrase.dart`). Nothing leaves the phone.
 *
 * Runs only while the Modo auto call sleeps; Dart stops it before the phone's
 * own recognizer takes the microphone.
 */
object WakeWord {
    private const val RATE = 16000

    private val main = Handler(Looper.getMainLooper())
    private val lock = Object()

    @Volatile
    private var model: Model? = null
    private var thread: Thread? = null

    /** Bumped by every start and stop: a thread from an older run goes quiet. */
    @Volatile
    private var gen = 0

    /**
     * Starts listening; [onText] gets (text, isFinal) for each change, [onError]
     * a reason once if it cannot listen. Both on the main thread.
     */
    fun start(context: Context, onText: (String, Boolean) -> Unit, onError: (String) -> Unit) {
        val app = context.applicationContext
        synchronized(lock) {
            halt()
            val my = ++gen
            thread = Thread({ run(app, my, onText, onError) }, "wake-word").apply { start() }
        }
    }

    /** Stops listening and frees the microphone before returning. */
    fun stop() {
        synchronized(lock) { halt() }
    }

    private fun halt() {
        gen++
        val t = thread ?: return
        t.interrupt()
        try {
            t.join(1500)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
        thread = null
    }

    private fun loadModel(app: Context): Model = model ?: synchronized(this) {
        model ?: run {
            LibVosk.setLogLevel(LogLevel.WARNINGS)
            Model(StorageService.sync(app, "vosk-es", "vosk")).also { model = it }
        }
    }

    @SuppressLint("MissingPermission")
    private fun run(app: Context, my: Int, onText: (String, Boolean) -> Unit, onError: (String) -> Unit) {
        fun live() = my == gen
        fun fail(why: String) {
            main.post { if (live()) onError(why) }
        }
        val m = try {
            loadModel(app)
        } catch (e: Exception) {
            fail("No pude cargar el reconocimiento de «OK ComandOS».")
            return
        }
        if (!live()) return
        val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val chunk = RATE / 5 // 0.2 s
        val recorder = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                maxOf(min, chunk * 2 * 2),
            )
        } catch (e: Exception) {
            fail("No puedo usar el micrófono para «OK ComandOS».")
            return
        }
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            recorder.release()
            fail("El micrófono está ocupado; no puedo escuchar «OK ComandOS».")
            return
        }
        val rec = Recognizer(m, RATE.toFloat())
        try {
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                fail("El micrófono está ocupado; no puedo escuchar «OK ComandOS».")
                return
            }
            val buf = ShortArray(chunk)
            var last = ""
            while (live() && !Thread.currentThread().isInterrupted) {
                val n = recorder.read(buf, 0, buf.size)
                if (n < 0) {
                    fail("Se perdió el micrófono; no puedo escuchar «OK ComandOS».")
                    break
                }
                if (n == 0) continue
                val final = rec.acceptWaveForm(buf, n)
                val text = try {
                    if (final) JSONObject(rec.result).optString("text") else JSONObject(rec.partialResult).optString("partial")
                } catch (_: Exception) {
                    ""
                }
                if (final) {
                    if (text.isNotBlank()) main.post { if (live()) onText(text, true) }
                    last = ""
                } else if (text.isNotBlank() && text != last) {
                    last = text
                    main.post { if (live()) onText(text, false) }
                }
            }
        } finally {
            try {
                recorder.stop()
            } catch (_: Exception) {
            }
            recorder.release()
            rec.close()
        }
    }
}
