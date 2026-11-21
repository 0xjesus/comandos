# notifications

"Avisos en segundo plano". Off until the person says yes (asked once after
pairing; switch in Ajustes).

- `notice.dart`: `/notices` and `/notices/watch` models (pure Dart).
- `notice_alerts.dart`: notice → notification mapping (pure, unit-tested).
- `notice_watcher.dart`: the long-poll loop and its states (pure, unit-tested).
- `notifications_controller.dart`: opt-in, permission, keep-alive service,
  watcher, taps; implements `NotificationsHook`.
- `notifications_prompt.dart`: the one-time question and the Ajustes rows.

Android side: `android/app/src/main/kotlin/.../NativeBridge.kt` (channel
`comandos/native`), `WatchService.kt`, `Notifier.kt`, `EngineHolder.kt`,
`BootReceiver.kt`. Design and memory trade-off: ARCHITECTURE.md → Notifications.
