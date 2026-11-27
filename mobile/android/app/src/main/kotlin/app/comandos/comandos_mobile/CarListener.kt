package app.comandos.comandos_mobile

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import org.vosk.LibVosk
import org.vosk.LogLevel
import org.vosk.Model
import org.vosk.Recognizer
import org.vosk.android.StorageService
import java.io.BufferedOutputStream
import java.io.File
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.log10
import kotlin.math.sqrt

/**
 * Modo auto's microphone: ONE listener that keeps the microphone from «OK
 * ComandOS» to the end of what the person says. Offline, with Vosk's small
 * Spanish model (assets `vosk-es`, see build.gradle.kts).
 *
 * - Sleeping: every new piece of text goes to Dart, which decides whether it
 *   was the wake phrase (`lib/features/car/wake_phrase.dart`). Nothing leaves
 *   the phone. The last [PRE_ROLL_MAX_MS] of audio are kept in a ring.
 * - A take (the person's turn): asked by Dart right when the wake phrase
 *   matched, without letting the microphone go, or directly for a follow-up.
 *   It writes a 16 kHz mono WAV that starts with the ring (so words said in
 *   the same breath as «OK ComandOS» are kept) and ends by word activity
 *   ([TakeEndpointer]), not by level. Dart gets the file, what Vosk heard and
 *   whether it heard words; it transcribes the file with OpenAI.
 *
 * After a take the microphone is freed: the phone's recognizer and the
 * read-aloud can use it. Dart stops the listener before them anyway.
 */
object CarListener {
    private const val TAG = "CarListener"
    private const val RATE = 16000
    private const val CHUNK = RATE / 10 // 0.1 s
    private const val PRE_ROLL_MAX_MS = 3000

    /** A take Dart asked for. [done] runs once, on the main thread. */
    class TakeSpec(
        val dir: String,
        val silenceMs: Long,
        val noWordsMs: Long,
        val maxMs: Long,
        val preRollMs: Int,
        val skipHint: Int,
        val onLevel: (Double) -> Unit,
        private val done: (Map<String, Any?>) -> Unit,
    ) {
        private val finished = AtomicBoolean(false)

        fun finish(result: Map<String, Any?>) {
            if (finished.compareAndSet(false, true)) main.post { done(result) }
        }
    }

    private val main = Handler(Looper.getMainLooper())

    /** Guards the thread's lifecycle (start / stop). */
    private val lock = Object()

    /** Guards the requests handed to the running thread. */
    private val hand = Object()

    @Volatile
    private var model: Model? = null
    private var thread: Thread? = null

    /** Bumped by every start and stop: a thread from an older run goes quiet. */
    @Volatile
    private var gen = 0

    // Requests to the running thread (under [hand]).
    private var wanted: TakeSpec? = null
    private var ending = false
    private var cancelling = false

    /** The running thread is sleeping (it can turn into a take). */
    private var sleeping = false

    /**
     * Listens for the wake phrase; [onText] gets (text, isFinal) for each
     * change, [onError] a reason once if it cannot listen. Both on the main
     * thread. Any take in progress is cancelled.
     */
    fun sleep(context: Context, onText: (String, Boolean) -> Unit, onError: (String) -> Unit) {
        val app = context.applicationContext
        synchronized(lock) {
            halt()
            val my = ++gen
            synchronized(hand) { sleeping = true }
            thread = Thread({ run(app, my, null, onText, onError) }, "car-listener").apply { start() }
        }
    }

    /**
     * Starts the person's turn. While sleeping it turns the running listener
     * into the take (pre-roll included, the microphone never let go);
     * otherwise it opens the microphone for it.
     */
    fun take(context: Context, spec: TakeSpec) {
        val app = context.applicationContext
        synchronized(lock) {
            synchronized(hand) {
                ending = false
                cancelling = false
                if (sleeping && thread?.isAlive == true) {
                    wanted?.finish(cancelled())
                    wanted = spec
                    return
                }
            }
            halt()
            val my = ++gen
            thread = Thread({ run(app, my, spec, { _, _ -> }, { why -> spec.finish(failed(why)) }) }, "car-listener")
                .apply { start() }
        }
    }

    /** Ends the take now; it completes with what was said so far. */
    fun end() {
        synchronized(hand) { ending = true }
    }

    /** Drops the take: its file is deleted, it completes as cancelled. */
    fun cancel() {
        synchronized(hand) { cancelling = true }
    }

    /** Stops listening and frees the microphone before returning. */
    fun stop() {
        synchronized(lock) { halt() }
    }

    private fun halt() {
        gen++
        val t = thread
        if (t != null) {
            t.interrupt()
            try {
                t.join(1500)
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            }
            thread = null
        }
        // A take asked for but never begun by the stopped thread.
        synchronized(hand) {
            wanted?.finish(cancelled())
            wanted = null
            sleeping = false
        }
    }

    private fun cancelled(): Map<String, Any?> = mapOf("end" to "cancelled")

    private fun failed(why: String): Map<String, Any?> = mapOf("end" to "error", "error" to why)

    private fun loadModel(app: Context): Model = model ?: synchronized(this) {
        model ?: run {
            LibVosk.setLogLevel(LogLevel.WARNINGS)
            Model(StorageService.sync(app, "vosk-es", "vosk")).also { model = it }
        }
    }

    /** A take being written. */
    private class Take(val spec: TakeSpec, val file: File, val out: BufferedOutputStream, val ends: TakeEndpointer) {
        var bytes = 0L
    }

    private fun run(
        app: Context,
        my: Int,
        first: TakeSpec?,
        onText: (String, Boolean) -> Unit,
        onError: (String) -> Unit,
    ) {
        try {
            listen(app, my, first, onText, onError)
        } finally {
            // Ended on its own (a take is over, the microphone was lost): a
            // take asked for meanwhile cannot begin here. A stopped run leaves
            // this to [halt] (a newer run may own these fields by now).
            synchronized(hand) {
                if (my == gen) {
                    wanted?.finish(failed("Se perdió el micrófono."))
                    wanted = null
                    sleeping = false
                }
            }
        }
    }

    @SuppressLint("MissingPermission")
    private fun listen(
        app: Context,
        my: Int,
        first: TakeSpec?,
        onText: (String, Boolean) -> Unit,
        onError: (String) -> Unit,
    ) {
        fun live() = my == gen
        fun fail(why: String) {
            first?.finish(failed(why))
            main.post { if (live()) onError(why) }
        }
        val m = try {
            loadModel(app)
        } catch (e: Exception) {
            fail("No pude cargar el reconocimiento de «OK ComandOS».")
            return
        }
        if (!live()) {
            first?.finish(cancelled())
            return
        }
        val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val recorder = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
                maxOf(min, CHUNK * 2 * 4),
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
        rec.setWords(true)
        val ring = ShortArray(RATE * PRE_ROLL_MAX_MS / 1000)
        var ringPos = 0
        var ringFill = 0
        var lastPartial = ""
        var debugAt = 0L
        var lastFinal = ""
        var lastFinalAt = 0L
        var take: Take? = null

        fun begin(spec: TakeSpec, fromSleep: Boolean): Take? {
            val now = System.currentTimeMillis()
            val file = File(spec.dir, "take-$now.wav")
            val out = try {
                File(spec.dir).mkdirs()
                BufferedOutputStream(FileOutputStream(file), 1 shl 16).also { it.write(wavHeader(0)) }
            } catch (e: Exception) {
                spec.finish(failed("No pude guardar la grabación."))
                return null
            }
            // What the recognizer already had of the utterance with the wake phrase.
            val (seed, open) = when {
                !fromSleep -> "" to false
                lastPartial.isNotBlank() -> lastPartial to true
                lastFinal.isNotBlank() && now - lastFinalAt < 2500 -> lastFinal to false
                else -> "" to false
            }
            val t = Take(
                spec, file, out,
                TakeEndpointer(now, spec.silenceMs, spec.noWordsMs, spec.maxMs, seed, open, spec.skipHint),
            )
            if (fromSleep) {
                // The pre-roll: the last [preRollMs] of the ring, oldest first.
                val n = minOf(ringFill, RATE * spec.preRollMs / 1000)
                val pre = ShortArray(n)
                for (i in 0 until n) pre[i] = ring[(ringPos - n + i + ring.size) % ring.size]
                write(t, pre, n)
            }
            Log.i(TAG, "take started (pre-roll ${t.bytes / 32} ms, seeded ${seed.isNotBlank()})")
            return t
        }

        fun close(t: Take, end: String) {
            try {
                t.out.close()
                RandomAccessFile(t.file, "rw").use { f ->
                    f.seek(0)
                    f.write(wavHeader(t.bytes))
                }
            } catch (e: Exception) {
                t.file.delete()
                t.spec.finish(failed("No pude guardar la grabación."))
                return
            }
            val words = t.ends.heardWords
            Log.i(TAG, "take ended: $end, ${t.bytes / 32} ms, words=$words")
            t.spec.finish(
                mapOf("end" to end, "path" to t.file.path, "text" to t.ends.text, "words" to words, "ms" to t.bytes / 32),
            )
        }

        fun drop(t: Take) {
            try {
                t.out.close()
            } catch (_: Exception) {
            }
            t.file.delete()
            Log.i(TAG, "take cancelled")
            t.spec.finish(cancelled())
        }

        try {
            recorder.startRecording()
            if (recorder.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                fail("El micrófono está ocupado; no puedo escuchar «OK ComandOS».")
                return
            }
            if (first != null) {
                take = begin(first, fromSleep = false) ?: return
            }
            val buf = ShortArray(CHUNK)
            while (live() && !Thread.currentThread().isInterrupted) {
                if (take == null) {
                    val spec = synchronized(hand) {
                        wanted.also {
                            wanted = null
                            if (it != null) sleeping = false
                        }
                    }
                    if (spec != null) take = begin(spec, fromSleep = true) ?: break
                }
                val n = recorder.read(buf, 0, buf.size)
                if (n < 0) {
                    fail("Se perdió el micrófono; no puedo escuchar «OK ComandOS».")
                    take?.let { close(it, "error") }
                    take = null
                    break
                }
                if (n == 0) continue
                val t = take
                // Diagnosis only, off by default: `adb shell setprop log.tag.<TAG> DEBUG`
                // logs the level and what Vosk understands while asleep.
                if (t == null && Log.isLoggable(TAG, Log.DEBUG)) {
                    val at = System.currentTimeMillis()
                    if (at - debugAt >= 2000) {
                        debugAt = at
                        Log.i(TAG, "asleep: level ${"%.1f".format(levelDb(buf, n))} dB")
                    }
                }
                if (t == null) {
                    for (i in 0 until n) ring[(ringPos + i) % ring.size] = buf[i]
                    ringPos = (ringPos + n) % ring.size
                    ringFill = minOf(ring.size, ringFill + n)
                } else {
                    write(t, buf, n)
                    val db = levelDb(buf, n)
                    t.spec.onLevel(db)
                }
                val final = rec.acceptWaveForm(buf, n)
                val now = System.currentTimeMillis()
                if (final) {
                    val (text, confs) = parseFinal(rec.result)
                    if (t == null) {
                        if (text.isNotBlank() && Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "heard (final): $text")
                        if (text.isNotBlank()) main.post { if (live()) onText(text, true) }
                        lastFinal = text
                        lastFinalAt = now
                    } else {
                        t.ends.final(text, confs, now)
                    }
                    lastPartial = ""
                } else {
                    val text = try {
                        JSONObject(rec.partialResult).optString("partial")
                    } catch (_: Exception) {
                        ""
                    }
                    if (text != lastPartial) {
                        lastPartial = text
                        if (t == null) {
                            if (text.isNotBlank() && Log.isLoggable(TAG, Log.DEBUG)) Log.i(TAG, "heard: $text")
                            if (text.isNotBlank()) main.post { if (live()) onText(text, false) }
                        } else {
                            t.ends.partial(text, now)
                        }
                    }
                }
                if (t != null) {
                    val (end, cancel) = synchronized(hand) { ending to cancelling }
                    if (cancel) {
                        drop(t)
                        take = null
                        break
                    }
                    val why = if (end) "stopped" else when (t.ends.tick(now)) {
                        TakeEndpointer.End.SILENCE -> "silence"
                        TakeEndpointer.End.NO_WORDS -> "nowords"
                        TakeEndpointer.End.MAX -> "max"
                        null -> null
                    }
                    if (why != null) {
                        close(t, why)
                        take = null
                        break // the take is over: the microphone is freed
                    }
                }
            }
        } finally {
            try {
                recorder.stop()
            } catch (_: Exception) {
            }
            recorder.release()
            rec.close()
            // Stopped from outside (hang up, another start): nothing is kept.
            take?.let { drop(it) }
        }
    }

    private fun write(t: Take, pcm: ShortArray, n: Int) {
        val b = ByteArray(n * 2)
        for (i in 0 until n) {
            val s = pcm[i].toInt()
            b[i * 2] = (s and 0xff).toByte()
            b[i * 2 + 1] = (s shr 8 and 0xff).toByte()
        }
        t.out.write(b)
        t.bytes += b.size
    }

    /** RMS level in dBFS (≈ -90 silence … 0 loudest). */
    private fun levelDb(pcm: ShortArray, n: Int): Double {
        var sum = 0.0
        for (i in 0 until n) {
            val v = pcm[i] / 32768.0
            sum += v * v
        }
        val rms = sqrt(sum / n)
        return if (rms <= 0) -120.0 else 20 * log10(rms)
    }

    /** Text and per-word confidences of a final result. */
    private fun parseFinal(json: String): Pair<String, List<Double>?> = try {
        val j = JSONObject(json)
        val words = j.optJSONArray("result")
        val confs = words?.let { a -> (0 until a.length()).map { a.getJSONObject(it).optDouble("conf", 1.0) } }
        j.optString("text") to confs
    } catch (_: Exception) {
        "" to null
    }

    /** 44-byte RIFF header for [dataBytes] of 16-bit mono PCM at [RATE]. */
    private fun wavHeader(dataBytes: Long): ByteArray {
        val h = ByteArray(44)
        fun str(at: Int, s: String) = s.forEachIndexed { i, c -> h[at + i] = c.code.toByte() }
        fun int(at: Int, v: Long) {
            for (i in 0 until 4) h[at + i] = (v shr (8 * i) and 0xff).toByte()
        }
        fun short(at: Int, v: Int) {
            h[at] = (v and 0xff).toByte()
            h[at + 1] = (v shr 8 and 0xff).toByte()
        }
        str(0, "RIFF")
        int(4, 36 + dataBytes)
        str(8, "WAVE")
        str(12, "fmt ")
        int(16, 16)
        short(20, 1) // PCM
        short(22, 1) // mono
        int(24, RATE.toLong())
        int(28, RATE * 2L) // byte rate
        short(32, 2) // block align
        short(34, 16) // bits
        str(36, "data")
        int(40, dataBytes)
        return h
    }
}
