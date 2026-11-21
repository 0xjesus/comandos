import 'package:flutter/services.dart';

/// Result of [NativeChannel.scanQr].
sealed class QrScanResult {
  const QrScanResult();
}

class QrScanned extends QrScanResult {
  const QrScanned(this.text);
  final String text;
}

/// `denied` (no camera permission), `failed` (camera unavailable) or
/// `cancelled` (the person went back).
class QrScanFailed extends QrScanResult {
  const QrScanFailed(this.reason);
  final String reason;
}

/// One local notification, ready to show. Built by `notice_alerts.dart`.
class LocalAlert {
  const LocalAlert({
    required this.tag,
    required this.urgent,
    required this.title,
    required this.body,
    this.session,
    this.pane,
    this.whenMs = 0,
    this.eventId,
  });

  /// Identity: a newer alert with the same tag replaces the old one.
  final String tag;

  /// High importance (heads-up, sound) when true.
  final bool urgent;
  final String title;
  final String body;

  /// Chat opened by a tap.
  final String? session;
  final String? pane;
  final int whenMs;

  /// Server event behind it (not sent to Android).
  final String? eventId;
}

/// What the notifications feature needs from Android. Faked in tests.
abstract class NotificationPlatform {
  /// The app may post notifications (permission + not blocked in settings).
  Future<bool> notificationsAllowed();

  /// Android 13+ permission dialog. True when allowed afterwards.
  Future<bool> requestNotifications();

  /// Starts the keep-alive foreground service. False when Android refused.
  Future<bool> startKeepAlive();
  Future<void> stopKeepAlive();
  Future<void> show(LocalAlert alert);
  Future<void> cancel(String tag);
  Future<void> cancelAll();

  /// The chat a notification tap asked for (taken once), or null.
  Future<({String? session, String? pane})?> takeTap();

  /// Called when a notification was tapped while the app runs.
  set onTap(void Function()? callback);

  /// Opens Android's settings for this app ([notifications]: its
  /// notification page).
  Future<void> openSettings({bool notifications = false});
}

/// The app's own platform channel (`android/.../NativeBridge.kt`). Every call
/// degrades to a harmless default when the platform side is missing (tests).
class NativeChannel implements NotificationPlatform {
  NativeChannel._() {
    _channel.setMethodCallHandler((MethodCall call) async {
      if (call.method == 'tap') _onTap?.call();
      return null;
    });
  }

  static final NativeChannel instance = NativeChannel._();
  static const MethodChannel _channel = MethodChannel('comandos/native');

  void Function()? _onTap;

  @override
  set onTap(void Function()? callback) => _onTap = callback;

  Future<T?> _call<T>(String method, [Map<String, Object?>? args]) async {
    try {
      return await _channel.invokeMethod<T>(method, args);
    } on MissingPluginException {
      return null;
    } on PlatformException {
      return null;
    }
  }

  /// Opens the native scanner (ScanActivity) and waits for its answer.
  Future<QrScanResult> scanQr() async {
    final Map<Object?, Object?>? r = await _call<Map<Object?, Object?>>('scanQr');
    final Object? text = r?['text'];
    if (text is String && text.isNotEmpty) return QrScanned(text);
    final Object? error = r?['error'];
    return QrScanFailed(error is String ? error : 'failed');
  }

  @override
  Future<bool> notificationsAllowed() async => await _call<bool>('notificationsAllowed') ?? false;

  @override
  Future<bool> requestNotifications() async => await _call<bool>('requestNotifications') ?? false;

  @override
  Future<bool> startKeepAlive() async => await _call<bool>('watchStart') ?? false;

  @override
  Future<void> stopKeepAlive() => _call<void>('watchStop');

  @override
  Future<void> show(LocalAlert a) => _call<void>('notify', <String, Object?>{
        'tag': a.tag,
        'urgent': a.urgent,
        'title': a.title,
        'body': a.body,
        'session': a.session,
        'pane': a.pane,
        'when': a.whenMs,
      });

  @override
  Future<void> cancel(String tag) => _call<void>('cancel', <String, Object?>{'tag': tag});

  @override
  Future<void> cancelAll() => _call<void>('cancelAll');

  @override
  Future<({String? session, String? pane})?> takeTap() async {
    final Map<Object?, Object?>? r = await _call<Map<Object?, Object?>>('takeTap');
    if (r == null) return null;
    final Object? s = r['session'];
    final Object? p = r['pane'];
    return (session: s is String ? s : null, pane: p is String ? p : null);
  }

  @override
  Future<void> openSettings({bool notifications = false}) =>
      _call<void>('openSettings', <String, Object?>{'notifications': notifications});
}
