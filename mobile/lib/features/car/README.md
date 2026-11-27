# car

"Modo auto": a hands-free **call** with one Claude Code session, the
orchestrator (`dash/native/auto.rs` on the desktop), which reads and drives the
other sessions and asks for a «sí» before it sends or answers anything. In
Android Auto, sessions are conversations instead
(`features/notifications/car_text.dart`). Open it with `CarMode.open(context)`
from the sessions app bar, the chat menu or settings; it is always the
orchestrator call.

## The call

1. Opening it connects to the orchestrator (`POST /auto/session`), says «Listo.
   Di OK ComandOS cuando me necesites.» and sleeps. Nothing is heard or sent
   until «OK ComandOS», «Hablar» or the steering wheel.
2. The person's turn starts the moment the phrase is heard, without letting go
   of the microphone (see «One microphone» below): «OK ComandOS, qué hay de
   nuevo» in one breath is kept whole, and «OK ComandOS» followed by a pause to
   think waits for the words. An earcon plays meanwhile. The turn ends when no
   new words were heard for the silence set in Ajustes › Voz (3 s by default,
   1–6 s), or empty after 8 s without words; 10 minutes at most. Pauses
   shorter than the silence never cut it.
3. The recording is transcribed like the chat's dictation (OpenAI with the
   person's key, same retries) and the wake phrase is taken off the text.
   Without a key, or if OpenAI fails or hears nothing, what the phone heard
   offline is the text: a turn in which words were heard is never dropped.
   A turn without words (silence, road noise) is dropped without transcribing.
4. The text goes to the orchestrator through the outbox; its answer is read
   aloud WHOLE (not the chat's read mode), in chunks, with Edge (free; the next
   chunk renders while one plays) and shown small on screen («Tú» /
   «ComandOS»). «Hablar» or the steering wheel cut the reading and listen.
5. After an answer it listens for a follow-up (the orchestrator often asks
   «Sí?»); 10 s without words and it sleeps again.
6. On the phone, as the whole utterance: «cuelga» hangs up, «repite» reads the
   last answer again, «espera» sleeps now. When the orchestrator itself asks a
   question (a permission), its options are read and a number is heard with the
   phone's recognizer (Modo auto's microphone is let go first), confirmed, and
   answered.
7. `CarService` (Android, microphone + media playback) keeps the call going
   with the screen off or Maps in front; its media session gets the steering
   wheel / headset buttons: ⏯ / ⏭ talk, end the turn or cut the reading;
   hang-up hangs up.

## One microphone: «OK ComandOS» and the turn

`CarListener.kt` holds the microphone (AudioRecord, VOICE_RECOGNITION, 16 kHz)
and runs Vosk's small Spanish model offline (the model is fetched and checked
at build time, never committed).

- Asleep, every new piece of text goes to Dart, which matches the phrase
  (`wake_phrase.dart`, `isWakePhrase`); the last 3 s of audio stay in a ring.
  Nothing it hears while asleep leaves the phone.
- On a match the call asks for the take (`CarPorts.takeAfterWake`): the same
  recording turns into a 16 kHz WAV that starts 2.5 s back (the phrase and
  what followed it), in the dictation's takes folder. A follow-up opens a
  take directly.
- The end of the turn is decided by words, not by level
  (`TakeEndpointer.kt`, JVM unit tests in `android/app/src/test`): in a
  moving car the voice is often not much louder than the road, but the road
  does not become words. The wake phrase does not count as words of the
  turn; an utterance whose every word Vosk is unsure of (< 0.5) is noise.
- Dart gets `{path, text, words}` (`car_mic.dart`), can end a take now
  («Hablar», the wheel) or cancel it (hang up), and frees the microphone
  before the phone's recognizer listens. After a take the microphone is
  free while the reply is read.
- If Modo auto's microphone cannot record (no model, microphone busy), the
  chat's dictation records the turn instead.

## Cost

Speaking is free: an Edge neural voice by default (`edge_voice.dart`: the
desktop renders it with `edge-tts` at `POST /tts`; the text goes to
Microsoft) or the phone's voice. When Edge fails or is slow the phone's voice
speaks at once. Listening after the wake phrase is an OpenAI transcription
(like the chat's dictation; the WAV is ≈ 1 MB per 30 s); turns without words
are dropped without transcribing.

## Files

- `orchestrator_call.dart`: the call as a pure state machine over
  `CarPorts` and an `OrchestratorLink` (tested with fakes).
- `orchestrator_screen.dart`: wiring (link, ports, wake, Android service).
- `orchestrator_view.dart`: the call screen, pure (state in, callbacks out).
- `car_ports.dart`: `CarPorts` and its live version (Modo auto's microphone,
  transcription, Edge/phone voice, outbox, chat).
- `car_mic.dart`: `CarMic`, Modo auto's microphone (`CarListener.kt`).
- `car_commands.dart`, `option_parser.dart`: spoken commands and options.
- `on_device.dart`, `edge_voice.dart`, `wake_phrase.dart`: voices and the wake phrase.

Sends go only through `sl.get<Outbox>().enqueue(...)`. Dictated text that was
not sent goes to the session's draft, never lost.
