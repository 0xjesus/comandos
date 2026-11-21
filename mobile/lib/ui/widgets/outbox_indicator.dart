import 'package:flutter/material.dart';

import '../../core/outbox/outbox.dart';
import '../format.dart';
import '../theme.dart';
import 'banner_strip.dart';

/// Always-visible strip when anything is pending or needs the person:
/// "2 mensajes por enviar" / "1 mensaje necesita tu decisión". Tap opens that
/// session. Accent `--err` when a decision is needed, `--working` otherwise.
class OutboxIndicator extends StatelessWidget {
  const OutboxIndicator({super.key, required this.outbox, required this.onOpen});

  final Outbox outbox;
  final void Function(String session) onOpen;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: outbox,
      builder: (BuildContext context, Widget? _) {
        final int attention = outbox.attentionCount;
        final int pending = outbox.pendingCount;
        final String? session = outbox.firstUnsettledSession;
        if (session == null || (attention == 0 && pending == 0)) {
          return const SizedBox.shrink();
        }
        final String text = attention > 0
            ? '${plural(attention, 'mensaje necesita', 'mensajes necesitan')} tu decisión'
                '${pending > 0 ? ' · ${plural(pending, 'por enviar', 'por enviar')}' : ''}'
            : plural(pending, 'mensaje por enviar', 'mensajes por enviar');
        return BannerStrip(
          text: text,
          color: attention > 0 ? Orbita.err : Orbita.working,
          onTap: () => onOpen(session),
        );
      },
    );
  }
}
