import 'package:flutter/material.dart';

import '../../core/models/ask_prompt.dart';
import '../../ui/theme.dart';

/// Permission prompt pinned above the composer: the question, its detail,
/// and one big button per option.
class AskCard extends StatelessWidget {
  const AskCard({super.key, required this.ask, required this.onAnswer, this.busy = false});

  final AskPrompt ask;
  final void Function(AskOption option) onAnswer;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    final List<String> detail = ask.detail.where((String l) => l.trim().isNotEmpty).toList();
    return Container(
      constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(context).height * 0.45),
      decoration: const BoxDecoration(
        color: Orbita.panel,
        border: Border(top: BorderSide(color: Orbita.waiting, width: 2)),
      ),
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            Text(
              ask.question.isEmpty ? 'El agente te pregunta' : ask.question,
              style: const TextStyle(color: Orbita.text, fontSize: 16, fontWeight: FontWeight.w600),
            ),
            if (detail.isNotEmpty) ...<Widget>[
              const SizedBox(height: 6),
              Text(
                detail.join('\n'),
                maxLines: 8,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12.5, color: Orbita.dim),
              ),
            ],
            const SizedBox(height: 10),
            for (final AskOption o in ask.options)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: OutlinedButton(
                  onPressed: busy ? null : () => onAnswer(o),
                  style: OutlinedButton.styleFrom(
                    minimumSize: const Size.fromHeight(52),
                    alignment: Alignment.centerLeft,
                    side: BorderSide(color: o.highlighted ? Orbita.brand : Orbita.line2),
                  ),
                  child: Text('${o.key}. ${o.label}', maxLines: 2, overflow: TextOverflow.ellipsis),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
