import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wakelock_plus/wakelock_plus.dart';

import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/session_feed.dart';
import '../../core/models/transcript.dart';
import '../../core/outbox/outbox.dart';
import '../../core/platform/native_channel.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../../ui/theme.dart';
import '../chat/chat_controller.dart';
import '../sessions/sessions_controller.dart';
import '../voice/reply_watcher.dart';
import '../voice/text_prep.dart';
import '../voice/voice_services.dart';
import '../voice/voice_settings.dart';
import 'car_machine.dart';
import 'car_ports.dart';
import 'edge_voice.dart';
import 'on_device.dart';
import 'option_parser.dart';
import 'orchestrator_screen.dart' show CarModeSwitch;

/// The call screen: Orbita's colours on a black ground, huge type.
abstract final class CarColors {
  static const Color bg = Color(0xFF000000);
  static const Color text = Orbita.text;
  static const Color dim = Orbita.dim;
  static const Color go = Orbita.done;
  static const Color wait = Orbita.waiting;
  static const Color work = Orbita.working;
  static const Color stop = Orbita.err;
  static const Color panel = Orbita.panel;
  static const Color hangUp = Color(0xFFE5484D);
}

/// "Modo auto" as a phone call: it starts talking on its own (how many
/// splits wait, then each split, its prompt or reply), listens after each
/// turn and understands spoken commands, so no button is ever needed. The
/// screen is a minimal call UI with one huge hang-up area; the other
/// buttons are optional. Not Android Auto: a regular full-screen screen.
class CarModeScreen extends StatefulWidget {
  const CarModeScreen({super.key, this.session, this.pane});

  final String? session;
  final String? pane;

  @override
  State<CarModeScreen> createState() => _CarModeScreenState();
}

class _CarModeScreenState extends State<CarModeScreen> {
  final VoiceServices _v = VoiceServices.instance;
  final Connection _connection = sl.get<Connection>();
  final Outbox _outbox = sl.get<Outbox>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  late final SessionsController _sessions = SessionsController(_connection);
  late final LiveCarPorts _ports = LiveCarPorts(
    voice: _v,
    outbox: _outbox,
    chat: () => _chat,
    connectivity: sl.maybe<ConnectivityWatch>(),
    // Edge's voice by default (Ajustes › Voz), the phone's as fallback.
    tts: EdgeVoice(
      render: () => edgeThrough(_connection.api),
      voice: () => _v.settings.value.carVoice,
      phone: DeviceVoice.tts,
      dir: _v.cacheDir,
    ),
  );
  late final CarMachine _machine = CarMachine(ports: _ports, session: widget.session, pane: widget.pane);

  ChatController? _chat;
  String? _boundKey;
  ReplyWatcher? _watcher;
  Timer? _recheck;
  ChatMessage? _excerptOf;
  String _excerpt = '';
  bool _started = false;
  bool _closing = false;

  /// The call service runs: the call goes on with the screen off or another
  /// app in front (and the steering-wheel buttons work).
  bool _service = false;

  /// On screen, or in a call kept alive by the service.
  bool get _live => _appVisible.value || _service;

  @override
  void initState() {
    super.initState();
    unawaited(WakelockPlus.enable().catchError((Object _) {}));
    unawaited(_v.ready());
    _sessions.addListener(_onFeed);
    _machine.addListener(_onMachine);
    _appVisible.addListener(_onAppVisible);
    // Polls, speaks and listens only while the app is in the foreground
    // (the wakelock keeps the screen on while Modo auto is open).
    _sessions.setVisible(_appVisible.value);
    _machine.setForeground(_appVisible.value);
    if (widget.session != null) _bind(widget.session!, widget.pane, null);
  }

  @override
  void dispose() {
    _machine.removeListener(_onMachine);
    NativeChannel.instance.onCarButton = null;
    unawaited(NativeChannel.instance.stopCarCall());
    _machine.dispose(); // stops speech, keeps anything said
    _ports.dispose();
    _sessions.removeListener(_onFeed);
    _sessions.dispose();
    _unbind();
    _recheck?.cancel();
    _appVisible.removeListener(_onAppVisible);
    unawaited(WakelockPlus.disable().catchError((Object _) {}));
    super.dispose();
  }

  /// Hidden (another app on top, screen off): no polling, no speech, no
  /// listening, no countdown. Back: polls resume and anything pending is
  /// read. The outbox follows the app's usual background rules.
  void _onAppVisible() {
    final bool on = _live;
    _machine.setForeground(on);
    _sessions.setVisible(on);
    _chat?.setVisible(on);
    if (!on) _recheck?.cancel();
  }

  void _onFeed() {
    final SessionFeed? feed = _sessions.feed;
    if (feed == null) return;
    _machine.updateSessions(splitsOf(feed.sessions));
    _onChat();
    if (!_started) {
      _started = true;
      // Settings (language, speed) first; never a tap to start the call.
      unawaited(_v.ready().then((_) async {
        await _startService();
        if (mounted) unawaited(_machine.startCall());
      }));
    }
  }

  /// The call keeps going in the background and hears the steering wheel.
  /// Asked once: the car's microphone over Bluetooth.
  Future<void> _startService() async {
    final NativeChannel native = NativeChannel.instance;
    await native.requestBluetooth();
    if (!mounted) return;
    native.onCarButton = (String b) {
      if (b == 'hangup') {
        unawaited(_machine.hangUp());
        return;
      }
      final CarButton? button = CarButton.named(b);
      if (button != null) unawaited(_machine.wheel(button));
    };
    _service = await native.startCarCall();
    if (mounted) _onAppVisible();
  }

  void _onMachine() {
    if (_machine.ended) {
      if (!_closing && mounted) {
        _closing = true;
        unawaited(Navigator.of(context).maybePop());
      }
      return;
    }
    final CarSession? s = _machine.current;
    if (s != null && s.key != _boundKey) _bind(s.session, s.pane, s.key);
  }

  void _bind(String session, String? pane, String? key) {
    _unbind();
    _boundKey = key ?? '$session\u0001${pane ?? ''}';
    final String? last = _v.settings.value.lastRead[session];
    _watcher = ReplyWatcher(alreadyRead: <String>{?last});
    _excerptOf = null;
    _excerpt = '';
    final ChatController c = _chat = ChatController(
      session: session,
      pane: pane,
      connection: _connection,
      outbox: _outbox,
    );
    c.addListener(_onChat);
    c.setVisible(_live);
  }

  void _unbind() {
    _recheck?.cancel();
    final ChatController? c = _chat;
    _chat = null;
    _boundKey = null;
    if (c != null) {
      c.removeListener(_onChat);
      c.dispose();
    }
  }

  void _onChat() {
    final ChatController? c = _chat;
    final ReplyWatcher? w = _watcher;
    if (c == null || w == null || !c.loaded) return;
    final bool feedWorking = _machine.current?.state == SessionState.work;
    final bool working = c.working || feedWorking;
    // The split's last finished reply, and whether it was ever read aloud
    // (checked before the watcher takes it as already known).
    final ReplyRef? latest = working ? null : currentReply(c.messages);
    _machine.updateCurrent(
      working: working,
      ask: c.ask,
      excerpt: _excerptFor(c.messages),
      pane: c.pane,
      latest: latest,
      latestUnread: latest != null && !w.wasRead(latest.id),
    );
    final ReplyRef? r = w.observe(c.messages, working: c.working, feedWorking: feedWorking);
    if (r != null) {
      _machine.replyCompleted(r);
    } else if (w.pending) {
      _recheck?.cancel();
      _recheck = Timer(w.settle + const Duration(milliseconds: 100), _onChat);
    }
  }

  /// End of the last assistant message, prepared for reading, ≤ 320 chars.
  String _excerptFor(List<ChatMessage> messages) {
    ChatMessage? last;
    for (int i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role == ChatRole.assistant) {
        last = messages[i];
        break;
      }
    }
    if (last == null) return _excerpt;
    if (identical(last, _excerptOf)) return _excerpt;
    _excerptOf = last;
    String t = selectForReading(prepareForSpeech(last.text), ReadMode.ending).replaceAll('\n\n', ' ');
    if (t.length > 320) t = '…${t.substring(t.length - 320)}';
    return _excerpt = t;
  }

  @override
  Widget build(BuildContext context) {
    return Theme(
      data: Theme.of(context).copyWith(scaffoldBackgroundColor: CarColors.bg),
      child: Scaffold(
        backgroundColor: CarColors.bg,
        body: SafeArea(
          child: ListenableBuilder(
            listenable: _machine,
            builder: (BuildContext context, Widget? _) => OrientationBuilder(
              builder: (BuildContext context, Orientation o) => o == Orientation.landscape
                  ? Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: <Widget>[
                        Expanded(
                          flex: 3,
                          child: Column(children: <Widget>[
                            CarModeSwitch(orchestrator: false, session: widget.session, pane: widget.pane),
                            _header(),
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
                        CarModeSwitch(orchestrator: false, session: widget.session, pane: widget.pane),
                        _header(),
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

  Color _stateColor(SessionState? s) => switch (s) {
        SessionState.ask || SessionState.limit => CarColors.wait,
        SessionState.work => CarColors.work,
        _ => CarColors.go,
      };

  Widget _header() {
    final CarSession? s = _machine.current;
    final bool can = _machine.canSwitch && _machine.sessions.length > 1;
    final String sub = s == null
        ? ''
        : <String>[
            'split ${s.split}${s.splits > 1 ? ' de ${s.splits}' : ''}',
            <String>[s.agent, s.model].where((String x) => x.isNotEmpty).join(' · '),
          ].where((String x) => x.isNotEmpty).join(' · ');
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 0),
      child: Row(
        children: <Widget>[
          _BigIcon(icon: Icons.chevron_left, label: 'Split anterior', onTap: can ? _machine.prev : null),
          Expanded(
            child: Column(
              children: <Widget>[
                Text(
                  s?.label ?? (_sessions.loadedOnce ? 'Sin sesiones' : 'Conectando…'),
                  textAlign: TextAlign.center,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: CarColors.text, fontSize: 32, fontWeight: FontWeight.w800, height: 1.1),
                ),
                if (sub.isNotEmpty)
                  Text(
                    sub,
                    textAlign: TextAlign.center,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(color: CarColors.dim, fontSize: 18, fontWeight: FontWeight.w600),
                  ),
                if (s != null)
                  Text(
                    s.stateText,
                    style: TextStyle(color: _stateColor(s.state), fontSize: 22, fontWeight: FontWeight.w700),
                  ),
              ],
            ),
          ),
          _BigIcon(icon: Icons.chevron_right, label: 'Split siguiente', onTap: can ? _machine.next : null),
        ],
      ),
    );
  }

  Widget _middle() {
    final CarMachine m = _machine;
    final AskPrompt? ask = m.ask;
    final bool confirming = m.phase == CarPhase.confirming && m.heard != null;
    final String hint = m.hint;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Text(
            m.status,
            textAlign: TextAlign.center,
            style: TextStyle(
              color: switch (m.phase) {
                CarPhase.listening => CarColors.stop,
                CarPhase.failed => CarColors.stop,
                CarPhase.confirming || CarPhase.choosing => CarColors.wait,
                _ => m.working ? CarColors.work : CarColors.go,
              },
              fontSize: 30,
              fontWeight: FontWeight.w800,
            ),
          ),
          if (hint.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                hint,
                textAlign: TextAlign.center,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: CarColors.wait, fontSize: 20, fontWeight: FontWeight.w600),
              ),
            ),
          if (m.notice != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                m.notice!,
                textAlign: TextAlign.center,
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: CarColors.dim, fontSize: 18),
              ),
            ),
          const SizedBox(height: 12),
          // No text to read while driving (and nothing for others to see):
          // what was heard and what the agent wrote are spoken, not shown.
          Expanded(
            child: ask != null && !confirming ? _askOptions(ask) : Center(child: _stateIcon(m)),
          ),
        ],
      ),
    );
  }

  /// One big glyph for the moment of the call, readable at a glance.
  Widget _stateIcon(CarMachine m) {
    final (IconData icon, Color color) = switch (m.phase) {
      CarPhase.listening => (Icons.mic, CarColors.stop),
      CarPhase.transcribing || CarPhase.sending => (Icons.more_horiz, CarColors.dim),
      CarPhase.confirming || CarPhase.choosing => (Icons.help_outline, CarColors.wait),
      CarPhase.reading => (Icons.volume_up_outlined, CarColors.go),
      CarPhase.failed => (Icons.error_outline, CarColors.stop),
      CarPhase.idle || CarPhase.waiting => (m.working ? Icons.autorenew : Icons.phone_in_talk_outlined, m.working ? CarColors.work : CarColors.go),
    };
    return Icon(icon, size: 140, color: color.withValues(alpha: 0.85));
  }

  Widget _askOptions(AskPrompt ask) {
    final bool busy = _machine.phase == CarPhase.sending || _machine.phase == CarPhase.choosing;
    return ListView(
      children: <Widget>[
        Text(
          ask.question.isEmpty ? 'El agente te pregunta' : ask.question,
          maxLines: 3,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(color: CarColors.text, fontSize: 24, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 8),
        for (final AskOption o in ask.choices)
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: CarBigButton(
              label: '${o.key}. ${speakableLabel(o.label)}',
              color: _machine.pendingOption?.key == o.key ? CarColors.wait : CarColors.panel,
              textColor: _machine.pendingOption?.key == o.key ? CarColors.bg : CarColors.text,
              height: 80,
              fontSize: 24,
              onTap: busy ? null : () => _machine.chooseOption(o),
            ),
          ),
      ],
    );
  }

  /// Optional buttons for the moment (never required: everything has a
  /// spoken equivalent), then the huge hang-up area.
  Widget _controls({bool compact = false}) {
    final CarMachine m = _machine;
    final double h = compact ? 80 : 88;
    final Widget row;
    switch (m.phase) {
      case CarPhase.confirming:
        row = Row(children: <Widget>[
          Expanded(child: CarBigButton(label: 'Cancelar', color: CarColors.panel, textColor: CarColors.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(
            child: CarBigButton(label: m.needsTap ? 'Enviar' : 'Enviar ya', color: CarColors.go, onTap: m.sendNow, height: h),
          ),
        ]);
      case CarPhase.choosing:
        row = Row(children: <Widget>[
          Expanded(child: CarBigButton(label: 'Cancelar', color: CarColors.panel, textColor: CarColors.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(child: CarBigButton(label: 'Confirmar', color: CarColors.go, onTap: m.confirmChoice, height: h)),
        ]);
      case CarPhase.failed:
        row = Row(children: <Widget>[
          Expanded(child: CarBigButton(label: 'Descartar', color: CarColors.panel, textColor: CarColors.stop, onTap: m.discard, height: h)),
          const SizedBox(width: 12),
          Expanded(child: CarBigButton(label: 'Reintentar', color: CarColors.wait, onTap: m.talk, height: h)),
        ]);
      case CarPhase.listening:
        row = Row(children: <Widget>[
          Expanded(child: CarBigButton(label: 'Cancelar', color: CarColors.panel, textColor: CarColors.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(child: CarBigButton(label: 'Terminar', color: CarColors.go, onTap: m.talk, height: h)),
        ]);
      case CarPhase.transcribing:
      case CarPhase.sending:
        row = CarBigButton(label: m.status, color: CarColors.panel, onTap: null, height: h);
      case CarPhase.idle:
      case CarPhase.reading:
      case CarPhase.waiting:
        row = Row(children: <Widget>[
          Expanded(
            flex: 2,
            child: m.speaking
                ? CarBigButton(label: 'Callar', color: CarColors.panel, textColor: CarColors.wait, onTap: m.hush, height: h)
                : CarBigButton(label: 'Hablar', color: CarColors.go, onTap: m.talk, height: h),
          ),
          if (m.working) ...<Widget>[
            const SizedBox(width: 12),
            Expanded(
              child: CarBigButton(label: 'Detener', color: CarColors.panel, textColor: CarColors.stop, onTap: m.stopAgent, height: h),
            ),
          ],
        ]);
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 12),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.end,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          row,
          const SizedBox(height: 12),
          CarBigButton(
            label: 'Colgar',
            color: CarColors.hangUp,
            textColor: CarColors.text,
            icon: Icons.call_end,
            height: compact ? 104 : 132,
            fontSize: 38,
            onTap: () {
              if (m.inCall) {
                unawaited(m.hangUp());
              } else {
                unawaited(Navigator.of(context).maybePop());
              }
            },
          ),
        ],
      ),
    );
  }
}

class CarBigButton extends StatelessWidget {
  const CarBigButton({
    super.key,
    required this.label,
    required this.color,
    required this.onTap,
    this.textColor = CarColors.bg,
    this.height = 88,
    this.fontSize = 28,
    this.icon,
  });

  final String label;
  final Color color;
  final Color textColor;
  final VoidCallback? onTap;
  final double height;
  final double fontSize;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final bool on = onTap != null;
    final Color fg = on ? textColor : Orbita.faint;
    return Semantics(
      button: true,
      enabled: on,
      label: label,
      child: SizedBox(
        height: height,
        width: double.infinity,
        child: Material(
          color: on ? color : CarColors.panel,
          borderRadius: BorderRadius.circular(22),
          child: InkWell(
            borderRadius: BorderRadius.circular(22),
            onTap: onTap,
            child: Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    if (icon != null) ...<Widget>[
                      Icon(icon, color: fg, size: fontSize * 1.2),
                      const SizedBox(width: 12),
                    ],
                    Flexible(
                      // One word shrinks to fit instead of breaking («Deten-er»).
                      child: label.contains(' ')
                          ? Text(
                              label,
                              textAlign: TextAlign.center,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(color: fg, fontSize: fontSize, fontWeight: FontWeight.w800),
                            )
                          : FittedBox(
                              fit: BoxFit.scaleDown,
                              child: Text(
                                label,
                                maxLines: 1,
                                style: TextStyle(color: fg, fontSize: fontSize, fontWeight: FontWeight.w800),
                              ),
                            ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _BigIcon extends StatelessWidget {
  const _BigIcon({required this.icon, required this.label, required this.onTap});

  final IconData icon;
  final String label;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => SizedBox(
        width: 88,
        height: 88,
        child: IconButton(
          tooltip: label,
          onPressed: onTap,
          iconSize: 64,
          color: CarColors.text,
          disabledColor: Orbita.line2,
          icon: Icon(icon),
        ),
      );
}
