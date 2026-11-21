import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/locator.dart';
import '../../core/outbox/drafts.dart';
import '../../ui/theme.dart';
import '../chat/chat_controller.dart';
import 'dictation.dart';
import 'openai_key_store.dart';
import 'reply_watcher.dart';
import 'voice.dart';
import 'voice_services.dart';

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
            if (mine && d.phase == DictationPhase.recording) {
              return IconButton(
                tooltip: 'Terminar dictado',
                onPressed: d.stop,
                icon: const Icon(Icons.stop_circle, color: Orbita.err),
              );
            }
            if (mine && d.phase == DictationPhase.transcribing) {
              return const SizedBox(
                width: kTouch,
                height: kTouch,
                child: Padding(padding: EdgeInsets.all(14), child: CircularProgressIndicator(strokeWidth: 2)),
              );
            }
            return IconButton(
              tooltip: 'Dictar',
              onPressed: d.busy ? null : () => unawaited(d.start(owner)),
              icon: const Icon(Icons.mic_none, color: Orbita.dim),
            );
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
            _row(
              <Widget>[
                const Icon(Icons.fiber_manual_record, color: Orbita.err, size: 14),
                const SizedBox(width: 6),
                Text('Escuchando ${clockText(d.elapsed)}', style: const TextStyle(color: Orbita.text)),
                const SizedBox(width: 10),
                Expanded(
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(3),
                    child: LinearProgressIndicator(
                      value: d.level,
                      minHeight: 6,
                      backgroundColor: Orbita.line,
                      color: Orbita.done,
                    ),
                  ),
                ),
                TextButton(onPressed: d.cancel, child: const Text('Cancelar')),
              ],
            ),
          if (mine && d.phase == DictationPhase.transcribing)
            _row(<Widget>[
              const SizedBox(width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2)),
              const SizedBox(width: 10),
              Expanded(
                child: Text(d.attempt > 1 ? 'Transcribiendo… (intento ${d.attempt} de 3)' : 'Transcribiendo…',
                    style: const TextStyle(color: Orbita.text)),
              ),
              TextButton(onPressed: d.cancel, child: const Text('Cancelar')),
            ]),
          if (notice != null)
            _row(<Widget>[
              Expanded(child: Text(notice, style: const TextStyle(color: Orbita.waiting))),
              IconButton(
                tooltip: 'Cerrar',
                onPressed: () => d.clearNotice(widget.owner),
                icon: const Icon(Icons.close, size: 18),
              ),
            ]),
          if (pending.isNotEmpty)
            _row(<Widget>[
              Expanded(
                child: Text(
                  'No se pudo transcribir${pending.length > 1 ? ' (${pending.length} grabaciones)' : ''}: '
                  '${pending.first.message ?? ''}',
                  style: const TextStyle(color: Orbita.err),
                ),
              ),
              TextButton(
                onPressed: d.busy ? null : () => unawaited(d.retry(pending.first.id)),
                child: const Text('Reintentar'),
              ),
              TextButton(
                onPressed: () => _confirmDiscard(pending.first),
                style: TextButton.styleFrom(foregroundColor: Orbita.err),
                child: const Text('Descartar'),
              ),
            ]),
        ];
        if (rows.isEmpty) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.only(bottom: 6),
          child: Column(mainAxisSize: MainAxisSize.min, children: rows),
        );
      },
    );
  }

  Future<void> _confirmDiscard(PendingTake t) async {
    final bool? ok = await showDialog<bool>(
      context: context,
      builder: (BuildContext c) => AlertDialog(
        backgroundColor: Orbita.panel,
        title: const Text('¿Descartar la grabación?'),
        content: const Text('Se borra el audio y no se transcribe.'),
        actions: <Widget>[
          TextButton(onPressed: () => Navigator.of(c).pop(false), child: const Text('Cancelar')),
          TextButton(
            onPressed: () => Navigator.of(c).pop(true),
            style: TextButton.styleFrom(foregroundColor: Orbita.err),
            child: const Text('Descartar'),
          ),
        ],
      ),
    );
    if (ok == true) await _v.dictation.discard(t.id);
  }

  Widget _row(List<Widget> children) => Container(
        margin: const EdgeInsets.only(bottom: 4),
        padding: const EdgeInsets.only(left: 10),
        decoration: BoxDecoration(color: Orbita.panel, borderRadius: BorderRadius.circular(10)),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: kTouch),
          child: Row(children: children),
        ),
      );
}

/// "0:07", "1:23".
String clockText(Duration d) => '${d.inMinutes}:${(d.inSeconds % 60).toString().padLeft(2, '0')}';

/// App-bar button shown only while something is being read aloud.
class SpeechStopButton extends StatelessWidget {
  const SpeechStopButton({super.key});

  @override
  Widget build(BuildContext context) {
    if (sl.maybe<Speaker>() == null) return const SizedBox.shrink();
    final VoiceServices v = VoiceServices.instance;
    return ListenableBuilder(
      listenable: v.speech,
      builder: (BuildContext context, Widget? _) => v.speech.speaking
          ? IconButton(
              tooltip: 'Callar',
              onPressed: v.speech.stop,
              icon: const Icon(Icons.volume_off, color: Orbita.waiting),
            )
          : const SizedBox.shrink(),
    );
  }
}

/// Menu entry for the chat: "Leer respuestas nuevas" with a check mark.
PopupMenuEntry<String>? autoReadMenuItem(String session) {
  if (sl.maybe<Speaker>() == null) return null;
  final VoiceServices v = VoiceServices.instance;
  return CheckedPopupMenuItem<String>(
    value: autoReadMenuValue,
    checked: v.keys.status.value.present && v.settings.autoReads(session),
    child: const Text('Leer respuestas nuevas'),
  );
}

const String autoReadMenuValue = 'voice-autoread';

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
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
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

  @override
  void initState() {
    super.initState();
    if (sl.maybe<Speaker>() == null) return;
    final VoiceServices v = _v = VoiceServices.instance;
    unawaited(v.ready().then((_) {
      if (!mounted) return;
      v.settings.addListener(_onSettings);
      widget.chat.addListener(_onChat);
      _onSettings();
    }));
  }

  @override
  void dispose() {
    _recheck?.cancel();
    _v?.settings.removeListener(_onSettings);
    widget.chat.removeListener(_onChat);
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
