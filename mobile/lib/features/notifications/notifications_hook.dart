import 'dart:async';

import 'package:path_provider/path_provider.dart';

import '../../app_navigator.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../core/platform/native_channel.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../../core/util/atomic_json_file.dart';
import 'notifications_controller.dart';
import 'notifications_prompt.dart';

/// Lifecycle hook for local notifications / the background watcher.
///
/// The app calls these at the right moments. To open a chat from a
/// notification tap the implementation calls
/// `AppNavigator.openChat(session, pane: pane)`.
abstract class NotificationsHook {
  /// The phone is (or just got) paired. Also called at boot when paired.
  Future<void> onPaired();

  /// The person unpaired: stop everything, drop any cached data.
  Future<void> onUnpaired();

  /// App moved to foreground (true) or background (false).
  void onForegroundChanged(bool foreground);
}

/// Does nothing (tests that do not exercise notifications).
class NoNotifications implements NotificationsHook {
  @override
  Future<void> onPaired() async {}

  @override
  Future<void> onUnpaired() async {}

  @override
  void onForegroundChanged(bool foreground) {}
}

/// Registers [NotificationsController] as the [NotificationsHook] (and under
/// its own type for the settings screen). Needs `Connection` registered.
/// Builds nothing heavy: settings are read on the first `onPaired`.
void registerNotifications() {
  final Connection? connection = sl.maybe<Connection>();
  if (connection == null) {
    sl.register<NotificationsHook>(NoNotifications());
    return;
  }
  sl.maybe<NotificationsController>()?.dispose();
  final ConnectivityWatch? net = sl.maybe<ConnectivityWatch>();
  final NotificationsController c = NotificationsController(
    platform: NativeChannel.instance,
    connection: connection,
    file: () async =>
        AtomicJsonFile('${(await getApplicationSupportDirectory()).path}/notifications.json'),
    visibleChat: AppNavigator.visibleChat,
    uiReady: AppNavigator.homeShown,
    openChat: (String session, String? pane) => AppNavigator.openChat(session, pane: pane),
    askUser: askBackgroundNotices,
    onMessage: showNoticeMessage,
    onReachable: () {
      // With the watcher on, undelivered messages keep going out in the
      // background too (the outbox otherwise pauses 60 s after hiding).
      final Outbox? outbox = sl.maybe<Outbox>();
      if (outbox != null && outbox.pendingCount > 0) outbox.setActive(true);
    },
    carSend: carSendThroughOutbox,
    connectivity: net?.online,
    foreground: sl.maybe<AppVisibility>()?.value ?? true,
  );
  sl
    ..register<NotificationsHook>(c)
    ..register<NotificationsController>(c);
}

/// A message dictated in the car goes through the outbox (durable, never
/// typed twice) and the car hears how it went: delivered within a few
/// seconds, or kept to retry.
Future<String> carSendThroughOutbox(String session, String? pane, String text) async {
  final Outbox? outbox = sl.maybe<Outbox>();
  if (outbox == null) return 'No pude enviar el mensaje desde aquí.';
  final OutboxItem item;
  try {
    item = await outbox.enqueue(session: session, pane: pane, text: text);
  } on OutboxRejection catch (e) {
    return e.message;
  }
  outbox.setActive(true);
  final Completer<String> done = Completer<String>();
  void check() {
    final OutboxItem? now = outbox.byId(item.id);
    if (done.isCompleted) return;
    switch (now?.state) {
      case OutboxState.delivered:
        done.complete('Enviado.');
      case OutboxState.failed:
        done.complete('No se envió: ${outboxFailureMessage(now?.failCode)}');
      case OutboxState.uncertain:
        done.complete('Llegó a la sesión pero no se confirmó; revísala antes de repetirlo.');
      case null:
      case OutboxState.queued:
      case OutboxState.sending:
        break;
    }
  }

  outbox.addListener(check);
  check();
  try {
    return await done.future.timeout(
      const Duration(seconds: 4),
      onTimeout: () => 'Guardado; se envía en cuanto tu equipo responda.',
    );
  } finally {
    outbox.removeListener(check);
  }
}
