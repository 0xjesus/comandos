# notifications

"Avisos en segundo plano". Off until the person says yes (asked once after
pairing; switch in Ajustes).

- `notice.dart`: `/notices` and `/notices/watch` models (pure Dart).
- `notice_alerts.dart`: notice → notification mapping (pure, unit-tested).
- `notice_watcher.dart`: the long-poll loop and its states (pure, unit-tested).
- `notifications_controller.dart`: opt-in, permission, keep-alive service,
  watcher, taps; implements `NotificationsHook`.
- `notifications_prompt.dart`: the one-time question and the Ajustes rows.
- `car_text.dart`: what Android Auto reads (short, no code, paths or secrets;
  a permission reads its command and options) and what a car reply means
  (pure, unit-tested).
- `car_settings.dart`: «En el coche · Android Auto» in Ajustes.

Android Auto: every session notification is a `MessagingStyle` conversation
with reply and mark-as-read actions (`Notifier.kt`, `CarReplyReceiver.kt`,
`res/xml/automotive_app_desc.xml`). The car's assistant dictates the reply and
asks «¿Lo envío?»; Dart then re-reads the pane: a permission is answered only
if it is the very prompt the car read, a message goes through the outbox and
never onto an open menu (the server also refuses it: `prompt_open`). Not from
Play: Android Auto needs «Fuentes desconocidas» in its developer settings.

Android side: `android/app/src/main/kotlin/.../NativeBridge.kt` (channel
`comandos/native`), `WatchService.kt`, `Notifier.kt`, `CarReplyReceiver.kt`, `EngineHolder.kt`,
`BootReceiver.kt`. Design and memory trade-off: ARCHITECTURE.md → Notifications.
