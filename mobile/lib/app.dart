import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'app_navigator.dart';
import 'core/bootstrap.dart';
import 'core/connection.dart';
import 'core/error_reporter.dart';
import 'core/locator.dart';
import 'core/outbox/drafts.dart';
import 'core/outbox/outbox.dart';
import 'core/polling/app_visibility.dart';
import 'features/notifications/notifications_hook.dart';
import 'ui/theme.dart';
import 'ui/widgets/error_boundary_banner.dart';

class ComandosApp extends StatefulWidget {
  const ComandosApp({super.key});

  @override
  State<ComandosApp> createState() => _ComandosAppState();
}

class _ComandosAppState extends State<ComandosApp> {
  AppLifecycleListener? _lifecycle;
  Timer? _backgroundGrace;

  /// After going to background the outbox may keep retrying this long, so a
  /// message sent right before switching apps still goes out. Then all
  /// timers stop until the app resumes.
  static const Duration backgroundGrace = Duration(seconds: 60);

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onStateChange: _onLifecycle);
    // The engine may have been running without UI (watch service): sync
    // visibility with the state the activity already reported.
    final AppLifecycleState? state = WidgetsBinding.instance.lifecycleState;
    if (state != null) _onLifecycle(state);
  }

  @override
  void dispose() {
    _lifecycle?.dispose();
    _backgroundGrace?.cancel();
    super.dispose();
  }

  void _onLifecycle(AppLifecycleState state) {
    // `inactive` (notification shade, app switcher animation) still counts
    // as visible; polling stops on hidden/paused.
    final bool resumed =
        state == AppLifecycleState.resumed || state == AppLifecycleState.inactive;
    final AppVisibility visibility = sl.get<AppVisibility>();
    if (visibility.value == resumed) return;
    visibility.value = resumed;
    final Outbox? outbox = sl.maybe<Outbox>();
    if (resumed) {
      _backgroundGrace?.cancel();
      _backgroundGrace = null;
      outbox?.setActive(true);
    } else {
      unawaited(sl.maybe<DraftStore>()?.flush());
      _backgroundGrace?.cancel();
      _backgroundGrace = Timer(backgroundGrace, () {
        _backgroundGrace = null;
        outbox?.setActive(false);
      });
    }
    sl.maybe<NotificationsHook>()?.onForegroundChanged(resumed);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'ComandOS',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      darkTheme: buildTheme(),
      themeMode: ThemeMode.dark,
      locale: const Locale('es'),
      supportedLocales: const <Locale>[Locale('es')],
      localizationsDelegates: GlobalMaterialLocalizations.delegates,
      navigatorKey: AppNavigator.key,
      navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
      builder: (BuildContext context, Widget? child) =>
          ErrorBoundaryBanner(child: child ?? const SizedBox.shrink()),
      home: const _BootScreen(),
    );
  }
}

/// First frame: a plain background (same colour as the launch window), then
/// local-only boot (keystore + two small files), then pairing or sessions.
class _BootScreen extends StatefulWidget {
  const _BootScreen();

  @override
  State<_BootScreen> createState() => _BootScreenState();
}

class _BootScreenState extends State<_BootScreen> {
  String? _failure;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _boot());
  }

  Future<void> _boot() async {
    try {
      await bootstrap();
    } catch (e, st) {
      sl.get<ErrorReporter>().report(e, st);
      if (mounted) {
        setState(() => _failure = 'No pude abrir los datos de la app. Ciérrala y ábrela de nuevo.');
      }
      return;
    }
    if (sl.get<Connection>().isPaired) {
      AppNavigator.showSessions();
    } else {
      AppNavigator.showPairing();
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: _failure == null
            ? const SizedBox.shrink()
            : Padding(
                padding: const EdgeInsets.all(24),
                child: Text(_failure!, textAlign: TextAlign.center),
              ),
      ),
    );
  }
}
