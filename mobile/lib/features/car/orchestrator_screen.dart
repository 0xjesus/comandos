import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:wakelock_plus/wakelock_plus.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/json_read.dart';
import '../../core/outbox/outbox.dart';
import '../../core/platform/native_channel.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../chat/chat_controller.dart';
import '../voice/voice_services.dart';
import 'car_machine.dart';
import 'car_mode.dart';
import 'car_mode_screen.dart';
import 'car_ports.dart';
import 'edge_voice.dart';
import 'on_device.dart';
import 'orchestrator_call.dart';

/// The orchestrator session through the app's own chat controller.
class LiveOrchestratorLink extends ChangeNotifier implements OrchestratorLink {
  LiveOrchestratorLink(this._connection, this._outbox);

  final Connection _connection;
  final Outbox _outbox;
  ChatController? _chat;
  ChatController? get chat => _chat;

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
      );

  @override
  void poke() => _chat?.poke();

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

/// «Por sesiones» / «Orquestador» at the top of both Modo auto screens. The
/// choice is remembered (Ajustes › Voz) and switches the screen right away.
class CarModeSwitch extends StatelessWidget {
  const CarModeSwitch({super.key, required this.orchestrator, this.session, this.pane});

  final bool orchestrator;
  final String? session;
  final String? pane;

  Future<void> _pick(BuildContext context, bool want) async {
    if (want == orchestrator) return;
    await VoiceServices.instance.settings.update((s) => s.copyWith(carOrchestrator: want));
    if (!context.mounted) return;
    await Navigator.of(context).pushReplacement(MaterialPageRoute<void>(
      settings: const RouteSettings(name: CarMode.routeName),
      builder: (BuildContext c) => want ? const OrchestratorScreen() : CarModeScreen(session: session, pane: pane),
    ));
  }

  @override
  Widget build(BuildContext context) {
    Widget pill(String label, IconData icon, bool on, bool value) => Expanded(
          child: Semantics(
            button: true,
            selected: on,
            child: InkWell(
              borderRadius: BorderRadius.circular(999),
              onTap: () => unawaited(_pick(context, value)),
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 180),
                height: 52,
                decoration: BoxDecoration(
                  color: on ? CarColors.panel : Colors.transparent,
                  borderRadius: BorderRadius.circular(999),
                  border: Border.all(color: on ? CarColors.go.withValues(alpha: 0.55) : Colors.transparent),
                ),
                padding: const EdgeInsets.symmetric(horizontal: 10),
                alignment: Alignment.center,
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: <Widget>[
                      Icon(icon, size: 22, color: on ? CarColors.go : CarColors.dim),
                      const SizedBox(width: 8),
                      Text(label,
                          style: TextStyle(
                              color: on ? CarColors.text : CarColors.dim, fontSize: 18, fontWeight: FontWeight.w700)),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
      child: Container(
        padding: const EdgeInsets.all(4),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(999),
          border: Border.all(color: CarColors.panel),
        ),
        child: Row(children: <Widget>[
          pill('Por sesiones', Icons.view_carousel_outlined, !orchestrator, false),
          const SizedBox(width: 4),
          pill('Orquestador', Icons.hub_outlined, orchestrator, true),
        ]),
      ),
    );
  }
}

/// Modo auto with the orchestrator: a phone call with one Claude Code session
/// that drives the others (`OrchestratorCall`). Same call manners as the
/// split-by-split mode: it starts talking on its own, listens after each
/// answer, the steering wheel talks, one huge hang-up; no text to read.
class OrchestratorScreen extends StatefulWidget {
  const OrchestratorScreen({super.key});

  @override
  State<OrchestratorScreen> createState() => _OrchestratorScreenState();
}

class _OrchestratorScreenState extends State<OrchestratorScreen> with SingleTickerProviderStateMixin {
  final VoiceServices _v = VoiceServices.instance;
  final Connection _connection = sl.get<Connection>();
  final Outbox _outbox = sl.get<Outbox>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  late final LiveOrchestratorLink _link = LiveOrchestratorLink(_connection, _outbox);
  late final LiveCarPorts _ports = LiveCarPorts(
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
  late final OrchestratorCall _call = OrchestratorCall(ports: _ports, link: _link);
  late final AnimationController _pulse = AnimationController(vsync: this, duration: const Duration(milliseconds: 1800))
    ..repeat();
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
    unawaited(NativeChannel.instance.stopCarCall());
    _call.dispose();
    _ports.dispose();
    _pulse.dispose();
    unawaited(WakelockPlus.disable().catchError((Object _) {}));
    super.dispose();
  }

  Color get _tone => switch (_call.phase) {
        OrchPhase.listening => CarColors.stop,
        OrchPhase.speaking => CarColors.go,
        OrchPhase.thinking || OrchPhase.sending || OrchPhase.connecting => CarColors.work,
        OrchPhase.asking => CarColors.wait,
        OrchPhase.failed => CarColors.stop,
        OrchPhase.idle || OrchPhase.ended => CarColors.dim,
      };

  IconData get _glyph => switch (_call.phase) {
        OrchPhase.listening => Icons.mic,
        OrchPhase.speaking => Icons.graphic_eq,
        OrchPhase.thinking || OrchPhase.sending || OrchPhase.connecting => Icons.hub_outlined,
        OrchPhase.asking => Icons.help_outline,
        OrchPhase.failed => Icons.error_outline,
        OrchPhase.idle || OrchPhase.ended => Icons.phone_in_talk_outlined,
      };

  /// The call's centre: a breathing orb in the colour of the moment.
  Widget _orb(double size) => AnimatedBuilder(
        animation: _pulse,
        builder: (BuildContext context, Widget? _) {
          final bool moving = _call.phase != OrchPhase.idle && _call.phase != OrchPhase.failed;
          final double t = moving ? _pulse.value : 0;
          return SizedBox.square(
            dimension: size,
            child: Stack(
              alignment: Alignment.center,
              children: <Widget>[
                for (int ring = 0; ring < 3; ring++)
                  Opacity(
                    opacity: moving ? (1 - ((t + ring / 3) % 1)) * 0.35 : 0.12,
                    child: Container(
                      width: size * (0.55 + 0.45 * ((t + ring / 3) % 1)),
                      height: size * (0.55 + 0.45 * ((t + ring / 3) % 1)),
                      decoration: BoxDecoration(shape: BoxShape.circle, border: Border.all(color: _tone, width: 3)),
                    ),
                  ),
                Container(
                  width: size * 0.5,
                  height: size * 0.5,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    gradient: RadialGradient(colors: <Color>[
                      _tone.withValues(alpha: 0.95),
                      _tone.withValues(alpha: 0.35 + 0.15 * math.sin(t * math.pi * 2).abs()),
                    ]),
                    boxShadow: <BoxShadow>[BoxShadow(color: _tone.withValues(alpha: 0.45), blurRadius: 48, spreadRadius: 4)],
                  ),
                  child: Icon(_glyph, size: size * 0.22, color: CarColors.bg),
                ),
              ],
            ),
          );
        },
      );

  Widget _middle() => Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: <Widget>[
          const Text('Orquestador',
              style: TextStyle(color: CarColors.dim, fontSize: 20, fontWeight: FontWeight.w700, letterSpacing: 0.5)),
          const SizedBox(height: 12),
          LayoutBuilder(
            builder: (BuildContext c, BoxConstraints b) =>
                _orb(math.min(b.maxWidth, 300).clamp(160, 300).toDouble()),
          ),
          const SizedBox(height: 16),
          Text(_call.status,
              textAlign: TextAlign.center,
              style: TextStyle(color: _tone, fontSize: 32, fontWeight: FontWeight.w800)),
        ],
      );

  Widget _controls({bool compact = false}) {
    final OrchestratorCall c = _call;
    final double h = compact ? 80 : 88;
    final bool speaking = c.phase == OrchPhase.speaking;
    final bool thinking = c.phase == OrchPhase.thinking || c.phase == OrchPhase.sending;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 12),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.end,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          Row(children: <Widget>[
            Expanded(
              flex: 2,
              child: speaking
                  ? CarBigButton(label: 'Callar', color: CarColors.panel, textColor: CarColors.wait, onTap: c.hush, height: h)
                  : CarBigButton(
                      label: c.phase == OrchPhase.listening ? 'Listo' : 'Hablar',
                      color: CarColors.go,
                      onTap: () => unawaited(c.talk()),
                      height: h,
                    ),
            ),
            if (thinking) ...<Widget>[
              const SizedBox(width: 12),
              Expanded(
                child: CarBigButton(
                    label: 'Detener',
                    color: CarColors.panel,
                    textColor: CarColors.stop,
                    onTap: () => unawaited(c.stopIt()),
                    height: h),
              ),
            ],
          ]),
          const SizedBox(height: 12),
          CarBigButton(
            label: 'Colgar',
            color: CarColors.hangUp,
            textColor: CarColors.text,
            icon: Icons.call_end,
            height: compact ? 104 : 132,
            fontSize: 38,
            onTap: () => unawaited(c.ended ? Navigator.of(context).maybePop() : c.hangUp()),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Theme(
        data: Theme.of(context).copyWith(scaffoldBackgroundColor: CarColors.bg),
        child: Scaffold(
          backgroundColor: CarColors.bg,
          body: SafeArea(
            child: ListenableBuilder(
              listenable: _call,
              builder: (BuildContext context, Widget? _) => OrientationBuilder(
                builder: (BuildContext context, Orientation o) => o == Orientation.landscape
                    ? Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: <Widget>[
                          Expanded(
                            flex: 3,
                            child: Column(children: <Widget>[
                              const CarModeSwitch(orchestrator: true),
                              Expanded(child: _middle()),
                            ]),
                          ),
                          const SizedBox(width: 12),
                          Expanded(flex: 2, child: _controls(compact: true)),
                        ],
                      )
                    : Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: <Widget>[
                          const CarModeSwitch(orchestrator: true),
                          Expanded(child: _middle()),
                          _controls(),
                        ],
                      ),
              ),
            ),
          ),
        ),
      );
}
