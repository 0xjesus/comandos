import 'package:flutter/material.dart';

import '../../core/models/ask_prompt.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// The menu open in the pane (a permission, a plan, a question), at the end of
/// the feed, drawn as the terminal draws it and as the web and desktop cards do:
/// its question tabs, the command or context, the question, and every row: the
/// number (or a box when it is a multiple choice), the label and the lines
/// under it; a field for «Type something», «Chat about this» and «Submit».
/// A tap answers that row (`ChatController.answerPrompt`: the desktop checks
/// it is still this menu and moves the cursor to the row with ↑/↓).
class AskCard extends StatelessWidget {
  const AskCard({super.key, required this.ask, required this.onAnswer, this.busy = false});

  final AskPrompt ask;

  /// A row chosen; [text] only for «Type something».
  final void Function(AskOption option, String? text) onAnswer;
  final bool busy;

  bool get _permission =>
      RegExp(r'proceed|permit|allow|want to|make this edit|run', caseSensitive: false).hasMatch(ask.question);

  @override
  Widget build(BuildContext context) {
    final List<String> detail = ask.detail.where((String l) => l.trim().isNotEmpty).toList();
    return Container(
      padding: const EdgeInsets.fromLTRB(15, 12, 15, 12),
      decoration: const BoxDecoration(
        color: Orbita.panel,
        border: Border.fromBorderSide(BorderSide(color: Orbita.brand)),
        borderRadius: BorderRadius.only(
          topLeft: Radius.circular(18),
          topRight: Radius.circular(18),
          bottomRight: Radius.circular(18),
          bottomLeft: Radius.circular(4),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          Text(
            _permission ? 'Pide permiso' : 'Te pregunta',
            style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, fontWeight: FontWeight.w600, color: Orbita.brand),
          ),
          const SizedBox(height: 6),
          if (ask.tabs.length > 1) ...<Widget>[
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: <Widget>[for (final (String label, bool done) in ask.tabs) _Tab(label: label, done: done)],
            ),
            const SizedBox(height: 8),
          ],
          if (detail.isNotEmpty) ...<Widget>[
            Container(
              padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
              decoration: BoxDecoration(
                color: Orbita.bg,
                borderRadius: BorderRadius.circular(8),
                border: Border.all(color: Orbita.line),
              ),
              child: Text(
                detail.join('\n'),
                maxLines: 10,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12.5, height: 1.4, color: Orbita.text),
              ),
            ),
            const SizedBox(height: 8),
          ],
          Container(
            padding: const EdgeInsets.fromLTRB(9, 1, 0, 1),
            decoration: const BoxDecoration(border: Border(left: BorderSide(color: Orbita.brand, width: 2))),
            child: Text(
              ask.question.isEmpty ? 'El agente espera tu respuesta' : ask.question,
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14.5, height: 1.4, color: Orbita.text, fontWeight: FontWeight.w600),
            ),
          ),
          const SizedBox(height: 10),
          for (final (int i, AskOption o) in ask.options.indexed) ...<Widget>[
            if (i > 0) const SizedBox(height: 6),
            if (o.kind == AskKind.free)
              _Free(option: o, enabled: !busy, onSend: (String t) => onAnswer(o, t))
            else
              _Option(option: o, onTap: busy ? null : () => onAnswer(o, null)),
          ],
          if (ask.multi) ...<Widget>[
            const SizedBox(height: 8),
            const Text('Marca las que quieras y toca Enviar.',
                style: TextStyle(fontFamily: Orbita.sans, fontSize: 12, color: Orbita.faint)),
          ],
        ],
      ),
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({required this.label, required this.done});
  final String label;
  final bool done;

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(999),
          border: Border.all(color: done ? Orbita.done.withValues(alpha: 0.5) : Orbita.line2),
        ),
        child: Text(
          '${done ? '✓ ' : ''}$label',
          style: TextStyle(fontFamily: Orbita.sans, fontSize: 12, fontWeight: FontWeight.w600, color: done ? Orbita.done : Orbita.dim),
        ),
      );
}

class _Option extends StatelessWidget {
  const _Option({required this.option, required this.onTap});
  final AskOption option;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final AskOption o = option;
    final bool submit = o.kind == AskKind.submit;
    final bool chat = o.kind == AskKind.chat;
    final String mark = o.checked == true
        ? '☑'
        : o.checked == false
            ? '☐'
            : o.key;
    return SelectionContainer.disabled(
      child: WebButton(
        onTap: onTap,
        tooltip: o.key.isEmpty ? o.label : '${o.key}. ${o.label}',
        face: submit ? Orbita.brand : Orbita.panel,
        ink: submit ? Orbita.onBrand : (chat ? Orbita.dim : Orbita.text),
        on: o.highlighted && !submit,
        radius: 10,
        lip: 2,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: submit
            ? const Center(
                child: Text('Enviar', style: TextStyle(fontFamily: Orbita.sans, fontSize: 14.5, fontWeight: FontWeight.w700, color: Orbita.onBrand)),
              )
            : Row(
                crossAxisAlignment: o.desc.isEmpty ? CrossAxisAlignment.center : CrossAxisAlignment.start,
                children: <Widget>[
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
                    decoration: BoxDecoration(
                      color: Orbita.panel2,
                      borderRadius: BorderRadius.circular(4),
                      border: Border.all(color: Orbita.line),
                    ),
                    child: Text(mark,
                        style: TextStyle(
                            fontFamily: Orbita.sans,
                            fontSize: o.checked != null ? 15 : 11,
                            fontWeight: FontWeight.w600,
                            color: o.checked == true ? Orbita.done : Orbita.dim)),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: <Widget>[
                        Text(
                          chat ? 'Hablar de esto en el chat' : o.label,
                          style: TextStyle(
                              fontFamily: Orbita.sans,
                              fontSize: 14,
                              height: 1.3,
                              fontWeight: FontWeight.w600,
                              color: chat ? Orbita.dim : Orbita.text),
                        ),
                        for (final String d in o.desc)
                          Padding(
                            padding: const EdgeInsets.only(top: 2),
                            child: Text(d,
                                style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, height: 1.35, color: Orbita.dim)),
                          ),
                      ],
                    ),
                  ),
                ],
              ),
      ),
    );
  }
}

/// «Type something»: a field and Enviar; the text answers on that row.
class _Free extends StatefulWidget {
  const _Free({required this.option, required this.enabled, required this.onSend});
  final AskOption option;
  final bool enabled;
  final void Function(String text) onSend;

  @override
  State<_Free> createState() => _FreeState();
}

class _FreeState extends State<_Free> {
  final TextEditingController _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _send() {
    final String t = _text.text.trim();
    if (t.isNotEmpty && widget.enabled) widget.onSend(t);
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.fromLTRB(12, 2, 4, 2),
      decoration: BoxDecoration(
        color: Orbita.bg,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: Orbita.line2),
      ),
      child: Row(
        children: <Widget>[
          Text(widget.option.key.isEmpty ? '✎' : widget.option.key,
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 11, fontWeight: FontWeight.w600, color: Orbita.dim)),
          const SizedBox(width: 10),
          Expanded(
            child: TextField(
              controller: _text,
              enabled: widget.enabled,
              minLines: 1,
              maxLines: 4,
              maxLength: 2000,
              textInputAction: TextInputAction.send,
              onSubmitted: (_) => _send(),
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, color: Orbita.text),
              decoration: const InputDecoration(
                hintText: 'Escribe tu respuesta…',
                counterText: '',
                border: InputBorder.none,
                isDense: true,
              ),
            ),
          ),
          TextButton(
            onPressed: widget.enabled ? _send : null,
            child: const Text('Enviar'),
          ),
        ],
      ),
    );
  }
}
