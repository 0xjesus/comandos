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
2. Then an earcon, and it records what is said like the chat's dictation
   (`DictationController`: same recording, OpenAI transcription with the
   person's key). The turn ends after the silence set in Ajustes › Voz (2.5 s
   by default); a long sentence is never cut while the person talks. Without a
   key the phone's recognizer listens instead.
3. The text goes to the orchestrator through the outbox; its answer is read
   aloud with Edge (free) and shown small on screen («Tú» / «ComandOS»).
4. After an answer it listens for a follow-up (the orchestrator often asks
   «¿Sí?»); 10 s without a voice and it sleeps again.
5. On the phone, as the whole utterance: «cuelga» hangs up, «repite» reads the
   last answer again, «espera» sleeps now. When the orchestrator itself asks a
   question (a permission), its options are read and a number is heard with the
   phone's recognizer, confirmed, and answered.
6. `CarService` (Android, microphone + media playback) keeps the call going
   with the screen off or Maps in front; its media session gets the steering
   wheel / headset buttons: ⏯ / ⏭ talk or cut the reading, hang-up hangs up.

## «OK ComandOS»

Heard offline by Vosk's small Spanish model (`WakeWord.kt`; the model is
fetched and checked at build time, never committed) and matched in Dart
(`wake_phrase.dart`). Nothing it hears while asleep leaves the phone.

## Cost

Speaking is free: an Edge neural voice by default (`edge_voice.dart`: the
desktop renders it with `edge-tts` at `POST /tts`; the text goes to
Microsoft) or the phone's voice. When Edge fails or is slow the phone's voice
speaks at once. Listening after the wake phrase is an OpenAI transcription
(like the chat's dictation); silent rounds are dropped without transcribing.

## Files

- `orchestrator_call.dart`: the call as a pure state machine over
  `CarPorts` and an `OrchestratorLink` (tested with fakes).
- `orchestrator_screen.dart`: wiring (link, ports, wake, Android service).
- `orchestrator_view.dart`: the call screen, pure (state in, callbacks out).
- `car_ports.dart`: `CarPorts` and its live version (dictation, Edge/phone
  voice, outbox, chat).
- `car_commands.dart`, `option_parser.dart`: spoken commands and options.
- `on_device.dart`, `edge_voice.dart`, `wake_phrase.dart`: voices and the wake phrase.

Sends go only through `sl.get<Outbox>().enqueue(...)`. Dictated text that was
not sent goes to the session's draft, never lost.
