# car

"Modo auto": hands-free screen for a phone on a car mount. Not Android Auto.
Open it with `CarMode.open(context, session:, pane:)` (sessions app bar, chat
menu, settings).

- `car_machine.dart`: the whole flow as an explicit state machine (pure Dart,
  injected clock/timers and `CarPorts`): Hablar → dictation → "Envío: …" +
  3 s countdown (Cancelar / Enviar ya) → outbox → "Enviado" / "Sin conexión…";
  completed replies read once; Conversación continua; permission prompts by
  button or by voice (confirmed with a cancellable countdown; ambiguous → ask
  again); announcements of other sessions; Detener (Escape) and Callar.
- `option_parser.dart`: spoken answers ("dos", "opción uno", "sí") → option.
- `car_ports.dart`: live `CarPorts` (dictation, speech, outbox, chat).
- `car_mode_screen.dart`: UI and glue. Reuses `SessionsController` and one
  `ChatController` (the screens below are covered, so their polling stops).
  Keeps the screen on (wakelock) only while open.

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
- A countdown (send or option) starts only after its read-back was heard to the
  end. TTS failed, cut by Callar, focus lost, or the app hidden → no timer: big
  "Enviar" / "Confirmar" and "Cancelar" buttons.
- A spoken answer is bound to the session, pane and prompt signature it was
  chosen for; if any changed, nothing is sent and the new prompt is read. The
  live port checks the chat again right before sending the keys.
- Speech answers a prompt only if that prompt was on screen AND read aloud to
  the end before listening started. Otherwise the dictation is a normal message,
  and a normal message is never sent while a prompt is open: it becomes the
  draft and the prompt is read. A prompt appearing during the send countdown
  does the same.
- No automatic session switch while listening, transcribing, confirming,
  choosing or answering.
- App hidden: polls stop, speech stops, recording stops (still transcribed; the
  text then waits for a tap), countdowns stop. Back: polls resume and pending
  replies / prompts are read.
- Automatic rounds (continuous conversation, re-asks) drop recordings with no
  detected speech without transcribing; two empty rounds stop the loop until
  Hablar is tapped.
- Spoken answers and re-asks release the dictated text (never a draft); a new
  dictation never leaves the previous recording orphaned.
- The send result says the truth: "Hay un mensaje anterior sin resolver; ábrelo
  en el chat" when the session's queue is blocked, "Hay un permiso abierto: se
  enviará cuando lo respondas" for `prompt_open`.
