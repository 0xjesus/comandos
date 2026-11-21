import 'package:flutter/material.dart';

import '../../core/locator.dart';
import '../../core/outbox/drafts.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../voice/chat_voice.dart';
import '../voice/voice.dart';

/// Shown over the composer while a permission prompt is open.
const String kAnswerPromptFirst = 'Responde primero el permiso de arriba';

/// `#cv-dock .cv-compose`: the growing text box (up to about six lines) and the
/// round brand send button with the arrow; the mic (dictation) sits left of
/// the send button. The text is kept as this session's draft while typing;
/// sending puts it in the outbox and the box clears only once that is
/// durable. A failure keeps the text where it is.
///
/// An empty box makes the arrow a plain Enter ([onEmptySend]), as on the web:
/// it confirms what the agent's menu has highlighted.
class Composer extends StatefulWidget {
  const Composer({
    super.key,
    required this.session,
    required this.pane,
    required this.baselineFor,
    required this.onSent,
    required this.controller,
    this.sendBlockedReason,
    this.hint = 'Escribe a la sesión…',
    this.onEmptySend,
    this.focusNode,
    this.onSay,
  });

  final String session;
  final String? Function() pane;
  final int? Function(String text) baselineFor;
  final VoidCallback onSent;

  /// Owned by the session view so "Editar" can put text back.
  final TextEditingController controller;

  /// When set, sending is refused (typing and the draft still work) and this
  /// one-line reason shows over the box.
  final String? sendBlockedReason;

  /// «Escribe a 1 · Claude · fable-5.2…» on a split.
  final String hint;

  /// The arrow with an empty box: Enter in the pane.
  final VoidCallback? onEmptySend;
  final FocusNode? focusNode;

  /// Shows a short message (defaults to a SnackBar).
  final void Function(String message)? onSay;

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
  void didUpdateWidget(Composer old) {
    super.didUpdateWidget(old);
    if (!identical(old.controller, widget.controller)) {
      old.controller.removeListener(_onChanged);
      widget.controller.addListener(_onChanged);
      _lastText = widget.controller.text;
    }
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
    final bool redraw = (t.length > Outbox.maxChars - 2000) != (_lastText.length > Outbox.maxChars - 2000) ||
        t.length > Outbox.maxChars - 2000;
    _lastText = t;
    _drafts.set(widget.session, t);
    if (redraw) setState(() {});
  }

  void _say(String msg) {
    final void Function(String)? say = widget.onSay;
    if (say != null) return say(msg);
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _send() async {
    if (_sending) return;
    final String text = _ctl.text;
    if (widget.sendBlockedReason != null) {
      _say(widget.sendBlockedReason!);
      return;
    }
    if (text.trim().isEmpty) {
      widget.onEmptySend?.call();
      return;
    }
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
    final double maxBox = (MediaQuery.sizeOf(context).height * .30).clamp(44, 7.8 * 16 + 24).toDouble();
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        if (_voice != null) DictationPanel(owner: widget.session, controller: _ctl),
        if (blocked != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 4, 12, 2),
            child: Text(
              blocked,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontFamily: Orbita.sans, color: Orbita.waiting, fontSize: 12.5, fontWeight: FontWeight.w600),
            ),
          ),
        if (len > Outbox.maxChars - 2000)
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 2, 12, 0),
            child: Text(
              tooLong ? '$len / ${Outbox.maxChars}: demasiado largo, divídelo' : '$len / ${Outbox.maxChars}',
              textAlign: TextAlign.right,
              style: TextStyle(fontFamily: Orbita.sans, color: tooLong ? Orbita.err : Orbita.faint, fontSize: 12),
            ),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(8, 2, 8, 8),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: <Widget>[
              Expanded(
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: 44, maxHeight: maxBox),
                  child: TextField(
                    controller: _ctl,
                    focusNode: widget.focusNode,
                    minLines: 1,
                    maxLines: 6,
                    keyboardType: TextInputType.multiline,
                    textInputAction: TextInputAction.newline,
                    textCapitalization: TextCapitalization.sentences,
                    cursorColor: Orbita.brand,
                    style: const TextStyle(fontFamily: Orbita.sans, fontSize: 16, height: 1.3, color: Orbita.text),
                    decoration: InputDecoration(
                      hintText: widget.hint,
                      hintMaxLines: 1,
                      hintStyle: TextStyle(fontFamily: Orbita.sans, fontSize: 16, height: 1.3, color: Orbita.text.withValues(alpha: .5)),
                      isDense: true,
                      filled: true,
                      fillColor: Orbita.bg,
                      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 11),
                      border: _border(Orbita.line2),
                      enabledBorder: _border(Orbita.line2),
                      focusedBorder: _border(Orbita.brand),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              if (_voice != null) DictationMic(owner: widget.session),
              WebButton(
                tooltip: 'Enviar; vacío envía Enter',
                onTap: _send,
                brand: true,
                size: 44,
                radius: 22,
                lip: 2,
                child: _sending
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2.5, color: Colors.white, backgroundColor: Colors.white30),
                      )
                    : const WebIcon(WebIcons.send, size: 20, color: Colors.white, semanticLabel: 'Enviar'),
              ),
            ],
          ),
        ),
      ],
    );
  }

  static OutlineInputBorder _border(Color c) =>
      OutlineInputBorder(borderRadius: BorderRadius.circular(14), borderSide: BorderSide(color: c));
}
