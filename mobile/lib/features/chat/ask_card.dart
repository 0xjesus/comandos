import 'package:flutter/material.dart';

import '../../core/models/ask_prompt.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// The permission prompt (or any numbered menu) open in the pane, at the end
/// of the feed. The web's chat leaves it to the terminal and the dock's
/// 1 / 2 keys; the phone's touch card of the session feed (`card_html` with
/// `touch`, `.fs-peek.touch`) is what the web offers to answer it with a tap,
/// so the same pieces are used here: the question as a `.fs-q` quote with the
/// brand rule, the detail lines, and one 44 px `.fs-opt` row per option with
/// its key in a `kbd`. The answer is bound to this prompt (see
/// `ChatController.answerPrompt`).
class AskCard extends StatelessWidget {
  const AskCard({super.key, required this.ask, required this.onAnswer, this.busy = false});

  final AskPrompt ask;
  final void Function(AskOption option) onAnswer;
  final bool busy;

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
          const Text(
            'Te pregunta',
            style: TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, fontWeight: FontWeight.w600, color: Orbita.brand),
          ),
          const SizedBox(height: 6),
          if (detail.isNotEmpty) ...<Widget>[
            Text(
              detail.join('\n'),
              maxLines: 8,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12.5, height: 1.4, color: Orbita.dim),
            ),
            const SizedBox(height: 6),
          ],
          Container(
            padding: const EdgeInsets.fromLTRB(9, 1, 0, 1),
            decoration: const BoxDecoration(border: Border(left: BorderSide(color: Orbita.brand, width: 2))),
            child: Text(
              ask.question.isEmpty ? 'El agente espera tu respuesta' : ask.question,
              style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, height: 1.4, color: Orbita.text),
            ),
          ),
          const SizedBox(height: 10),
          for (final (int i, AskOption o) in ask.options.indexed) ...<Widget>[
            if (i > 0) const SizedBox(height: 6),
            _Option(option: o, onTap: busy ? null : () => onAnswer(o)),
          ],
        ],
      ),
    );
  }
}

class _Option extends StatelessWidget {
  const _Option({required this.option, required this.onTap});
  final AskOption option;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return SelectionContainer.disabled(
      child: WebButton(
        onTap: onTap,
        tooltip: '${option.key}. ${option.label}',
        face: Orbita.panel,
        ink: Orbita.text,
        on: option.highlighted,
        radius: 10,
        lip: 2,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        child: Row(
          children: <Widget>[
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
              decoration: BoxDecoration(
                color: Orbita.panel2,
                borderRadius: BorderRadius.circular(4),
                border: Border.all(color: Orbita.line),
              ),
              child: Text(option.key,
                  style: const TextStyle(fontFamily: Orbita.sans, fontSize: 11, fontWeight: FontWeight.w600, color: Orbita.dim)),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                option.label,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, height: 1.3, fontWeight: FontWeight.w400, color: Orbita.text),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
