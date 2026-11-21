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

Sends go only through `sl.get<Outbox>().enqueue(...)`. Cancelled or unsent
dictated text goes to the chat composer if it is open underneath, else to the
session's draft.
