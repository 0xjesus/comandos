import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/locator.dart';
import '../../core/outbox/drafts.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../chat/chat_controller.dart';
import 'dictation.dart';
import 'openai_key_store.dart';
import 'reply_watcher.dart';
import 'voice.dart';
import 'voice_services.dart';

// The voice controls of the chat, drawn like the web page's own controls
// (`#cv-dock .cv-compose` and `.cv-act`): the mic is a round 44 px key left of
// the send button, read-aloud is one more button next to «Copiar». The voice
// engine (dictation, speech, settings) is unchanged.

/// Composer mic: tap to record, tap again to stop. Hidden without a key.
class DictationMic extends StatelessWidget {
  const DictationMic({super.key, required this.owner});

  final String owner;

  @override
  Widget build(BuildContext context) {
    final VoiceServices v = VoiceServices.instance;
    unawaited(v.keys.load());
    return ValueListenableBuilder<KeyStatus>(
      valueListenable: v.keys.status,
      builder: (BuildContext context, KeyStatus k, Widget? _) {
        if (!k.present) return const SizedBox.shrink();
        return ListenableBuilder(
          listenable: v.dictation,
          builder: (BuildContext context, Widget? _) {
            final DictationController d = v.dictation;
            final bool mine = d.activeOwner == owner;
            final Widget key;
            if (mine && d.phase == DictationPhase.recording) {
              key = WebButton(
                tooltip: 'Terminar dictado',
                onTap: d.stop,
                size: 44,
                radius: 22,
                lip: 2,
                face: Orbita.err,
                ink: Colors.white,
                child: Container(
                  width: 14,
                  height: 14,
                  decoration: BoxDecoration(color: Colors.white, borderRadius: BorderRadius.circular(3)),
                ),
              );
            } else if (mine && d.phase == DictationPhase.transcribing) {
              key = const WebButton(
                tooltip: 'Transcribiendo',
                size: 44,
                radius: 22,
                lip: 2,
                face: Orbita.bg,
                child: SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2.5, color: Orbita.dim),
                ),
              );
            } else {
              key = WebButton(
                tooltip: 'Dictar',
                onTap: d.busy ? null : () => unawaited(d.start(owner)),
                size: 44,
                radius: 22,
                lip: 2,
                face: Orbita.bg,
                ink: Orbita.dim,
                child: const WebIcon(WebIcons.mic, size: 20, semanticLabel: 'Dictar'),
              );
            }
            return Padding(padding: const EdgeInsets.only(right: 8), child: key);
          },
        );
      },
    );
  }
}

/// Strip above the composer: recording (time + level + Cancelar),
/// "Transcribiendo…", notices, and recordings waiting for a retry.
/// Also the sink that inserts dictated text at the cursor.
class DictationPanel extends StatefulWidget {
  const DictationPanel({super.key, required this.owner, required this.controller});

  final String owner;
  final TextEditingController controller;

  @override
  State<DictationPanel> createState() => _DictationPanelState();
}

class _DictationPanelState extends State<DictationPanel> {
  final VoiceServices _v = VoiceServices.instance;
  String? _confirmDiscard;

  @override
  void initState() {
    super.initState();
    _v.dictation.attach(widget.owner, _insert);
    _v.keys.status.addListener(_onKey);
    unawaited(_v.ready().then((_) => _onKey()));
  }

  void _onKey() {
    if (_v.keys.status.value.present) unawaited(_v.dictation.load());
  }

  @override
  void didUpdateWidget(DictationPanel old) {
    super.didUpdateWidget(old);
    if (old.owner != widget.owner) {
      _v.dictation.detach(old.owner, _insert);
      _v.dictation.attach(widget.owner, _insert);
    }
  }

  @override
  void dispose() {
    _v.keys.status.removeListener(_onKey);
    _v.dictation.detach(widget.owner, _insert);
    // Leaving mid-recording keeps what was said: it is transcribed and
    // lands in this session's draft.
    if (_v.dictation.activeOwner == widget.owner && _v.dictation.phase == DictationPhase.recording) {
      unawaited(_v.dictation.stop());
    }
    super.dispose();
  }

  Future<void> _insert(String text) async {
    final TextEditingController c = widget.controller;
    final (String next, int cursor) = insertDictation(c.text, c.selection.start, c.selection.end, text);
    c.value = TextEditingValue(text: next, selection: TextSelection.collapsed(offset: cursor));
    await sl.maybe<DraftStore>()?.setNow(widget.owner, next);
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _v.dictation,
      builder: (BuildContext context, Widget? _) {
        final DictationController d = _v.dictation;
        final bool mine = d.activeOwner == widget.owner;
        final List<PendingTake> pending = d.pendingFor(widget.owner);
        final String? notice = d.noticeFor(widget.owner);
        final List<Widget> rows = <Widget>[
          if (mine && d.phase == DictationPhase.recording)
            _row(<Widget>[
              Container(
                width: 9,
                height: 9,
                decoration: const BoxDecoration(color: Orbita.err, shape: BoxShape.circle),
              ),
              const SizedBox(width: 8),
              Text('Escuchando ${clockText(d.elapsed)}', style: _text),
              const SizedBox(width: 10),
              Expanded(child: _Level(level: d.level)),
              const SizedBox(width: 8),
              _key('Cancelar', d.cancel),
            ]),
          if (mine && d.phase == DictationPhase.transcribing)
            _row(<Widget>[
              const SizedBox(width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2, color: Orbita.dim)),
              const SizedBox(width: 10),
              Expanded(
                child: Text(d.attempt > 1 ? 'Transcribiendo… (intento ${d.attempt} de 3)' : 'Transcribiendo…', style: _text),
              ),
              _key('Cancelar', d.cancel),
            ]),
          if (notice != null)
            _row(<Widget>[
              Expanded(child: Text(notice, style: _text.copyWith(color: Orbita.waiting))),
              _key('Cerrar', () => d.clearNotice(widget.owner)),
            ]),
          if (pending.isNotEmpty)
            _row(<Widget>[
              Expanded(
                child: Text(
                  _confirmDiscard == pending.first.id
                      ? '¿Descartar la grabación? Se borra el audio y no se transcribe.'
                      : 'No se pudo transcribir${pending.length > 1 ? ' (${pending.length} grabaciones)' : ''}: '
                          '${pending.first.message ?? ''}',
                  style: _text.copyWith(color: Orbita.err),
                ),
              ),
              if (_confirmDiscard == pending.first.id) ...<Widget>[
                _key('No', () => setState(() => _confirmDiscard = null)),
                const SizedBox(width: 6),
                _key('Descartar', () {
                  final String id = pending.first.id;
                  setState(() => _confirmDiscard = null);
                  unawaited(d.discard(id));
                }, danger: true),
              ] else ...<Widget>[
                _key('Reintentar', d.busy ? null : () => unawaited(d.retry(pending.first.id))),
                const SizedBox(width: 6),
                _key('Descartar', () => setState(() => _confirmDiscard = pending.first.id), danger: true),
              ],
            ]),
        ];
        if (rows.isEmpty) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.fromLTRB(8, 6, 8, 0),
          child: Column(mainAxisSize: MainAxisSize.min, children: rows),
        );
      },
    );
  }

  static const TextStyle _text = TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.text);

  Widget _key(String label, VoidCallback? onTap, {bool danger = false}) => WebButton(
        onTap: onTap,
        face: Orbita.bg,
        ink: danger ? Orbita.err : Orbita.text,
        radius: 10,
        lip: 2,
        minHeight: 36,
        child: Text(label, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, fontWeight: FontWeight.w600)),
      );

  Widget _row(List<Widget> children) => Container(
        margin: const EdgeInsets.only(bottom: 6),
        padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
        decoration: BoxDecoration(
          color: Orbita.bg,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(color: Orbita.line2),
        ),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 44),
          child: Row(children: children),
        ),
      );
}

/// Microphone level: a 6 px bar in `--done` over `--line`.
class _Level extends StatelessWidget {
  const _Level({required this.level});
  final double level;

  @override
  Widget build(BuildContext context) => Container(
        key: const ValueKey<String>('dictation-level'),
        height: 6,
        decoration: BoxDecoration(color: Orbita.line, borderRadius: BorderRadius.circular(3)),
        alignment: Alignment.centerLeft,
        child: FractionallySizedBox(
          widthFactor: level.clamp(0.0, 1.0),
          child: Container(decoration: BoxDecoration(color: Orbita.done, borderRadius: BorderRadius.circular(3))),
        ),
      );
}

/// "0:07", "1:23".
String clockText(Duration d) => '${d.inMinutes}:${(d.inSeconds % 60).toString().padLeft(2, '0')}';

/// «Leer» next to «Copiar» on an answer; «Callar» while it is being read.
/// Hidden when no [Speaker] is registered. Without a key, speaking leads to
/// Ajustes › Voz (the voice engine decides).
class ReadAloudAct extends StatelessWidget {
  const ReadAloudAct({super.key, required this.text});

  final String text;

  /// The text being read by a tap here (so only its card says «Callar»).
  static String? _reading;

  @override
  Widget build(BuildContext context) {
    final Speaker? speaker = sl.maybe<Speaker>();
    if (speaker == null) return const SizedBox.shrink();
    final VoiceServices? v = VoiceServices.existing;
    Widget button(bool speaking) => SelectionContainer.disabled(
          child: WebButton(
            tooltip: speaking ? 'Callar' : 'Leer en voz alta',
            onTap: () {
              if (speaking) {
                _reading = null;
                unawaited(speaker.stop());
              } else {
                _reading = text;
                unawaited(speaker.speak(text));
              }
            },
            face: Orbita.bg,
            ink: Orbita.text,
            radius: 12,
            lip: 2,
            padding: const EdgeInsets.symmetric(horizontal: 14),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                WebIcon(speaking ? WebIcons.volumeOff : WebIcons.volumeOn, size: 16, color: Orbita.text),
                const SizedBox(width: 6),
                Text(speaking ? 'Callar' : 'Leer',
                    style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13.5, fontWeight: FontWeight.w600)),
              ],
            ),
          ),
        );
    if (v == null) return button(false);
    return ListenableBuilder(
      listenable: v.speech,
      builder: (BuildContext context, Widget? _) => button(v.speech.speaking && _reading == text),
    );
  }
}

/// «Leer nuevas» at the end of the dock's key row: reads each new answer of
/// this session aloud (on = pressed). Hidden when no [Speaker] is registered.
class AutoReadKey extends StatelessWidget {
  const AutoReadKey({super.key, required this.session, required this.builder});

  final String session;

  /// Draws the key like its neighbours: (label, pressed, onTap).
  final Widget Function(String label, bool on, VoidCallback onTap) builder;

  @override
  Widget build(BuildContext context) {
    if (sl.maybe<Speaker>() == null) return const SizedBox.shrink();
    final VoiceServices? v = VoiceServices.existing;
    bool on() => v != null && v.keys.status.value.present && v.settings.autoReads(session);
    Widget key() => builder('Leer nuevas', on(), () => unawaited(toggleAutoRead(context, session)));
    if (v == null) return key();
    return ListenableBuilder(listenable: v.settings, builder: (BuildContext context, Widget? _) => key());
  }
}

Future<void> toggleAutoRead(BuildContext context, String session) async {
  final VoiceServices v = VoiceServices.instance;
  await v.ready();
  if (!v.keys.status.value.present) {
    if (context.mounted) await openVoiceSettings(context);
    return;
  }
  final bool on = !v.settings.autoReads(session);
  await v.settings.setAutoRead(session, on);
  if (!on) await v.speech.stop();
  if (!context.mounted) return;
  ScaffoldMessenger.maybeOf(context)
    ?..hideCurrentSnackBar()
    ..showSnackBar(SnackBar(
      content: Text(on
          ? 'Leeré en voz alta cada respuesta nueva de esta sesión (${v.settings.value.autoReadMode.label.toLowerCase()}).'
          : 'Ya no leeré las respuestas de esta sesión.'),
    ));
}

/// Invisible: while the chat is open and "Leer respuestas nuevas" is on,
/// reads each new assistant reply once it completes. Never re-reads.
class ChatAutoReader extends StatefulWidget {
  const ChatAutoReader({super.key, required this.chat, required this.session});

  final ChatController chat;
  final String session;

  @override
  State<ChatAutoReader> createState() => _ChatAutoReaderState();
}

class _ChatAutoReaderState extends State<ChatAutoReader> {
  VoiceServices? _v;
  ReplyWatcher? _watcher;
  Timer? _recheck;
  bool _on = false;
  bool _attached = false;

  @override
  void initState() {
    super.initState();
    if (sl.maybe<Speaker>() == null) return;
    final VoiceServices v = _v = VoiceServices.instance;
    unawaited(v.ready().then((_) {
      if (!mounted) return;
      _attached = true;
      v.settings.addListener(_onSettings);
      widget.chat.addListener(_onChat);
      _onSettings();
    }));
  }

  @override
  void dispose() {
    _recheck?.cancel();
    if (_attached) {
      _v?.settings.removeListener(_onSettings);
      widget.chat.removeListener(_onChat);
    }
    super.dispose();
  }

  void _onSettings() {
    final VoiceServices v = _v!;
    final bool on = v.settings.autoReads(widget.session) && v.keys.status.value.present;
    if (on == _on) return;
    _on = on;
    if (on) {
      final String? last = v.settings.value.lastRead[widget.session];
      _watcher = ReplyWatcher(alreadyRead: <String>{?last});
      if (widget.chat.loaded) _watcher!.baseline(widget.chat.messages);
    } else {
      _watcher = null;
      _recheck?.cancel();
    }
  }

  void _onChat() {
    final ReplyWatcher? w = _watcher;
    final VoiceServices? v = _v;
    if (!_on || w == null || v == null || !widget.chat.loaded) return;
    final ReplyRef? r = w.observe(widget.chat.messages, working: widget.chat.working);
    if (r != null) {
      unawaited(v.settings.markRead(widget.session, r.id));
      // Never talk over a dictation in progress.
      if (!v.dictation.busy) unawaited(v.speech.speakReply(r.text, mode: v.settings.value.autoReadMode));
    } else if (w.pending) {
      _recheck?.cancel();
      _recheck = Timer(w.settle + const Duration(milliseconds: 100), _onChat);
    }
  }

  @override
  Widget build(BuildContext context) => const SizedBox.shrink();
}
