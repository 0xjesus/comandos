import 'package:path_provider/path_provider.dart';

import '../../app_navigator.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/outbox/outbox.dart';
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
    connectivity: net?.online,
    foreground: sl.maybe<AppVisibility>()?.value ?? true,
  );
  sl
    ..register<NotificationsHook>(c)
    ..register<NotificationsController>(c);
}
