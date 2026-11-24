import 'dart:async';

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
    this.speech,
    this.sender,
    this.askSig,
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

  /// What Android Auto reads aloud: the state first, then the reply made to
  /// be heard (`car_text.dart`). Null: the notification is not a
  /// conversation (no reply from the car).
  final String? speech;

  /// Who «says» it in the car (the session's name).
  final String? sender;

  /// Signature of the permission prompt [speech] reads, when it reads one:
  /// a car reply only answers that very prompt.
  final String? askSig;

  LocalAlert withSpeech(String speech, {String? askSig, String? pane}) => LocalAlert(
        tag: tag,
        urgent: urgent,
        title: title,
        body: body,
        session: session,
        pane: pane ?? this.pane,
        whenMs: whenMs,
        eventId: eventId,
        speech: speech,
        sender: sender,
        askSig: askSig,
      );
}

/// A reply dictated in the car (Android Auto) or typed in the notification.
class CarReply {
  const CarReply({required this.tag, required this.text, this.session, this.pane, this.askSig});
  final String tag;
  final String text;
  final String? session;
  final String? pane;
  final String? askSig;
}

/// What the notifications feature needs from Android. Faked in tests.
abstract class NotificationPlatform {
  /// The app may post notifications (permission + not blocked in settings).
  Future<bool> notificationsAllowed();

  /// Android 13+ permission dialog. True when allowed afterwards.
  Future<bool> requestNotifications();

  /// Starts the keep-alive foreground service. False when Android refused.
  Future<bool> startKeepAlive();

  /// True (once) when Android restarted the keep-alive service but did not
  /// let it run in the foreground, so background notices stopped.
  Future<bool> takeKeepAliveLost();
  Future<void> stopKeepAlive();
  Future<void> show(LocalAlert alert);
  Future<void> cancel(String tag);
  Future<void> cancelAll();

  /// The chat a notification tap asked for (taken once), or null.
  Future<({String? session, String? pane})?> takeTap();

  /// Called when a notification was tapped while the app runs.
  set onTap(void Function()? callback);

  /// Handles a reply from the car; returns what the conversation shows (and
  /// the car may read) as the outcome, in Spanish.
  set onCarReply(Future<String> Function(CarReply reply)? handler);

  /// The person marked a conversation as read in the car.
  set onCarRead(void Function(String tag, String? session)? handler);

  /// Opens Android's settings for this app ([notifications]: its
  /// notification page).
  Future<void> openSettings({bool notifications = false});
}

/// The app's own platform channel (`android/.../NativeBridge.kt`). Every call
/// degrades to a harmless default when the platform side is missing (tests).
class NativeChannel implements NotificationPlatform {
  NativeChannel._() {
    _channel.setMethodCallHandler((MethodCall call) async {
      final Map<Object?, Object?> a = call.arguments is Map ? call.arguments as Map<Object?, Object?> : const <Object?, Object?>{};
      String? str(String k) => a[k] is String && (a[k] as String).isNotEmpty ? a[k] as String : null;
      switch (call.method) {
        case 'tap':
          _onTap?.call();
        case 'carReply':
          final Future<String> Function(CarReply)? h = _onCarReply;
          if (h == null) return 'ComandOS aún está arrancando; no envié nada. Vuelve a intentarlo.';
          return h(CarReply(
            tag: str('tag') ?? '',
            text: str('text') ?? '',
            session: str('session'),
            pane: str('pane'),
            askSig: str('askSig'),
          ));
        case 'carRead':
          _onCarRead?.call(str('tag') ?? '', str('session'));
        case 'carButton':
          if (call.arguments is String) _onCarButton?.call(call.arguments as String);
      }
      return null;
    });
  }

  static final NativeChannel instance = NativeChannel._();
  static const MethodChannel _channel = MethodChannel('comandos/native');

  void Function()? _onTap;
  Future<String> Function(CarReply reply)? _onCarReply;
  void Function(String tag, String? session)? _onCarRead;
  void Function(String button)? _onCarButton;

  /// Steering-wheel / headset buttons while Modo auto is in a call
  /// (`talk`, `next`, `previous`) and `hangup` from the call notice.
  set onCarButton(void Function(String button)? handler) => _onCarButton = handler;

  /// Starts the Modo auto call service: ComandOS keeps listening with the
  /// screen off or another app in front, and gets the steering-wheel buttons.
  /// False when Android refused.
  Future<bool> startCarCall() async => await _call<bool>('carStart') ?? false;

  Future<void> stopCarCall() => _call<void>('carStop');

  /// Asks for the car's microphone over Bluetooth (Android 12+).
  Future<bool> requestBluetooth() async => await _call<bool>('requestBluetooth') ?? false;

  @override
  set onTap(void Function()? callback) => _onTap = callback;

  @override
  set onCarReply(Future<String> Function(CarReply reply)? handler) {
    _onCarReply = handler;
    // Replies that arrived while Dart was not running wait in Android.
    if (handler != null) unawaited(_drainCarReplies());
  }

  @override
  set onCarRead(void Function(String tag, String? session)? handler) => _onCarRead = handler;

  Future<void> _drainCarReplies() async {
    final List<Object?>? queued = await _call<List<Object?>>('takeCarReplies');
    for (final Object? q in queued ?? const <Object?>[]) {
      if (q is! Map) continue;
      final Future<String> Function(CarReply)? h = _onCarReply;
      if (h == null) return;
      String? str(String k) => q[k] is String && (q[k] as String).isNotEmpty ? q[k] as String : null;
      final String tag = str('tag') ?? '';
      final String say = await h(CarReply(
        tag: tag,
        text: str('text') ?? '',
        session: str('session'),
        pane: str('pane'),
        askSig: str('askSig'),
      ));
      await _call<void>('carReplied', <String, Object?>{'tag': tag, 'text': str('text') ?? '', 'say': say});
    }
  }

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
  Future<bool> takeKeepAliveLost() async => await _call<bool>('watchTakeLost') ?? false;

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
        'speech': a.speech,
        'sender': a.sender,
        'askSig': a.askSig,
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
