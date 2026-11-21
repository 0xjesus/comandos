import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wakelock_plus/wakelock_plus.dart';

import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/session_feed.dart';
import '../../core/models/transcript.dart';
import '../../core/outbox/outbox.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../chat/chat_controller.dart';
import '../sessions/sessions_controller.dart';
import '../voice/openai_key_store.dart';
import '../voice/reply_watcher.dart';
import '../voice/text_prep.dart';
import '../voice/voice.dart';
import '../voice/voice_services.dart';
import '../voice/voice_settings.dart';
import 'car_machine.dart';
import 'car_ports.dart';
import 'option_parser.dart';

/// Very high contrast palette for a phone on a car mount.
abstract final class _C {
  static const Color bg = Color(0xFF000000);
  static const Color text = Color(0xFFFFFFFF);
  static const Color dim = Color(0xFFB8C2D6);
  static const Color go = Color(0xFF2EE59D);
  static const Color wait = Color(0xFFFFC233);
  static const Color work = Color(0xFF7AA5FF);
  static const Color stop = Color(0xFFFF5C6A);
  static const Color panel = Color(0xFF14181F);
}

/// "Modo auto": eyes on the road. One giant Hablar button, huge type, the
/// reply read aloud, permission prompts by button or by voice. Not Android
/// Auto: this is a regular full-screen app screen.
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
  );
  late final CarMachine _machine = CarMachine(ports: _ports, session: widget.session);

  ChatController? _chat;
  String? _boundId;
  ReplyWatcher? _watcher;
  Timer? _recheck;
  Timer? _warm;
  ChatMessage? _excerptOf;
  String _excerpt = '';

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
    if (widget.session != null) _bind(widget.session!, widget.pane);
    // A few seconds in (never delaying the first words): cache the fixed
    // phrases so feedback still speaks without network.
    _warm = Timer(const Duration(seconds: 8), () => unawaited(_v.speech.warm(kCarPhrases)));
  }

  @override
  void dispose() {
    _machine.removeListener(_onMachine);
    _machine.dispose(); // stops speech, keeps anything said
    _ports.dispose();
    _sessions.removeListener(_onFeed);
    _sessions.dispose();
    _unbind();
    _recheck?.cancel();
    _warm?.cancel();
    _appVisible.removeListener(_onAppVisible);
    unawaited(WakelockPlus.disable().catchError((Object _) {}));
    super.dispose();
  }

  /// Hidden (another app on top, screen off): no polling, no speech, no
  /// listening, no countdown. Back: polls resume and anything pending is
  /// read. The outbox follows the app's usual background rules.
  void _onAppVisible() {
    final bool on = _appVisible.value;
    _machine.setForeground(on);
    _sessions.setVisible(on);
    _chat?.setVisible(on);
    if (!on) _recheck?.cancel();
  }

  void _onFeed() {
    final SessionFeed? feed = _sessions.feed;
    if (feed == null) return;
    _machine.updateSessions(<CarSession>[
      for (final SessionSummary s in feed.sessions)
        CarSession(session: s.session, label: s.label, state: s.state, ts: s.ts, pane: s.pane, reply: s.reply),
    ]);
    _onChat();
  }

  void _onMachine() {
    final String? id = _machine.currentId;
    if (id != null && id != _boundId) _bind(id, _machine.current?.pane);
  }

  void _bind(String session, String? pane) {
    _unbind();
    _boundId = session;
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
    c.setVisible(_appVisible.value);
  }

  void _unbind() {
    _recheck?.cancel();
    final ChatController? c = _chat;
    _chat = null;
    _boundId = null;
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
    _machine.updateCurrent(
      working: c.working || feedWorking,
      ask: c.ask,
      excerpt: _excerptFor(c.messages),
      pane: c.pane,
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
      data: Theme.of(context).copyWith(scaffoldBackgroundColor: _C.bg),
      child: Scaffold(
        backgroundColor: _C.bg,
        body: SafeArea(
          child: ValueListenableBuilder<KeyStatus>(
            valueListenable: _v.keys.status,
            builder: (BuildContext context, KeyStatus key, Widget? _) {
              if (key.loaded && !key.present) return _NeedsKey(onOpen: () => openVoiceSettings(context));
              return ListenableBuilder(
                listenable: Listenable.merge(<Listenable>[_machine, _v.speech]),
                builder: (BuildContext context, Widget? _) => OrientationBuilder(
                  builder: (BuildContext context, Orientation o) => o == Orientation.landscape
                      ? Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: <Widget>[
                            Expanded(flex: 3, child: Column(children: <Widget>[_header(), Expanded(child: _middle())])),
                            const SizedBox(width: 12),
                            Expanded(flex: 2, child: _controls(compact: true)),
                          ],
                        )
                      : Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: <Widget>[_header(), Expanded(child: _middle()), _controls()],
                        ),
                ),
              );
            },
          ),
        ),
      ),
    );
  }

  Color _stateColor(SessionState? s) => switch (s) {
        SessionState.ask || SessionState.limit => _C.wait,
        SessionState.work => _C.work,
        _ => _C.go,
      };

  Widget _header() {
    final CarSession? s = _machine.current;
    final bool can = _machine.canSwitch && _machine.sessions.length > 1;
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 0),
      child: Row(
        children: <Widget>[
          _BigIcon(icon: Icons.chevron_left, label: 'Sesión anterior', onTap: can ? _machine.prev : null),
          Expanded(
            child: Column(
              children: <Widget>[
                Text(
                  s?.label ?? (_sessions.loadedOnce ? 'Sin sesiones' : 'Conectando…'),
                  textAlign: TextAlign.center,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: _C.text, fontSize: 34, fontWeight: FontWeight.w800, height: 1.1),
                ),
                if (s != null)
                  Text(
                    s.stateText,
                    style: TextStyle(color: _stateColor(s.state), fontSize: 24, fontWeight: FontWeight.w700),
                  ),
              ],
            ),
          ),
          _BigIcon(icon: Icons.chevron_right, label: 'Sesión siguiente', onTap: can ? _machine.next : null),
        ],
      ),
    );
  }

  Widget _middle() {
    final CarMachine m = _machine;
    final AskPrompt? ask = m.ask;
    final bool confirming = m.phase == CarPhase.confirming && m.heard != null;
    final String? speechError = _v.speech.lastError?.userMessage;
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
                CarPhase.listening => _C.stop,
                CarPhase.failed => _C.stop,
                CarPhase.confirming || CarPhase.choosing => _C.wait,
                _ => m.working ? _C.work : _C.go,
              },
              fontSize: 30,
              fontWeight: FontWeight.w800,
            ),
          ),
          if (m.notice != null || speechError != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                m.notice ?? 'Voz: $speechError',
                textAlign: TextAlign.center,
                maxLines: 3,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: _C.dim, fontSize: 20),
              ),
            ),
          const SizedBox(height: 12),
          Expanded(
            child: confirming
                ? Center(
                    child: Text(
                      '«${m.heard}»',
                      textAlign: TextAlign.center,
                      maxLines: 6,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(color: _C.wait, fontSize: 30, fontWeight: FontWeight.w700, height: 1.2),
                    ),
                  )
                : ask != null
                    ? _askOptions(ask)
                    : Text(
                        m.excerpt.isEmpty ? '' : m.excerpt,
                        maxLines: 6,
                        overflow: TextOverflow.fade,
                        style: const TextStyle(color: _C.text, fontSize: 26, height: 1.25),
                      ),
          ),
        ],
      ),
    );
  }

  Widget _askOptions(AskPrompt ask) {
    final bool busy = _machine.phase == CarPhase.sending || _machine.phase == CarPhase.choosing;
    return ListView(
      children: <Widget>[
        Text(
          ask.question.isEmpty ? 'El agente te pregunta' : ask.question,
          maxLines: 3,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(color: _C.text, fontSize: 24, fontWeight: FontWeight.w700),
        ),
        const SizedBox(height: 8),
        for (final AskOption o in ask.options)
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: _BigButton(
              label: '${o.key}. ${speakableLabel(o.label)}',
              color: _machine.pendingOption?.key == o.key ? _C.wait : _C.panel,
              textColor: _machine.pendingOption?.key == o.key ? _C.bg : _C.text,
              height: 96,
              fontSize: 26,
              onTap: busy ? null : () => _machine.chooseOption(o),
            ),
          ),
      ],
    );
  }

  Widget _controls({bool compact = false}) {
    final CarMachine m = _machine;
    final double h = compact ? 96 : 104;
    final Widget main;
    switch (m.phase) {
      case CarPhase.confirming:
        main = Row(children: <Widget>[
          Expanded(child: _BigButton(label: 'Cancelar', color: _C.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(
            child: _BigButton(label: m.needsTap ? 'Enviar' : 'Enviar ya', color: _C.go, onTap: m.sendNow, height: h),
          ),
        ]);
      case CarPhase.choosing:
        main = Row(children: <Widget>[
          Expanded(child: _BigButton(label: 'Cancelar', color: _C.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(child: _BigButton(label: 'Confirmar', color: _C.go, onTap: m.confirmChoice, height: h)),
        ]);
      case CarPhase.failed:
        main = Row(children: <Widget>[
          Expanded(child: _BigButton(label: 'Descartar', color: _C.panel, textColor: _C.stop, onTap: m.discard, height: h)),
          const SizedBox(width: 12),
          Expanded(flex: 2, child: _BigButton(label: 'Reintentar', color: _C.wait, onTap: m.talk, height: h)),
        ]);
      case CarPhase.listening:
        main = Row(children: <Widget>[
          Expanded(child: _BigButton(label: 'Cancelar', color: _C.panel, textColor: _C.stop, onTap: m.cancel, height: h)),
          const SizedBox(width: 12),
          Expanded(flex: 2, child: _BigButton(label: 'Terminar', color: _C.stop, onTap: m.talk, height: h)),
        ]);
      case CarPhase.transcribing:
      case CarPhase.sending:
        main = _BigButton(label: m.status, color: _C.panel, onTap: null, height: h);
      case CarPhase.idle:
      case CarPhase.reading:
        main = _BigButton(label: 'Hablar', color: _C.go, height: compact ? 120 : 150, fontSize: 44, onTap: m.talk);
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 12),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.end,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          Row(children: <Widget>[
            Expanded(
              child: _BigButton(
                label: 'Callar',
                color: _C.panel,
                textColor: m.speaking || _v.speech.speaking ? _C.wait : _C.dim,
                onTap: m.speaking || _v.speech.speaking ? m.hush : null,
                height: h,
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: _BigButton(
                label: 'Detener',
                color: _C.panel,
                textColor: m.working ? _C.stop : _C.dim,
                onTap: m.working ? m.stopAgent : null,
                height: h,
              ),
            ),
          ]),
          const SizedBox(height: 12),
          main,
          const SizedBox(height: 8),
          SizedBox(
            height: compact ? 48 : 56,
            width: double.infinity,
            child: TextButton(
              onPressed: () => Navigator.of(context).maybePop(),
              child: const Text('Salir del modo auto', style: TextStyle(color: _C.dim, fontSize: 20)),
            ),
          ),
        ],
      ),
    );
  }
}

class _BigButton extends StatelessWidget {
  const _BigButton({
    required this.label,
    required this.color,
    required this.onTap,
    this.textColor = _C.bg,
    this.height = 104,
    this.fontSize = 30,
  });

  final String label;
  final Color color;
  final Color textColor;
  final VoidCallback? onTap;
  final double height;
  final double fontSize;

  @override
  Widget build(BuildContext context) {
    final bool on = onTap != null;
    return Semantics(
      button: true,
      enabled: on,
      label: label,
      child: SizedBox(
        height: height,
        width: double.infinity,
        child: Material(
          color: on ? color : _C.panel,
          borderRadius: BorderRadius.circular(22),
          child: InkWell(
            borderRadius: BorderRadius.circular(22),
            onTap: onTap,
            child: Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Text(
                  label,
                  textAlign: TextAlign.center,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: on ? textColor : const Color(0xFF5E6980),
                    fontSize: fontSize,
                    fontWeight: FontWeight.w800,
                  ),
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
        width: 96,
        height: 96,
        child: IconButton(
          tooltip: label,
          onPressed: onTap,
          iconSize: 72,
          color: _C.text,
          disabledColor: const Color(0xFF3A4252),
          icon: Icon(icon),
        ),
      );
}

class _NeedsKey extends StatelessWidget {
  const _NeedsKey({required this.onOpen});

  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            const Text(
              'El modo auto necesita la llave de OpenAI para escucharte y leerte las respuestas.',
              textAlign: TextAlign.center,
              style: TextStyle(color: _C.text, fontSize: 26, fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 24),
            _BigButton(label: 'Configurar la voz', color: _C.go, onTap: onOpen),
            const SizedBox(height: 12),
            _BigButton(
              label: 'Salir',
              color: _C.panel,
              textColor: _C.text,
              onTap: () => Navigator.of(context).maybePop(),
            ),
          ],
        ),
      );
}
