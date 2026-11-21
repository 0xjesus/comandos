import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'app.dart';
import 'core/bootstrap.dart';
import 'core/error_reporter.dart';
import 'core/locator.dart';
import 'core/polling/app_visibility.dart';
import 'ui/widgets/error_boundary_banner.dart';

/// Nothing heavy here: no I/O, no network. Services that touch disk are
/// built after the first frame (see `_BootScreen` in app.dart).
void main() {
  _initEngine();
  _systemBars();
  runApp(const ComandosApp());
}

/// Entry point of the engine the watch service starts when Android restarts
/// it (or at boot) with no activity: services and the notice watcher only,
/// no widgets. When an activity attaches to this engine later, the UI starts
/// on it (one engine, never two). Called by `EngineHolder.kt`.
@pragma('vm:entry-point')
Future<void> watchMain() async {
  _initEngine();
  sl.get<AppVisibility>().value = false;
  try {
    await bootstrap();
  } catch (e, st) {
    sl.get<ErrorReporter>().report(e, st);
  }
  late final AppLifecycleListener listener;
  listener = AppLifecycleListener(onStateChange: (AppLifecycleState s) {
    if (s != AppLifecycleState.resumed && s != AppLifecycleState.inactive) return;
    listener.dispose();
    _systemBars();
    runApp(const ComandosApp());
  });
}

void _initEngine() {
  WidgetsFlutterBinding.ensureInitialized();
  registerEarlyServices();
  final ErrorReporter reporter = sl.get<ErrorReporter>();

  // Global error boundary: report, never crash, never a red screen in release.
  FlutterError.onError = (FlutterErrorDetails details) {
    if (kDebugMode) FlutterError.presentError(details);
    reporter.report(details.exception, details.stack);
  };
  PlatformDispatcher.instance.onError = (Object error, StackTrace stack) {
    reporter.report(error, stack);
    return true;
  };
  if (!kDebugMode) ErrorWidget.builder = quietErrorWidget;

  // The app shows no images (icons are a font): keep the decoded-image cache
  // tiny instead of Flutter's default 100 MB / 1000 entries.
  PaintingBinding.instance.imageCache
    ..maximumSize = 32
    ..maximumSizeBytes = 4 << 20;
}

void _systemBars() {
  SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge);
  SystemChrome.setSystemUIOverlayStyle(const SystemUiOverlayStyle(
    statusBarColor: Colors.transparent,
    systemNavigationBarColor: Colors.transparent,
    statusBarIconBrightness: Brightness.light,
    systemNavigationBarIconBrightness: Brightness.light,
  ));
}
