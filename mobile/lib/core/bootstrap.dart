import 'dart:async';

import 'package:path_provider/path_provider.dart';

import '../features/car/car_mode.dart';
import '../features/notifications/notifications_hook.dart';
import '../features/voice/voice.dart';
import 'connection.dart';
import 'config/credential_store.dart';
import 'error_reporter.dart';
import 'locator.dart';
import 'outbox/drafts.dart';
import 'outbox/message_transport.dart';
import 'outbox/outbox.dart';
import 'outbox/outbox_store.dart';
import 'polling/app_visibility.dart';
import 'polling/connectivity_watch.dart';

/// Builds and registers every service. Runs after the first frame (the boot
/// screen is already painted), does local disk/keystore reads only, never
/// network. Idempotent: a second call (the UI starting on an engine the
/// watch service already booted) returns at once.
Future<void> bootstrap() => _booting ??= _bootstrap().catchError((Object e, StackTrace st) {
      _booting = null; // a failed boot may be retried
      Error.throwWithStackTrace(e, st);
    });

Future<void>? _booting;

Future<void> _bootstrap() async {
  final String dir = (await getApplicationSupportDirectory()).path;

  final Connection connection = Connection(SecureCredentialStore());
  final Outbox outbox = Outbox(
    store: FileOutboxStore('$dir/outbox.json'),
    transport: ApiMessageTransport(() => connection.api),
  );
  final DraftStore drafts = DraftStore('$dir/drafts.json');

  await Future.wait(<Future<void>>[connection.load(), outbox.load(), drafts.load()]);

  outbox.setHost(connection.server?.displayHost);
  connection.addListener(() => outbox.setHost(connection.server?.displayHost));

  final ConnectivityWatch connectivity = ConnectivityWatch(onRegained: outbox.nudge)..start();

  sl
    ..register<Connection>(connection)
    ..register<Outbox>(outbox)
    ..register<DraftStore>(drafts)
    ..register<ConnectivityWatch>(connectivity);

  // Feature hooks. The voice, car and notifications work replaces these
  // registrations inside their own register functions; nothing else changes.
  registerVoice();
  registerCarMode();
  registerNotifications();
  if (connection.isPaired) unawaited(sl.get<NotificationsHook>().onPaired());
}

/// Services that must exist before the first frame (cheap, no I/O).
void registerEarlyServices() {
  sl
    ..register<ErrorReporter>(ErrorReporter())
    ..register<AppVisibility>(AppVisibility());
}
