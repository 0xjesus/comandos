
import 'package:flutter/material.dart';

import '../../core/locator.dart';
import '../../core/outbox/drafts.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../ui/theme.dart';
import '../voice/chat_voice.dart';
import '../voice/voice.dart';

/// Shown under the composer while a permission prompt is open.
const String kAnswerPromptFirst = 'Responde primero el permiso de arriba';

/// Bottom text box. The text is persisted as a draft while typing; on send
/// it goes to the outbox first, and only once that is durable does the box
/// clear. A failure keeps the text where it is.
class Composer extends StatefulWidget {
  const Composer({
    super.key,
    required this.session,
    required this.pane,
    required this.baselineFor,
    required this.onSent,
    required this.controller,
    this.sendBlockedReason,
  });

  final String session;
  final String? Function() pane;
  final int? Function(String text) baselineFor;
  final VoidCallback onSent;

  /// Owned by the chat screen so "Editar" can put text back.
  final TextEditingController controller;

  /// When set, sending is disabled (typing and the draft still work) and
  /// this one-line reason shows under the box.
  final String? sendBlockedReason;

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  final DraftStore _drafts = sl.get<DraftStore>();
  final Outbox _outbox = sl.get<Outbox>();
  final VoiceInput? _voice = sl.maybe<VoiceInput>();
  bool _sending = false;

  TextEditingController get _ctl => widget.controller;

  @override
  void initState() {
    super.initState();
    _lastText = _ctl.text;
    _ctl.addListener(_onChanged);
  }

  @override
  void dispose() {
    _ctl.removeListener(_onChanged);
    super.dispose();
  }

  String _lastText = '';

  void _onChanged() {
    final String t = _ctl.text;
    if (t == _lastText) return;
    final bool emptinessChanged = t.isEmpty != _lastText.isEmpty ||
        (t.length > Outbox.maxChars) != (_lastText.length > Outbox.maxChars);
    _lastText = t;
    _drafts.set(widget.session, t);
    if (emptinessChanged || t.length > Outbox.maxChars - 2000) setState(() {});
  }

  void _say(String msg) {
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _send() async {
    if (_sending || widget.sendBlockedReason != null) return;
    final String text = _ctl.text;
    if (text.trim().isEmpty) return;
    if (text.length > Outbox.maxChars) {
      _say('El mensaje tiene ${text.length} caracteres; el máximo es ${Outbox.maxChars}. '
          'Divídelo en partes.');
      return;
    }
    setState(() => _sending = true);
    try {
      // After "Editar" on a failed message, the edited text takes its place
      // in the queue (ahead of the messages it held).
      final OutboxItem? editing = _outbox.editingFor(widget.session);
      await _outbox.enqueue(
        session: widget.session,
        pane: widget.pane(),
        text: text,
        transcriptBaseline: widget.baselineFor(text),
        replaces: editing?.id,
      );
      // Durable in the outbox: now the box may clear. Keep anything typed
      // during the (millisecond) wait.
      final String now = _ctl.text;
      _ctl.text = now.startsWith(text) ? now.substring(text.length).trimLeft() : now;
      await _drafts.setNow(widget.session, _ctl.text);
      widget.onSent();
    } on OutboxRejection catch (e) {
      _say(e.message);
    } catch (_) {
      _say('No pude guardar el mensaje. Sigue en el cuadro; inténtalo de nuevo.');
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final int len = _ctl.text.length;
    final bool tooLong = len > Outbox.maxChars;
    final String? blocked = widget.sendBlockedReason;
    final bool canSend = !_sending && _ctl.text.trim().isNotEmpty && !tooLong && blocked == null;
    return Material(
      color: Orbita.bar,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            if (_voice != null) DictationPanel(owner: widget.session, controller: _ctl),
            if (blocked != null)
              Padding(
                padding: const EdgeInsets.only(bottom: 4, left: 8, right: 8),
                child: Row(
                  children: <Widget>[
                    const Icon(Icons.front_hand_outlined, size: 14, color: Orbita.waiting),
                    const SizedBox(width: 6),
                    Expanded(
                      child: Text(
                        blocked,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(color: Orbita.waiting, fontSize: 12.5),
                      ),
                    ),
                  ],
                ),
              ),
            if (len > Outbox.maxChars - 2000)
              Padding(
                padding: const EdgeInsets.only(bottom: 4, right: 8),
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Text(
                    tooLong
                        ? '$len / ${Outbox.maxChars}: demasiado largo, divídelo'
                        : '$len / ${Outbox.maxChars}',
                    style: TextStyle(color: tooLong ? Orbita.err : Orbita.faint, fontSize: 12),
                  ),
                ),
              ),
            Row(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: <Widget>[
                if (_voice != null) DictationMic(owner: widget.session),
                Expanded(
                  child: TextField(
                    controller: _ctl,
                    minLines: 1,
                    maxLines: 6,
                    keyboardType: TextInputType.multiline,
                    textCapitalization: TextCapitalization.sentences,
                    decoration: const InputDecoration(hintText: 'Escribe al agente'),
                  ),
                ),
                const SizedBox(width: 6),
                SizedBox(
                  width: kTouch,
                  height: kTouch,
                  child: IconButton.filled(
                    tooltip: 'Enviar',
                    onPressed: canSend ? _send : null,
                    icon: _sending
                        ? const SizedBox(
                            width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2))
                        : const Icon(Icons.arrow_upward),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
