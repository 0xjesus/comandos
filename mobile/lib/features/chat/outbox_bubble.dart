import 'package:flutter/material.dart';

import '../../core/outbox/outbox_item.dart';
import '../../ui/theme.dart';

/// A message of ours still in the outbox, shown under the transcript with
/// its state: reloj (en cola / enviando), ✓ (entregado), ! (falló), ? (no sé).
class OutboxBubble extends StatelessWidget {
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
  Widget build(BuildContext context) {
    final bool promptOpen = item.waitCode == WaitCode.promptOpen;
    final (IconData icon, Color color, String status) = switch (item.state) {
      OutboxState.queued when blocked => (Icons.schedule, Orbita.faint, 'En cola: espera al anterior'),
      OutboxState.queued || OutboxState.sending when promptOpen =>
        (Icons.front_hand_outlined, Orbita.waiting, kWaitingForPrompt),
      OutboxState.queued => (Icons.schedule, Orbita.working, item.lastError ?? 'En cola'),
      OutboxState.sending => (Icons.schedule, Orbita.working, 'Enviando…'),
      OutboxState.delivered => (Icons.check, Orbita.done, 'Entregado'),
      OutboxState.failed => (Icons.error_outline, Orbita.err, outboxFailureMessage(item.failCode)),
      OutboxState.uncertain => (Icons.help_outline, Orbita.waiting, outboxFailureMessage(FailCode.uncertain)),
    };
    final bool needsHuman = item.needsHuman;
    return Align(
      alignment: Alignment.centerRight,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(36, 6, 12, 6),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: Orbita.panel2,
            borderRadius: BorderRadius.circular(16),
            border: Border.all(color: needsHuman ? color : Orbita.line),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 9, 12, 6),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: <Widget>[
                Text(
                  item.text,
                  maxLines: 12,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: Orbita.text, fontSize: 15, height: 1.4),
                ),
                const SizedBox(height: 6),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    Flexible(
                      child: Text(
                        status,
                        textAlign: TextAlign.end,
                        style: TextStyle(color: color, fontSize: 12.5),
                      ),
                    ),
                    const SizedBox(width: 4),
                    Icon(icon, size: 15, color: color),
                  ],
                ),
                if (needsHuman)
                  Wrap(
                    alignment: WrapAlignment.end,
                    spacing: 4,
                    children: <Widget>[
                      if (item.state == OutboxState.uncertain)
                        TextButton(onPressed: onArrived, child: const Text('Sí llegó')),
                      TextButton(
                        onPressed: onRetry,
                        child: Text(item.state == OutboxState.uncertain ? 'Reenviar' : 'Reintentar'),
                      ),
                      TextButton(onPressed: onEdit, child: const Text('Editar')),
                      TextButton(
                        onPressed: onDiscard,
                        style: TextButton.styleFrom(foregroundColor: Orbita.err),
                        child: const Text('Descartar'),
                      ),
                    ],
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Above the composer while a failed message is being edited: the later
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
    return Material(
      color: Orbita.bar,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 2, 8, 0),
        child: Row(
          children: <Widget>[
            const Icon(Icons.pause_circle_outline, size: 15, color: Orbita.waiting),
            const SizedBox(width: 6),
            Expanded(
              child: Text(text, style: const TextStyle(color: Orbita.waiting, fontSize: 12.5)),
            ),
            TextButton(onPressed: onDiscard, child: const Text('Descartar edición')),
          ],
        ),
      ),
    );
  }
}
