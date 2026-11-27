import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:wakelock_plus/wakelock_plus.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/json_read.dart';
import '../../core/models/session_feed.dart';
import '../../core/outbox/outbox.dart';
import '../../core/platform/native_channel.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../chat/chat_controller.dart';
import '../voice/voice_services.dart';
import 'car_mic.dart';
import 'car_ports.dart';
import 'orchestrator_view.dart';
import 'edge_voice.dart';
import 'on_device.dart';
import 'orchestrator_call.dart';
import 'wake_phrase.dart';

/// The orchestrator session through the app's own chat controller.
class LiveOrchestratorLink extends ChangeNotifier implements OrchestratorLink {
  LiveOrchestratorLink(this._connection, this._outbox);

  final Connection _connection;
  final Outbox _outbox;
  ChatController? _chat;
  ChatController? get chat => _chat;
  String? _session;
  String? _pane;
  bool _busy = false;
  DateTime _busyAt = DateTime.fromMillisecondsSinceEpoch(0);
  bool _busyLoading = false;

  /// Whether the desktop sees the orchestrator working, from `/session-feed`
  /// (at most every 1.5 s; a failed read keeps the last answer).
  Future<void> _refreshBusy() async {
    final ComandosApi? api = _connection.api;
    final String? session = _session;
    if (api == null || session == null || _busyLoading) return;
    if (DateTime.now().difference(_busyAt) < const Duration(milliseconds: 1500)) return;
    _busyLoading = true;
    _busyAt = DateTime.now();
    try {
      final SessionFeed feed = await api.sessionFeed();
      bool busy = false;
      for (final SessionSummary s in feed.sessions) {
        if (s.session != session) continue;
        final PaneSummary? p = s.paneList.where((PaneSummary p) => p.pane == _pane).firstOrNull;
        busy = (p?.state ?? s.state) == SessionState.work;
      }
      if (busy != _busy) {
        _busy = busy;
        notifyListeners();
      }
    } catch (_) {
      // Keep the last answer: the transcript rule still applies.
    } finally {
      _busyLoading = false;
    }
  }

  @override
  Future<(String, String?)> open() async {
    final ComandosApi? api = _connection.api;
    if (api == null) throw Exception('El teléfono no está vinculado.');
    final ApiResponse r = await api.request('POST', '/auto/session', body: const <String, Object?>{});
    if (r.status == 404) throw Exception('Actualiza ComandOS en la computadora.');
    if (!r.ok) throw Exception(readString(r.map, 'error').isEmpty ? 'El servidor respondió ${r.status}.' : readString(r.map, 'error'));
    final String session = readString(r.map, 'session');
    final String pane = readString(r.map, 'pane');
    if (session.isEmpty) throw Exception('El servidor no dio la sesión.');
    _session = session;
    _pane = pane.isEmpty ? null : pane;
    _chat?.dispose();
    _chat = ChatController(session: session, pane: pane.isEmpty ? null : pane, connection: _connection, outbox: _outbox)
      ..addListener(notifyListeners)
      ..setVisible(true);
    return (session, pane.isEmpty ? null : pane);
  }

  @override
  OrchSnapshot get now => OrchSnapshot(
        messages: _chat?.messages ?? const <Never>[],
        ask: _chat?.ask,
        loaded: _chat?.loaded ?? false,
        busy: _busy,
      );

  @override
  void poke() {
    _chat?.poke();
    unawaited(_refreshBusy());
  }

  @override
  Future<String?> answer(AskPrompt ask, AskOption option) async =>
      await _chat?.answerPrompt(ask, option) ?? 'No hay sesión.';

  @override
  Future<String?> interrupt() async => await _chat?.interrupt();

  void setVisible(bool on) => _chat?.setVisible(on);

  @override
  void close() {
    _chat?.removeListener(notifyListeners);
    _chat?.dispose();
    _chat = null;
  }
}

/// Modo auto: a phone call with one Claude Code session that drives the
/// others (`OrchestratorCall`). It opens asleep and listens after «OK
/// ComandOS», a tap or the steering wheel; the replies are read aloud.
class OrchestratorScreen extends StatefulWidget {
  const OrchestratorScreen({super.key});

  @override
  State<OrchestratorScreen> createState() => _OrchestratorScreenState();
}

class _OrchestratorScreenState extends State<OrchestratorScreen>  {
  final VoiceServices _v = VoiceServices.instance;
  final Connection _connection = sl.get<Connection>();
  final Outbox _outbox = sl.get<Outbox>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  late final LiveOrchestratorLink _link = LiveOrchestratorLink(_connection, _outbox);

  /// Hears «OK ComandOS» and records the person's turn without letting go
  /// of the microphone in between.
  late final NativeCarMic _mic = NativeCarMic(dir: _v.dictation.takesDir);
  late final LiveCarPorts _ports = LiveCarPorts(
    mic: _mic,
    voice: _v,
    outbox: _outbox,
    chat: () => _link.chat,
    connectivity: sl.maybe<ConnectivityWatch>(),
    tts: EdgeVoice(
      render: () => edgeThrough(_connection.api),
      voice: () => _v.settings.value.carVoice,
      phone: DeviceVoice.tts,
      dir: _v.cacheDir,
    ),
  );
  late final OrchestratorCall _call =
      OrchestratorCall(ports: _ports, link: _link, wake: NativeWake(keepMic: true));
  bool _service = false;
  bool _closing = false;

  bool get _live => _appVisible.value || _service;

  @override
  void initState() {
    super.initState();
    unawaited(WakelockPlus.enable().catchError((Object _) {}));
    _call.addListener(_onCall);
    _appVisible.addListener(_onVisible);
    unawaited(_v.ready().then((_) async {
      await _startService();
      if (mounted) unawaited(_call.start());
    }));
  }

  Future<void> _startService() async {
    final NativeChannel native = NativeChannel.instance;
    await native.requestBluetooth();
    if (!mounted) return;
    native.onCarButton = (String b) {
      if (b == 'hangup') {
        unawaited(_call.hangUp());
        return;
      }
      final CarButton? button = CarButton.named(b);
      if (button != null) unawaited(_call.wheel(button));
    };
    _service = await native.startCarCall();
    if (mounted) _onVisible();
  }

  void _onVisible() {
    _call.setForeground(_live);
    _link.setVisible(_live);
  }

  void _onCall() {
    if (_call.ended && !_closing && mounted) {
      _closing = true;
      unawaited(Navigator.of(context).maybePop());
    }
  }

  @override
  void dispose() {
    _call.removeListener(_onCall);
    _appVisible.removeListener(_onVisible);
    NativeChannel.instance.onCarButton = null;
    NativeChannel.instance.onTakeLevel = null;
    unawaited(NativeChannel.instance.stopCarCall());
    _call.dispose();
    _ports.dispose();
    unawaited(WakelockPlus.disable().catchError((Object _) {}));
    super.dispose();
  }

  Future<void> _hangUp() async {
    if (_call.ended) {
      await Navigator.of(context).maybePop();
    } else {
      await _call.hangUp();
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: Listenable.merge(<Listenable>[_call, _v.dictation, _mic.level]),
        builder: (BuildContext context, Widget? _) => OrchestratorCallView(
          phase: _call.phase,
          status: _call.status,
          heard: _call.heard,
          said: _call.said,
          wake: !_call.noWake,
          level: _call.phase == OrchPhase.listening ? max(_mic.level.value, _v.dictation.level) : 0,
          onTalk: () => unawaited(_call.talk()),
          onHush: _call.hush,
          onStop: () => unawaited(_call.stopIt()),
          onHangUp: () => unawaited(_hangUp()),
        ),
      );
}
