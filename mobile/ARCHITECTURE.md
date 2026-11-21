# ComandOS Android: architecture

Native Flutter client for the ComandOS dashboard, reached over Tailscale HTTPS
(`https://<host>.ts.net`) with the dashboard token in `X-Comandos-Token`.
Android only, minSdk 26. No Firebase, no analytics, no code generation.

## Layout

```
lib/
  main.dart              error boundary + runApp; no I/O. Also `watchMain` (headless
                         entry for the watch service, see Notifications)
  app.dart               MaterialApp, lifecycle → AppVisibility, boot screen
  app_navigator.dart     AppNavigator.openChat / showSessions / showPairing (no context needed)
  core/
    locator.dart         `sl`: tiny service locator (register / get / maybe)
    bootstrap.dart       builds and registers every service after the first frame
    connection.dart      current pairing + ComandosApi; pair() / unpair()
    error_reporter.dart  global error banner sink
    api/                 ComandosApi (one keep-alive client, deadlines, cancel, typed
                         errors); api_response.dart is pure Dart (used by tool/)
    platform/            NativeChannel: the app's own channel `comandos/native`
                         (notifications, keep-alive service, taps, QR scanner)
    config/              PairedServer, pairing URL/QR parser, secure credential store
    models/              session feed, transcript, ask prompt (lenient JSON readers)
    outbox/              THE outbox: item, state machine, transport, file store, drafts
    polling/             Poller (one-shot timers), AppVisibility, ConnectivityWatch
    util/                clock/timer injection, AtomicJsonFile (tmp + fsync + rename)
  features/
    pairing/             QR scan (native ScanActivity) / paste link, check with GET /session-feed
    sessions/            home list: sections, controller (2.5 s poll), rows
    chat/                transcript poll (2 s), markdown, ask card, composer, outbox bubbles
    settings/            re-pair, unpair, background notices, pending messages
    voice/               OpenAI dictation + read-aloud, Ajustes › Voz (see voice/README.md)
    car/                 Modo auto: state machine + screen (see car/README.md)
    notifications/       "Avisos en segundo plano": notice models, watcher, mapping,
                         controller (implements NotificationsHook), opt-in UI
  ui/                    theme (Noche Órbita), formatting, shared widgets
tool/live_check.dart     read-only check of the parsers against a real server
android/app/src/main/kotlin/app/comandos/comandos_mobile/
  MainActivity.kt        engine from EngineHolder; kept alive while the watcher runs
  EngineHolder.kt        the single FlutterEngine (activity and watch service share it)
  NativeBridge.kt        channel `comandos/native`
  WatchService.kt        foreground service (remoteMessaging) that keeps the process alive
  Notifier.kt            channels + per-session notifications (framework API, no AndroidX)
  BootReceiver.kt        restarts the watcher after reboot / app update, if it was on
  ScanActivity.kt        QR scanner: camera preview + ZXing QR reader, offline
```

## Rules every screen follows

- Services come from `sl`; widgets never build services or `http` clients.
- Network only through `sl.get<Connection>().api` (`ComandosApi`). HTTP statuses
  never throw from `request()`; `requestOk()` and typed routes throw `ApiException`
  subtypes whose `userMessage` is Spanish copy ready to show.
- Never log the token. `PairedServer.toString()` redacts it.
- Polling uses `Poller` (single in-flight, one-shot timer) and runs only while the
  screen is visible: `AppVisibility` (app resumed/inactive) AND not covered by
  another route (`RouteAware` via `AppNavigator.routes`).
- Lists: `ListView.builder` / slivers, stable keys, reuse identical widget instances
  for unchanged rows (signature cache in sessions, `Expando` per message in chat).
- A poll that brings nothing new notifies nobody: no widget rebuilds and no frame
  is scheduled (`SessionsController` compares rows + "hace N min" texts;
  `ChatController` only notifies on real changes). Tested in
  `test/features/rebuild_and_lifecycle_test.dart` with `debugOnRebuildDirtyWidget`.

## The outbox (`core/outbox/`)

Sending NEVER goes straight to HTTP. Everything that sends text calls

```dart
await sl.get<Outbox>().enqueue(session: s, pane: p, text: t, transcriptBaseline: n);
```

and clears its input only after that future completes (it completes once the item
is durable in `outbox.json`). It throws `OutboxRejection` (Spanish `message`) for
empty / >20000 chars / unpaired / disk failure; keep the text in that case.
Control characters the desktop refuses (422) are stripped first; `\n` and `\t`
stay, CR/CRLF become `\n` (as the desktop does).

States: `queued → sending → delivered | failed(failCode) | uncertain`.

- Delivery: `POST /session/message {clientMessageId, session, pane, text, submit:true}`.
  The id is generated once; automatic retries reuse it. After an ambiguous failure
  (timeout, 5xx, 202) the next attempt first asks `GET /session/message?clientMessageId=`.
- Retry on network/timeout/5xx/202: 1, 2, 5, 10, 30 s, then every 60 s.
  409/429: 0.7, 1, 2, 3, then 5 s. `nudge()` (connectivity back, app resumed) retries now.
- 409 `prompt_open` (a permission prompt is open in that pane; typing there could
  answer it): no receipt is kept, so the item stays `queued` with the SAME id and
  `waitCode: prompt_open`, retried every 3 s ("Esperando a que respondas el
  permiso"); it goes out by itself once the prompt is answered.
- No automatic retry on 404 `pane_gone`/`session_gone`, 400/422, 401/403: `failed`.
  The person picks Reintentar, Editar, or Descartar. Reintentar KEEPS the id (the
  desktop keeps no receipt for a rejection; if the message did arrive after all it
  answers `duplicate`) and targets the current pane (after `pane_gone` a stale pane
  is dropped). Only "Reenviar" on an `uncertain` item uses a fresh id.
- Editar: the text goes to the composer. If later messages of that session are
  waiting, the item stays in place with `editing: true`, holding them ("N mensajes
  en espera" over the composer) until the composer's next send replaces it
  (`enqueue(replaces:)`, same queue position) or the person taps "Descartar
  edición". Without later messages it is simply removed.
- Strict FIFO per session: a queued/sending/failed/uncertain/editing head blocks
  the rest of its session. Other sessions are independent.
- The chat composer cannot send while the session shows a permission prompt
  ("Responde primero el permiso de arriba"); typing and the draft still work.
- Old server (404 without `{code}` on the route): legacy `POST /send` once per item,
  persisted as `legacyTried` before the request. Network error/5xx there →
  `uncertain` ("No sé si llegó"), never resent automatically; the outbox checks
  `/chat/transcript` a few times (needs a baseline) and otherwise leaves it to the
  person (Sí llegó / Reenviar / Editar / Descartar). Text >4000 chars is refused
  for the legacy route (it would truncate).
- Items are bound to the host they were written for; never sent elsewhere.
- `transcriptBaseline` = how many user messages with that text the chat shows now;
  the chat feeds `observeTranscript()` so delivered bubbles disappear once the
  transcript shows them (or after 2 min).
- Pure Dart, injectable clock/timer/transport/store; see `test/core/outbox_test.dart`.

Drafts: `DraftStore` keeps per-session composer text (debounced 400 ms, flushed on
background) in `drafts.json`.

## Extension points for the next agents

| Feature | Implement | Register in | Used by |
|---|---|---|---|
| Dictation | `VoiceInput.dictate()` → `String?` (`features/voice/voice.dart`) | `registerVoice()` | composer mic + strip (`chat_voice.dart`; hidden without an OpenAI key) |
| Read aloud | `Speaker.speak/stop` (`features/voice/voice.dart`) | `registerVoice()` | message long-press → "Leer en voz alta"; chat menu "Leer respuestas nuevas" |
| Modo auto | a `CarModeBuilder` (`features/car/car_mode.dart`) | `registerCarMode()` | sessions app bar, chat menu, settings → `CarMode.open(context, session:, pane:)` |
| Notifications | done: `NotificationsController` (`features/notifications/`) | `registerNotifications()` | boot (if paired), pairing, unpair, foreground changes, Ajustes |

- Open a chat from anywhere (notification tap): `AppNavigator.openChat(session, pane: pane)`.
- Car mode sends through `Outbox.enqueue` like the composer; it may read the
  transcript with `ChatController` or `sl.get<Connection>().api!.transcript(...)`.
- Manifest permissions (each justified in a comment there): INTERNET,
  ACCESS_NETWORK_STATE, CAMERA, RECORD_AUDIO, POST_NOTIFICATIONS,
  FOREGROUND_SERVICE(_REMOTE_MESSAGING), RECEIVE_BOOT_COMPLETED. WAKE_LOCK was
  dropped: wakelock_plus uses FLAG_KEEP_SCREEN_ON, which needs no permission.
- Backups: `allowBackup="false"` plus `dataExtractionRules` (Android 12+) and
  `fullBackupContent` that exclude every domain from cloud backup and from
  device-to-device transfer (`res/xml/`). Tested in `test/android/`.
- `AtomicJsonFile.read` treats bad JSON, invalid UTF-8 and read errors alike: the
  file is moved aside (`.corrupt-<ms>`) and the app starts empty.
  Every plugin adds Java/Kotlin (sometimes native) code to the APK: ask before
  adding one; prefer a few lines in `NativeBridge.kt`.
- Background: after the app is hidden the outbox keeps retrying for 60 s, then
  `Outbox.setActive(false)` stops its timer. While the notice watcher runs, every
  round that reaches the desktop calls `setActive(true)` if messages are pending,
  so they keep going out in the background.

## Notifications ("Avisos en segundo plano")

Off by default. Asked once right after pairing (one question, "Ahora no" /
"Sí, avisarme"), switch in Ajustes. On "Sí", Android 13+ shows its permission
dialog right after the in-app explanation. Denied → it stays off and says how to
allow it in Android's settings.

**Mechanism.** Web Push is off on the server, so the phone long-polls:
`GET /notices/watch?rev=<rev>&wait=25` returns when anything changes, then
`GET /notices?after=<n>&limit=50` page by page (`NoticeWatcher`). The cursor
(`rev`, `after`) and settings live in `notifications.json` (AtomicJsonFile):

- First run for a desktop: only reads the current position (`wait=0` →
  `latest`); old notices are never replayed. Long absence: only the last 100
  events. Server history went backwards: the cursor follows, no replay.
- Network loss / 5xx / timeout: backoff 2, 5, 10, 30, 60 s, then every 60 s;
  connectivity back or app resumed → retry now. Progress is saved page by page.
- Server restart: a network error, a retry, the sequence continues.
- 401/403: the watcher stops, the service stops, `Connection.tokenRejected` is
  set (the list shows the banner) and, if the app is hidden, one notification
  says "vuelve a vincular". Pairing again resumes.
- A server that answers an unchanged watch at once (ignores `wait`) gets a 5 s
  pause, so the loop cannot spin. While the app is hidden, rounds start at least
  4 s apart even when the revision keeps changing (many busy agents).
- If Android restarts `WatchService` but refuses `startForeground` (Android 12+
  background limits), the service records it (`takeLost`); the next app open
  shows "Los avisos en segundo plano se detuvieron; ábrelos de nuevo en Ajustes"
  and starts the service again.

**Notifications** (`notice_alerts.dart`, pure): `needsHuman` → "<proyecto> te
espera" + excerpt on the high-importance channel; done → "<proyecto> terminó"
(cancelled: "se detuvo"), error → "<proyecto> falló" on the default channel
(done can be turned off in Ajustes; errors and requests always notify). One
notification per session (tag `s:<session>`): a newer event replaces it. Skipped:
news/usage/focus/info, events already read on the desktop, a request no longer
pending (answered elsewhere), and the session whose chat is on screen
(`AppNavigator.visibleChat`). A "te espera" disappears when its request stops
being pending; opening a chat clears that session's notification. Tap →
`AppNavigator.openChat(session, pane:)`, once the sessions list is up.

**Process model (the memory decision).** flutter_foreground_task was evaluated
and dropped: its service always constructs a second `FlutterEngine` (verified in
its `ForegroundTask.kt`, even with no Dart callback), i.e. a second engine shell
with its own UI/raster/IO threads, a second isolate group heap and every plugin
registered again. Estimated 10–30 MB PSS extra while running, on top of the
app's own engine whenever the UI is alive (not measured: no device).

Chosen design, one engine only:

1. The long-poll runs in the app's main isolate, sharing the HTTP client.
2. `WatchService` (our ~100-line Kotlin service, type `remoteMessaging`, no
   6-hour limit, may start at boot) only keeps the process in foreground-service
   priority and shows the quiet "ComandOS · atento a tus sesiones" notification
   (IMPORTANCE_MIN channel: no sound, no badge, collapsed).
3. `MainActivity` gets its engine from `EngineHolder` and does not destroy it
   while the service runs, so swiping the app away keeps the watcher alive; with
   the service off the engine dies with the activity as before.
4. If Android restarts the sticky service, or at boot / after an update
   (`BootReceiver`), with no engine alive, `EngineHolder` starts the same single
   engine headless on `watchMain` (services + watcher, no widgets). If an
   activity attaches later, `watchMain` calls `runApp` on that engine.

Cost: while the UI is open, zero extra (no second engine, one more idle HTTP
connection). In the background, the process stays at what a backgrounded Flutter
app already uses (estimate 40–70 MB PSS, surfaces released; unmeasured) instead
of being cached/killed; Dart work is one pending socket read plus a few KB per
change. Measure on a device with
`adb shell dumpsys meminfo app.comandos.comandos_mobile` (app open, app in
background, app swiped away). Battery: no timers while waiting; no wake lock
(Doze may delay delivery; Ajustes links to the app's battery setting).

Notifications, channels, the service and taps are framework API calls in
`Notifier.kt` / `NativeBridge.kt` (no AndroidX, no Gson): this replaced
flutter_local_notifications and flutter_foreground_task.

## QR scanner

`QrScanScreen` (same public Dart API: push it, it pops with the scanned text or
null) launches `ScanActivity`: `android.hardware.Camera` preview + ZXing's
`QRCodeReader` on a background thread, centre square, every other frame
inverted (light-on-dark codes). No Play Services, no model download, offline.
ZXing adds ~31 KB of dex after R8 (ML Kit was ~5.8 MB). Camera permission is
asked by the activity; denied → the screen says so and offers Android settings.

## Startup and runtime

- Before the first frame: binding, two cheap services, error handlers, image
  cache limits, system bars. The first frame is the dark boot screen; pairing
  state, outbox and drafts load right after it, then the list (or pairing) shows.
  Notifications read their settings asynchronously after that.
- Android 12+ splash: `values-v31/styles.xml` (background #0A0D13, the launcher
  glyph); older: dark `launch_background`. No white flash.
- Image cache capped (32 entries / 4 MB): the app shows no images.
- JSON bodies under 384 KB are decoded inline (spawning an isolate for the
  common 8–80 KB bodies cost more than decoding them).
- Polls that change nothing rebuild nothing (see Rules). A transcript answer
  with no agent and no messages no longer blanks an open chat (transient gap).
- Opening/closing a chat 25 times leaves no listener, timer or visible-chat
  state behind (`test/features/rebuild_and_lifecycle_test.dart`).
- In the background only the optional watcher (and the outbox while it has
  pending messages and the watcher runs) do network work.

## APK size

| arm64-v8a release | before | after |
|---|---|---|
| APK | 25.7 MB | 17.8 MB |
| classes.dex | 4.36 MB | 1.70 MB |
| libapp.so (Dart) | 5.83 MB | 5.44 MB |
| ML Kit (`libbarhopper_v3.so` + models) | 5.83 MB | 0 |
| resources.arsc | 360 KB | 18 KB |

Savings: ML Kit/CameraX/Play Services scanner → ZXing; notifications and the
service in our own Kotlin (dropped flutter_local_notifications + its Gson
keep-all rule, flutter_foreground_task); dropped the unused shared_preferences
(DataStore + its native lib); `localeFilters` es/en; no PNG mipmaps (vector
adaptive icon); `--obfuscate --split-debug-info`. What is left: the Flutter
engine (11.1 MB), Dart code, and media3/ExoPlayer from just_audio (~0.8 MB of
the remaining dex; voice).

## Build

```
export PATH=$HOME/flutter_global_3_27_4/flutter/bin:$PATH
flutter analyze && flutter test
flutter build apk --release --split-per-abi --obfuscate --split-debug-info=build/symbols --tree-shake-icons
```

Output: `build/app/outputs/flutter-apk/app-arm64-v8a-release.apk` (phones),
plus armeabi-v7a and x86_64. Keep `build/symbols/` of each release to decode
stack traces (`flutter symbolize`). Release is signed with the local debug
keystore (personal sideload), R8 + resource shrinking on
(`android/app/proguard-rules.pro`). Size breakdown:
`flutter build apk --release --target-platform android-arm64 --analyze-size`.

## Live check against a real server

`dart run tool/live_check.dart` (on the desktop running ComandOS, loopback,
no token) calls only read routes (`/session-feed`, `/chat/transcript`,
`/notices`, `/notices/watch`, `GET /session/message` with a random id) and
parses each answer with the app's own models. It prints shapes and counts,
never content. Run it after server changes.
