package app.comandos.comandos_mobile

import app.comandos.comandos_mobile.TakeEndpointer.End
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TakeEndpointerTest {
    private fun ends(seed: String = "", open: Boolean = false) =
        TakeEndpointer(startMs = 0, silenceMs = 3000, noWordsMs = 8000, maxMs = 600_000, seed = seed, seedOpen = open)

    @Test
    fun `OK ComandOS and a pause to think waits for words, it does not end`() {
        val e = ends("okay comandos", open = true)
        assertFalse(e.heardWords)
        assertNull(e.tick(3500))
        // The recognizer revises the wake phrase: still not a word of the turn.
        e.partial("okay y comandos", 1000)
        e.final("okay y comandos", listOf(0.7, 0.6, 1.0), 1500)
        assertFalse(e.heardWords)
        assertNull(e.tick(6000))
        // Five seconds later the person talks.
        e.partial("revisa", 6000)
        e.partial("revisa el deploy", 6800)
        assertTrue(e.heardWords)
        assertNull(e.tick(9000))
        e.final("revisa el deploy", listOf(1.0, 1.0, 0.9), 7500)
        assertNull(e.tick(9700))
        assertEquals(End.SILENCE, e.tick(9800))
        assertEquals("revisa el deploy", e.text)
    }

    @Test
    fun `words in the same breath as the wake phrase count at once`() {
        val e = ends("okay comandos qué hay", open = true)
        assertTrue(e.heardWords)
        e.partial("okay comandos qué hay de nuevo", 600)
        e.final("okay comandos qué hay de nuevo", null, 1200)
        assertNull(e.tick(3500))
        assertEquals(End.SILENCE, e.tick(3600))
        assertEquals("qué hay de nuevo", e.text)
    }

    @Test
    fun `an utterance already finished when the take began is still the turn`() {
        val e = ends("okay comandos qué hay de nuevo", open = false)
        assertTrue(e.heardWords)
        assertEquals("qué hay de nuevo", e.text)
        assertEquals(End.SILENCE, e.tick(3000))
    }

    @Test
    fun `pauses shorter than the silence never cut a long turn`() {
        val e = ends()
        var t = 500L
        repeat(40) { i ->
            e.partial("frase $i", t)
            e.final("frase $i", null, t + 600)
            assertNull(e.tick(t + 2900))
            t += 2900 // a 2.3 s pause after each sentence
        }
        assertEquals(40, e.text.split("frase").size - 1)
        val last = t - 2900
        assertNull(e.tick(last + 2999))
        assertEquals(End.SILENCE, e.tick(last + 3000))
    }

    @Test
    fun `no words at all ends empty`() {
        val e = ends()
        assertNull(e.tick(7900))
        assertEquals(End.NO_WORDS, e.tick(8000))
        assertFalse(e.heardWords)
        assertEquals("", e.text)
    }

    @Test
    fun `noise taken for an unsure word neither extends the turn nor enters it`() {
        val e = ends()
        e.partial("sí", 1000)
        e.final("sí", listOf(1.0), 1400)
        e.partial("el", 2500)
        e.final("el", listOf(0.31), 3000)
        assertEquals("sí", e.text)
        assertEquals(End.SILENCE, e.tick(4000))
    }

    @Test
    fun `a partial that vanished leaves no word behind`() {
        val e = ends()
        e.partial("a", 1000)
        e.final("", null, 1500)
        assertFalse(e.heardWords)
        assertEquals(End.NO_WORDS, e.tick(8000))
    }

    @Test
    fun `the hard cap ends a turn that never pauses`() {
        val e = TakeEndpointer(0, 3000, 8000, maxMs = 10_000)
        for (t in 0L until 10_000L step 500) e.partial("palabra $t", t)
        assertEquals(End.MAX, e.tick(10_000))
        assertTrue(e.heardWords)
    }

    @Test
    fun `wake end is found after chatter and in either spelling`() {
        assertEquals(2, TakeEndpointer.wakeEnd(TakeEndpointer.words("okay comandos")))
        assertEquals(4, TakeEndpointer.wakeEnd(TakeEndpointer.words("aunque hay y comando revisa")))
        assertNull(TakeEndpointer.wakeEnd(TakeEndpointer.words("vamos por tacos")))
    }
}
