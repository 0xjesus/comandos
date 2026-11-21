import 'package:flutter/material.dart';

import '../../core/outbox/outbox_item.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'message_view.dart';

/// A message of ours still in the outbox, as the web draws a message on its
/// way (`.cv-me.pending`: the brand bubble at 60 % with the text as typed),
/// with its state under it and, when the person must decide, the actions:
/// Sí llegó / Reenviar (uncertain), Reintentar (failed), Editar, Descartar.
class OutboxBubble extends StatefulWidget {
  const OutboxBubble({
    super.key,
    required this.item,
    required this.blocked,
    required this.onRetry,
    required this.onEdit,
    required this.onDiscard,
    required this.onArrived,
  });

  final OutboxItem item;

  /// Waiting behind an earlier message of the same session.
  final bool blocked;
  final VoidCallback onRetry;
  final VoidCallback onEdit;
  final VoidCallback onDiscard;
  final VoidCallback onArrived;

  @override
  State<OutboxBubble> createState() => _OutboxBubbleState();
}

class _OutboxBubbleState extends State<OutboxBubble> {
  /// «Descartar» asks once more, inline (no dialog on the web either).
  bool _confirm = false;

  @override
  Widget build(BuildContext context) {
    final OutboxItem item = widget.item;
    final bool promptOpen = item.waitCode == WaitCode.promptOpen;
    final (Color color, String status) = switch (item.state) {
      OutboxState.queued when widget.blocked => (Orbita.faint, 'En cola: espera al anterior'),
      OutboxState.queued || OutboxState.sending when promptOpen => (Orbita.waiting, kWaitingForPrompt),
      OutboxState.queued => (Orbita.dim, item.lastError ?? 'En cola'),
      OutboxState.sending => (Orbita.dim, 'Enviando…'),
      OutboxState.delivered => (Orbita.done, 'Entregado ✓'),
      OutboxState.failed => (Orbita.err, outboxFailureMessage(item.failCode)),
      OutboxState.uncertain => (Orbita.waiting, outboxFailureMessage(FailCode.uncertain)),
    };
    final bool needsHuman = item.needsHuman;
    final bool faded = item.isPending || item.state == OutboxState.delivered;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        FractionallySizedBox(
          widthFactor: .85,
          alignment: Alignment.centerRight,
          child: Align(
            alignment: Alignment.centerRight,
            child: Opacity(
              opacity: faded ? .6 : 1,
              child: Container(
                padding: const EdgeInsets.fromLTRB(14, 10, 14, 10),
                decoration: BoxDecoration(
                  color: Orbita.brand,
                  border: needsHuman ? Border.all(color: color, width: 2) : null,
                  borderRadius: const BorderRadius.only(
                    topLeft: Radius.circular(18),
                    topRight: Radius.circular(18),
                    bottomLeft: Radius.circular(18),
                    bottomRight: Radius.circular(4),
                  ),
                ),
                child: Text(
                  item.text,
                  maxLines: 12,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, height: 1.5, color: Colors.white),
                ),
              ),
            ),
          ),
        ),
        const SizedBox(height: 4),
        Padding(
          padding: const EdgeInsets.only(right: 4),
          child: Text(
            status,
            textAlign: TextAlign.end,
            style: TextStyle(fontFamily: Orbita.sans, fontSize: 12, fontWeight: FontWeight.w600, color: color),
          ),
        ),
        if (needsHuman) ...<Widget>[
          const SizedBox(height: 6),
          Wrap(
            alignment: WrapAlignment.end,
            spacing: 6,
            runSpacing: 6,
            children: _confirm
                ? <Widget>[
                    Padding(
                      padding: const EdgeInsets.only(top: 12),
                      child: Text(
                        item.state == OutboxState.uncertain ? '¿Descartar? Puede que ya haya llegado.' : '¿Descartar? No se enviará.',
                        style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.dim),
                      ),
                    ),
                    CvAct(label: 'No', onTap: () => setState(() => _confirm = false)),
                    _danger('Descartar', () {
                      setState(() => _confirm = false);
                      widget.onDiscard();
                    }),
                  ]
                : <Widget>[
                    if (item.state == OutboxState.uncertain) CvAct(label: 'Sí llegó', onTap: widget.onArrived),
                    CvAct(label: item.state == OutboxState.uncertain ? 'Reenviar' : 'Reintentar', onTap: widget.onRetry),
                    CvAct(label: 'Editar', onTap: widget.onEdit),
                    _danger('Descartar', () => setState(() => _confirm = true)),
                  ],
          ),
        ],
      ],
    );
  }

  Widget _danger(String label, VoidCallback onTap) => WebButton(
        onTap: onTap,
        face: Orbita.bg,
        ink: Orbita.err,
        radius: 12,
        lip: 2,
        padding: const EdgeInsets.symmetric(horizontal: 14),
        child: Text(label, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13.5, fontWeight: FontWeight.w600)),
      );
}

/// Over the composer while a failed message is being edited: the later
/// messages of the session wait for the edited text (or for "Descartar").
class EditHoldNote extends StatelessWidget {
  const EditHoldNote({super.key, required this.held, required this.onDiscard});

  final int held;
  final VoidCallback onDiscard;

  @override
  Widget build(BuildContext context) {
    final String text = held == 0
        ? 'Editando un mensaje que no se envió'
        : held == 1
            ? '1 mensaje en espera hasta que envíes el texto editado'
            : '$held mensajes en espera hasta que envíes el texto editado';
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 6, 8, 2),
      child: Row(
        children: <Widget>[
          Expanded(
            child: Text(text, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, color: Orbita.waiting)),
          ),
          WebButton(
            onTap: onDiscard,
            face: Orbita.bg,
            ink: Orbita.text,
            radius: 10,
            lip: 2,
            minHeight: 36,
            child: const Text('Descartar edición', style: TextStyle(fontFamily: Orbita.sans, fontSize: 13, fontWeight: FontWeight.w600)),
          ),
        ],
      ),
    );
  }
}
