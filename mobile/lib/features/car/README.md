# car

"Modo auto": a hands-free **call** for a phone on a car mount (in Android Auto,
sessions are conversations instead: `features/notifications/car_text.dart`). Open it with `CarMode.open(context, session:, pane:)` (sessions app bar,
chat menu, settings).

## The call (no button is ever required)

1. Opening it starts the call: "N splits te esperan", then it visits the splits
   one by one (a split = one pane of a session, from `paneList`), waiting
   first: "Pagos, split 2, claude opus 5.", then the permission prompt and its
   options, or the new reply; then an earcon and it listens.
2. What is said is a command, an answer, or a message. A message is read back
   whole ("Tu mensaje: «…». ¿Lo envío?") and sent **only on an explicit «sí» /
   «envíalo»** (or ⏯ / Enviar): silence, «no» or anything else keeps it as a
   draft. A prompt is read with the command it approves, answered by saying the
   option ("uno", "sí", "la dos"), read back ("Opción dos: no. ¿Confirmo?") and
   answered only on a «sí"; the pane is read again on the server right before.
   A tapped option asks «¿Confirmo?» too.
3. Commands, as the whole utterance (`car_commands.dart`): «siguiente»,
   «anterior», «repite», «no»/«cancela», «para»/«cuelga» (hang up), «espera»
   (pause; «sigue» resumes, or it resumes alone after 2 min), «mensaje» (open a
   turn while waiting). Longer sentences are messages ("sigue con el deploy").
4. Silence moves on to the next split that waits; with none, it says "Nada más
   por ahora", listens for commands a few rounds, then stays silent and speaks
   as soon as a reply, a prompt or another split arrives.
5. Commands are heard when it listens (after each turn, after «¿Lo envío?»,
   while waiting), not over its own voice: ⏯ on the steering wheel (or Callar)
   cuts a long reading. «siguiente» / «anterior» go to the next split that waits.
6. In a call `CarService` (Android, microphone + media playback) keeps it going
   with the screen off or Maps in front, and its media session gets the
   steering-wheel / headset buttons: ⏯ talk, cut the reading or confirm; ⏭ next
   split; ⏮ repeat. «Colgar» on its notice hangs up. The screen shows only the
   state and a big glyph, never the reply or the dictated text.

## «OK ComandOS» (Orquestador)

The orchestrator call works like «OK Google»: after it speaks it listens; 10 s
without the person's voice and it sleeps («Di «OK ComandOS»»). The phrase is
heard offline by Vosk's small Spanish model (`WakeWord.kt`; the model is fetched
and checked at build time, never committed) and matched in Dart
(`wake_phrase.dart`). Nothing it hears while asleep leaves the phone. The
steering wheel or «Hablar» wakes it too; «espera» puts it to sleep now.

## Cost

Nothing costs money. It speaks with an Edge neural voice by default
(`edge_voice.dart`: the desktop renders it with `edge-tts` at `POST /tts`; the
text goes to Microsoft) or the phone's voice (Ajustes › Voz › Voz del Modo
auto); when Edge fails or is slow the phone's voice speaks at once and stays
for 2 minutes. Listening is always the phone's recognizer (`speech_to_text`),
never OpenAI, not even as a fallback.

## Files

- `car_machine.dart`: the whole flow as an explicit state machine (pure Dart,
  injected clock/timers and `CarPorts`). `startCall()` turns the call on;
  without it the machine is the original tap-driven Modo auto (kept, tested).
- `car_commands.dart`: spoken commands → `CarCommand`.
- `option_parser.dart`: spoken answers ("dos", "opción uno", "sí") → option.
- `on_device.dart`: the phone's voice and recognizer (`DeviceVoice`).
- `car_ports.dart`: live `CarPorts` (device voice/recognizer, dictation, outbox, chat).
- `car_mode_screen.dart`: the call UI and glue. Reuses `SessionsController` and
  one `ChatController` bound to the current split. Keeps the screen on
  (wakelock) only while open.

Sends go only through `sl.get<Outbox>().enqueue(...)`, to the pane the chat
reads (not the feed's). Cancelled or unsent dictated text goes to the chat
composer if it is open underneath, else to the session's draft.

## Safety rules (the person is driving; when in doubt, do nothing and ask again)

- Spoken options (`option_parser.dart`): a number counts only when it is the
  whole answer ("dos", "2") or follows "opción" / "número" / "la" / "el";
  ordinals only after such a word ("la segunda"); `una` never. A "no"-word
  anywhere can only pick a "No" option; "espera", "para", "alto", "detente",
  "cancela", "ninguna"… anywhere pick nothing. Two numbers, yes + no, or
  anything else → null → ask again. Table-tested (`test/car/option_parser_test.dart`).
- A command counts only as the whole utterance and only where it applies: a
  bare "no" / "sí" is a message in an open turn, a cancel / confirm during a
  read-back.
- A countdown (send or option) starts only after its read-back was heard to the
  end. TTS failed, cut by Callar or focus lost → no timer: big "Enviar" /
  "Confirmar" and "Cancelar" buttons.
- A spoken answer is bound to the session, pane and prompt signature it was
  chosen for; if any changed, nothing is sent and the new prompt is read. The
  live port checks the chat again right before sending the keys.
- Speech answers a prompt only if that prompt was on screen AND read aloud to
  the end before listening started. Otherwise the dictation is a normal message,
  and a normal message is never sent while a prompt is open: it becomes the
  draft and the prompt is read. A prompt appearing during the send countdown
  does the same.
- No automatic split switch while listening, transcribing, confirming,
  choosing or answering.
- A split left in silence is not visited again on its own until its state
  changes (no ping-pong between unanswered prompts).
- Nothing can listen (no recognizer, no permission, no key): it says why once
  and waits for a tap; no loop.
- App hidden: polls stop, speech stops, recording stops (still transcribed),
  countdowns stop. Back: polls resume and pending replies / prompts are read;
  in a call a pending read-back is said again and counts down again.
- Automatic rounds drop recordings with no detected speech without
  transcribing (silence never costs a transcription).
- Spoken answers, commands and re-asks release the dictated text (never a
  draft); a new dictation never leaves the previous recording orphaned.
- The send result says the truth: "Hay un mensaje anterior sin resolver; ábrelo
  en el chat" when the session's queue is blocked, "Hay un permiso abierto: se
  enviará cuando lo respondas" for `prompt_open`.
