package app.comandos.comandos_mobile

/**
 * Decides when the person's turn in Modo auto is over, from the words the
 * offline recognizer (Vosk) hears, never from the input level: in a moving
 * car the voice is often not much louder than the road, but road noise does
 * not turn into words.
 *
 * - The turn ends [silenceMs] after the last new word.
 * - With no word at all within [noWordsMs] it ends empty ([End.NO_WORDS]).
 * - [maxMs] is a hard cap.
 *
 * A take that starts right after «OK ComandOS» is seeded with what the
 * recognizer had for that utterance ([seed]); the wake phrase itself (up to
 * «comandos») is not a word of the turn, so «OK ComandOS» followed by a pause
 * to think waits for words instead of ending, while «OK ComandOS, qué hay de
 * nuevo» said in one breath already counts.
 *
 * Pure (time comes in as arguments) so it is unit tested
 * (`src/test/.../TakeEndpointerTest.kt`).
 */
class TakeEndpointer(
    private val startMs: Long,
    private val silenceMs: Long,
    private val noWordsMs: Long,
    private val maxMs: Long,
    seed: String = "",
    seedOpen: Boolean = false,
    skipHint: Int = 0,
) {
    enum class End { SILENCE, NO_WORDS, MAX }

    /** Words of finished utterances that belong to the turn. */
    private val finals = mutableListOf<String>()

    /** The utterance in progress (if the take started inside it) still starts with the wake phrase. */
    private var firstOpen = seedOpen

    /** Words of the first utterance that are the wake phrase. */
    private val skip: Int

    /** Last new word of a finished utterance. */
    private var confirmedAt: Long? = null

    /** Last change of the utterance in progress (provisional: it may turn out to be noise). */
    private var pendingAt: Long? = null
    private var pendingWords = emptyList<String>()

    init {
        val seedWords = words(seed)
        skip = if (seedWords.isEmpty()) 0 else wakeEnd(seedWords) ?: skipHint.coerceIn(0, seedWords.size)
        if (seedOpen) {
            firstOpen = true
            partial(seed, startMs)
        } else if (seedWords.isNotEmpty()) {
            firstOpen = true
            final(seed, null, startMs)
        }
    }

    /** A partial result of the utterance in progress. */
    fun partial(text: String, nowMs: Long) {
        val beyond = beyond(words(text))
        if (beyond == pendingWords) return
        pendingWords = beyond
        pendingAt = if (beyond.isEmpty()) null else nowMs
    }

    /**
     * The utterance ended with [text]; [confs] are the recognizer's per-word
     * confidences when it gave them. An utterance whose every word is unsure
     * is taken for noise: it neither extends the turn nor enters its text.
     */
    fun final(text: String, confs: List<Double>?, nowMs: Long) {
        val all = words(text)
        val cut = cutFor(all)
        val beyond = all.drop(cut)
        val sure = confs?.takeIf { it.size == all.size }?.drop(cut)
        val noise = sure != null && sure.isNotEmpty() && sure.all { it < NOISE_CONF }
        if (beyond.isNotEmpty() && !noise) {
            finals.add(beyond.joinToString(" "))
            confirmedAt = pendingAt ?: nowMs
        }
        firstOpen = false
        pendingWords = emptyList()
        pendingAt = null
    }

    /** Whether the turn is over at [nowMs], and why. */
    fun tick(nowMs: Long): End? {
        if (nowMs - startMs >= maxMs) return End.MAX
        val last = listOfNotNull(confirmedAt, pendingAt).maxOrNull()
            ?: return if (nowMs - startMs >= noWordsMs) End.NO_WORDS else null
        return if (nowMs - last >= silenceMs) End.SILENCE else null
    }

    /** Some word of the turn was heard (the wake phrase does not count). */
    val heardWords: Boolean get() = finals.isNotEmpty() || pendingWords.isNotEmpty()

    /** What the recognizer heard in the turn, without the wake phrase. */
    val text: String get() = (finals + pendingWords.joinToString(" ")).filter { it.isNotBlank() }.joinToString(" ")

    private fun beyond(all: List<String>): List<String> = all.drop(cutFor(all))

    /**
     * Wake-phrase words at the start of [all]: only in the first utterance.
     * Found again in every revision («okay comandos» may become «okay y
     * comandos»), else what the seed said.
     */
    private fun cutFor(all: List<String>): Int = if (!firstOpen) 0 else (wakeEnd(all) ?: skip).coerceAtMost(all.size)

    companion object {
        /** Vosk gives real words 0.3–1.0 even in road noise; an utterance of only lower ones is noise. */
        const val NOISE_CONF = 0.5

        fun words(text: String): List<String> = text.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }

        /** Words up to and including «comandos» (the end of the wake phrase), or null. */
        fun wakeEnd(words: List<String>): Int? {
            val i = words.indexOfFirst { it.lowercase().startsWith("comando") }
            return if (i < 0) null else i + 1
        }
    }
}
