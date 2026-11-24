import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/connection.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/transcript.dart';
import '../car/option_parser.dart';
import 'car_text.dart';
import '../../core/platform/native_channel.dart';
import '../../core/util/atomic_json_file.dart';
import '../../core/util/clock.dart';
import 'notice.dart';
import 'notice_alerts.dart';
import 'notice_watcher.dart';
import 'notifications_hook.dart';

/// "Avisos en segundo plano": owns the opt-in, the keep-alive service, the
/// [NoticeWatcher] and the notifications it produces.
///
/// The watcher runs in the app's main isolate (no second Flutter engine);
/// the native foreground service only keeps the process alive while the
/// app is in the background. See ARCHITECTURE.md → Notifications.
class NotificationsController extends ChangeNotifier implements NotificationsHook {
  NotificationsController({
    required this.platform,
    required this.connection,
    required Future<AtomicJsonFile> Function() file,
    required this.visibleChat,
    required this.uiReady,
    required this.openChat,
    NoticeTransport? transport,
    this.askUser,
    this.onMessage,
    this.onReachable,
    this.carSend,
    this.connectivity,
    bool foreground = true,
    TimerFactory timer = systemTimer,
  })  : _file = file,
        _transport = transport ?? ApiNoticeTransport(() => connection.api),
        _foreground = foreground,
        _timer = timer {
    platform.onTap = _handleTap;
    platform.onCarReply = _carReply;
    platform.onCarRead = _carRead;
    visibleChat.addListener(_onVisibleChat);
    uiReady.addListener(_handleTap);
    uiReady.addListener(_showLost);
    connection.tokenRejected.addListener(_onTokenRejected);
    connectivity?.addListener(_onConnectivity);
  }

  static const int maxShown = 64;

  /// Shown on the next app open after Android stopped the keep-alive service.
  static const String keepAliveLostMessage =
      'Los avisos en segundo plano se detuvieron; ábrelos de nuevo en Ajustes';

  final NotificationPlatform platform;
  final Connection connection;

  /// Session whose chat is on screen (null when none).
  final ValueListenable<String?> visibleChat;

  /// True once the sessions list is shown (a tap may open a chat).
  final ValueListenable<bool> uiReady;
  final void Function(String session, String? pane) openChat;

  /// Shows the one-time opt-in question. Null when no screen can show it.
  final Future<bool?> Function()? askUser;

  /// Shows a short message (Spanish) to the person, if the app is visible.
  final void Function(String message, {bool openNotificationSettings})? onMessage;

  /// The desktop answered: delivery work may resume (the outbox).
  final void Function()? onReachable;

  /// True while the network is up (connectivity hint).
  final ValueListenable<bool>? connectivity;

  /// Queues a message dictated in the car (the outbox: never lost) and says
  /// how it went, in Spanish.
  final Future<String> Function(String session, String? pane, String text)? carSend;

  /// Tag of the «probar en el coche» conversation: replies stay on the phone.
  static const String carTestTag = 'comandos:car-test';

  final Future<AtomicJsonFile> Function() _file;
  final NoticeTransport _transport;
  final TimerFactory _timer;

  AtomicJsonFile? _doc;
  Future<void>? _loading;
  bool _enabled = false;
  bool _asked = false;
  bool _alsoDone = true;
  bool _carContent = true;
  bool _disposed = false;
  bool _allowed = true;
  bool _keepAliveRefused = false;
  bool _keepAliveLost = false;
  bool _asking = false;
  bool _foreground;
  String? _host;
  NoticeCursor? _cursor;
  final Map<String, ShownAlert> _shown = <String, ShownAlert>{};
  NoticeWatcher? _watcher;

  bool get enabled => _enabled;
  bool get alsoDone => _alsoDone;

  /// The car reads what the agent wrote (else only who needs you).
  bool get carContent => _carContent;

  /// Android lets the app post notifications.
  bool get allowed => _allowed;

  /// Android refused to start the keep-alive service last time.
  bool get keepAliveRefused => _keepAliveRefused;

  /// Android stopped the service in the background; not yet told.
  bool get keepAliveLost => _keepAliveLost;
  /// Work that finishes after [dispose] (a save, an answer) tells no one.
  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  WatcherState get watcherState => _watcher?.state ?? WatcherState.stopped;
  bool get asked => _asked;

  // ---- NotificationsHook ---------------------------------------------------

  @override
  Future<void> onPaired() async {
    await _load();
    final String? host = connection.server?.displayHost;
    if (host != _host) {
      // Another desktop (or first pairing): start from its current position.
      _host = host;
      _cursor = null;
      _shown.clear();
      _stopWatcher();
      await _save();
    }
    await _reconcile();
    _maybeAsk();
    _handleTap();
  }

  @override
  Future<void> onUnpaired() async {
    await _load();
    _stopWatcher();
    await platform.stopKeepAlive();
    await platform.cancelAll();
    _host = null;
    _cursor = null;
    _shown.clear();
    await _save();
    notifyListeners();
  }

  @override
  void onForegroundChanged(bool foreground) {
    _foreground = foreground;
    if (!foreground || _loading == null) return;
    unawaited(_checkLost());
    unawaited(_refreshAllowed());
    unawaited(_reconcile());
    _watcher?.nudge();
    _maybeAsk();
  }

  // ---- Settings ------------------------------------------------------------

  /// Turns the feature on (asking Android's permission first) or off.
  /// Returns Spanish copy when it could not be turned on.
  Future<String?> setEnabled(bool on) async {
    await _load();
    if (on) {
      bool ok = await platform.notificationsAllowed();
      if (!ok) ok = await platform.requestNotifications();
      _allowed = ok;
      if (!ok) {
        notifyListeners();
        return 'Sin permiso de notificaciones no puedo avisarte. '
            'Actívalo en los ajustes de Android.';
      }
    }
    _enabled = on;
    _asked = true;
    if (!on) {
      await platform.cancelAll();
      _shown.clear();
    }
    await _save();
    await _reconcile();
    if (on && _keepAliveRefused) {
      return 'Android no dejó arrancar el aviso en segundo plano. Vuelve a intentarlo '
          'con la app abierta.';
    }
    return null;
  }

  Future<void> setAlsoDone(bool on) async {
    await _load();
    _alsoDone = on;
    await _save();
    notifyListeners();
  }

  Future<void> setCarContent(bool on) async {
    await _load();
    _carContent = on;
    await _save();
    notifyListeners();
  }

  /// Posts a conversation the car shows like a real one: hear it, reply,
  /// and the reply comes back without reaching any agent.
  Future<String?> testInCar() async {
    await _load();
    if (!await platform.notificationsAllowed()) {
      return 'Android no deja mostrar notificaciones. Permítelas en los ajustes de la app.';
    }
    await platform.show(const LocalAlert(
      tag: carTestTag,
      urgent: true,
      title: 'Prueba de ComandOS',
      body: 'Así te llegará un aviso en el coche. Contesta para probar.',
      speech: 'Así te llegará un aviso de una sesión. Pulsa responder y di algo para probar; '
          'no se envía a ningún agente.',
      sender: 'ComandOS',
    ));
    return null;
  }

  Future<void> openSettings({bool notifications = false}) =>
      platform.openSettings(notifications: notifications);

  // ---- Internals -----------------------------------------------------------

  Future<void> _load() => _loading ??= () async {
        _doc = await _file();
        Object? json;
        try {
          json = await _doc!.read();
        } catch (_) {
          json = null;
        }
        if (json is Map) {
          _enabled = json['enabled'] == true;
          _asked = json['asked'] == true;
          _alsoDone = json['alsoDone'] != false;
          _carContent = json['carContent'] != false;
          final Object? host = json['host'];
          _host = host is String ? host : null;
          final Object? rev = json['rev'];
          final Object? after = json['after'];
          if (rev is String && after is int) _cursor = NoticeCursor(rev: rev, after: after);
          final Object? shown = json['shown'];
          if (shown is Map) {
            shown.forEach((Object? k, Object? v) {
              if (k is String && v is Map && v['e'] is String) {
                _shown[k] = ShownAlert(eventId: v['e'] as String, waitsOnRequest: v['w'] == true);
              }
            });
          }
        }
        _allowed = await platform.notificationsAllowed();
        await _checkLost();
      }();

  /// Asks Android whether the keep-alive service was stopped in the
  /// background; if so (and notices are on), tells the person once.
  Future<void> _checkLost() async {
    bool lost = false;
    try {
      lost = await platform.takeKeepAliveLost();
    } catch (_) {}
    if (lost && _enabled) _keepAliveLost = true;
    _showLost();
  }

  void _showLost() {
    final void Function(String, {bool openNotificationSettings})? say = onMessage;
    if (!_keepAliveLost || say == null || !_foreground || !uiReady.value) return;
    _keepAliveLost = false;
    say(keepAliveLostMessage);
  }

  Future<void> _save() async {
    final AtomicJsonFile? doc = _doc;
    if (doc == null) return;
    try {
      await doc.write(<String, Object?>{
        'v': 1,
        'enabled': _enabled,
        'asked': _asked,
        'alsoDone': _alsoDone,
        'carContent': _carContent,
        'host': _host,
        'rev': _cursor?.rev,
        'after': _cursor?.after,
        'shown': <String, Object?>{
          for (final MapEntry<String, ShownAlert> e in _shown.entries)
            e.key: <String, Object?>{'e': e.value.eventId, 'w': e.value.waitsOnRequest},
        },
      });
    } catch (_) {
      // Disk trouble: the next save retries; worst case a restart replays a
      // few notices (each replaces its session's notification).
    }
  }

  Future<void> _refreshAllowed() async {
    final bool now = await platform.notificationsAllowed();
    if (now != _allowed) {
      _allowed = now;
      notifyListeners();
    }
  }

  bool get _wanted => _enabled && connection.isPaired && !connection.tokenRejected.value;

  Future<void> _reconcile() async {
    await _load();
    if (_wanted) {
      final NoticeWatcher w = _watcher ??= NoticeWatcher(
        transport: _transport,
        cursor: _cursor,
        saveCursor: (NoticeCursor c) {
          _cursor = c;
          return _save();
        },
        onNotices: _onNotices,
        onPending: _onPending,
        onUnauthorized: _onUnauthorized,
        onHealthy: onReachable,
        hidden: () => !_foreground,
        timer: _timer,
      );
      if (!w.isRunning) w.start();
      _keepAliveRefused = !await platform.startKeepAlive();
    } else {
      _stopWatcher();
      await platform.stopKeepAlive();
    }
    notifyListeners();
  }

  void _stopWatcher() {
    _watcher?.stop();
    _watcher = null;
  }

  String? get _openChat => _foreground ? visibleChat.value : null;

  void _onNotices(List<Notice> notices, Set<String> pending) {
    final List<LocalAlert> alerts = alertsFor(notices,
        openChat: _openChat, includeDone: _alsoDone, pending: pending, carContent: _carContent);
    for (final LocalAlert a in alerts) {
      unawaited(_post(a));
      _shown.remove(a.tag);
      _shown[a.tag] = ShownAlert.of(a);
    }
    while (_shown.length > maxShown) {
      _shown.remove(_shown.keys.first);
    }
  }

  /// Shows [a]; one that waits on you first reads the pane, so the car hears
  /// the permission it would approve (the command and the options).
  Future<void> _post(LocalAlert a) async {
    LocalAlert alert = a;
    final String? session = a.session;
    final api = connection.api;
    if (a.urgent && session != null && a.speech != null && api != null) {
      try {
        final TranscriptResponse t = await api
            .transcript(session: session, pane: a.pane)
            .timeout(const Duration(seconds: 3));
        final AskPrompt? ask = t.ask;
        if (ask != null) alert = a.withSpeech(askCarText(ask), askSig: ask.signature, pane: t.pane);
      } catch (_) {
        // Unreachable: the plain notice goes out; a car reply re-reads anyway.
      }
    }
    await platform.show(alert);
  }

  /// A reply from the car. Every reply re-reads the pane first: a permission
  /// is only answered when it is the very prompt the driver heard, a message
  /// never lands on an open menu. Returns the outcome the car shows.
  Future<String> _carReply(CarReply r) async {
    if (r.tag == carTestTag) {
      return r.text.trim().isEmpty
          ? 'No escuché nada. Es una prueba: no se envió nada.'
          : 'Te escuché: «${r.text.trim()}». Es una prueba: no se envió a ningún agente.';
    }
    final String? session = r.session;
    if (session == null) return 'No sé a qué sesión va; no envié nada.';
    final api = connection.api;
    if (api == null) return 'El teléfono no está vinculado; no envié nada.';
    AskPrompt? open;
    String? pane = r.pane;
    bool checked = false;
    try {
      final TranscriptResponse t =
          await api.transcript(session: session, pane: r.pane).timeout(const Duration(seconds: 4));
      open = t.ask;
      pane = t.pane ?? r.pane;
      checked = true;
    } catch (_) {
      // A permission is never answered without looking at it again.
      if (r.askSig != null) return 'Sin conexión con tu equipo: no respondí el permiso. Vuelve a intentarlo.';
    }
    final CarReplyPlan plan = planCarReply(r.text, open: open, heardSig: checked ? r.askSig : null);
    switch (plan) {
      case CarRefuse(:final String why):
        return why;
      case CarAnswer(:final AskOption option):
        try {
          await api.answerAsk(session: session, pane: pane, option: option.key);
          return 'Respondí la opción ${optionWord(option.key)}: ${speakableLabel(option.label)}.';
        } catch (_) {
          return 'No pude responder el permiso; revísalo en la sesión.';
        }
      case CarSend(:final String text):
        final Future<String> Function(String, String?, String)? send = carSend;
        if (send == null) return 'No pude enviar el mensaje desde aquí.';
        return send(session, pane, text);
    }
  }

  void _carRead(String tag, String? session) {
    _shown.remove(tag);
    final api = connection.api;
    if (session != null && api != null) unawaited(api.sessionSeen(session).catchError((Object _) {}));
  }

  void _onPending(Set<String> pending) {
    final List<String> gone = resolvedTags(_shown, pending);
    if (gone.isEmpty) return;
    for (final String t in gone) {
      unawaited(platform.cancel(t));
      _shown.remove(t);
    }
    unawaited(_save());
  }

  void _onUnauthorized() {
    _watcher = null;
    connection.tokenRejected.value = true;
    unawaited(platform.stopKeepAlive());
    if (!_foreground) {
      unawaited(platform.show(const LocalAlert(
        tag: tokenAlertTag,
        urgent: false,
        title: 'ComandOS: vuelve a vincular',
        body: 'Tu equipo rechazó el token. Abre la app y vincula de nuevo para seguir '
            'recibiendo avisos.',
      )));
    }
    notifyListeners();
  }

  void _onTokenRejected() {
    if (_loading == null) return;
    if (!connection.tokenRejected.value) unawaited(platform.cancel(tokenAlertTag));
    unawaited(_reconcile());
  }

  void _onConnectivity() {
    if (connectivity?.value ?? false) _watcher?.nudge();
  }

  void _onVisibleChat() {
    final String? s = visibleChat.value;
    if (s == null) return;
    final String tag = tagForSession(s);
    unawaited(platform.cancel(tag));
    if (_shown.remove(tag) != null) unawaited(_save());
  }

  Future<void> _handleTap() async {
    if (!uiReady.value) return; // taken once the sessions list is up
    final ({String? pane, String? session})? tap = await platform.takeTap();
    final String? session = tap?.session;
    if (session != null) openChat(session, tap?.pane);
  }

  void _maybeAsk() {
    final Future<bool?> Function()? ask = askUser;
    if (ask == null || _asked || _asking || !_foreground || !connection.isPaired) return;
    _asking = true;
    // Let the sessions list settle first.
    _timer(const Duration(milliseconds: 700), () async {
      bool? yes;
      try {
        yes = _foreground && !_asked ? await ask() : null;
      } finally {
        _asking = false;
      }
      if (yes == null) return; // nothing could show it; ask next time
      if (!yes) {
        _asked = true;
        await _save();
        notifyListeners();
        return;
      }
      final String? problem = await setEnabled(true);
      onMessage?.call(problem ?? 'Listo: te aviso cuando una sesión te necesite.',
          openNotificationSettings: problem != null && !_allowed);
    });
  }

  @override
  void dispose() {
    _disposed = true;
    _stopWatcher();
    platform.onTap = null;
    platform.onCarReply = null;
    platform.onCarRead = null;
    visibleChat.removeListener(_onVisibleChat);
    uiReady.removeListener(_handleTap);
    uiReady.removeListener(_showLost);
    connection.tokenRejected.removeListener(_onTokenRejected);
    connectivity?.removeListener(_onConnectivity);
    super.dispose();
  }
}
