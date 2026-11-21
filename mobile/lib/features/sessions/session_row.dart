import 'package:flutter/material.dart';

import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/widgets/state_chip.dart';

/// One session card. Immutable and cheap; the list reuses the same instance
/// while the row's signature does not change, so Flutter skips its rebuild.
class SessionRow extends StatelessWidget {
  const SessionRow({
    super.key,
    required this.summary,
    required this.when,
    required this.unsettled,
    required this.onTap,
  });

  final SessionSummary summary;

  /// Relative time, already formatted ("hace 3 min").
  final String when;

  /// Outbox items for this session that are not delivered yet.
  final int unsettled;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final SessionSummary s = summary;
    final String excerpt = s.state == SessionState.ask && s.ask != null
        ? s.ask!.question
        : (s.reply.isNotEmpty ? s.reply : s.prompt);
    return InkWell(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Row(
              children: <Widget>[
                if (s.unread) ...<Widget>[
                  const _Dot(),
                  const SizedBox(width: 8),
                ],
                Expanded(
                  child: Text(
                    s.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: Orbita.text,
                      fontSize: 16,
                      fontWeight: s.unread ? FontWeight.w700 : FontWeight.w600,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                Text(when, style: const TextStyle(color: Orbita.faint, fontSize: 12)),
              ],
            ),
            const SizedBox(height: 6),
            Row(
              children: <Widget>[
                StateChip(s.state),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    s.agentModel,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(color: Orbita.dim, fontSize: 13),
                  ),
                ),
                if (unsettled > 0) ...<Widget>[
                  const Icon(Icons.schedule, size: 14, color: Orbita.working),
                  const SizedBox(width: 3),
                  Text('$unsettled', style: const TextStyle(color: Orbita.working, fontSize: 12)),
                ],
              ],
            ),
            if (excerpt.isNotEmpty) ...<Widget>[
              const SizedBox(height: 6),
              Text(
                excerpt,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: Orbita.dim, fontSize: 14, height: 1.3),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _Dot extends StatelessWidget {
  const _Dot();

  @override
  Widget build(BuildContext context) => const SizedBox(
        width: 8,
        height: 8,
        child: DecoratedBox(
          decoration: BoxDecoration(color: Orbita.brand, shape: BoxShape.circle),
        ),
      );
}

/// Section title row.
class SectionHeader extends StatelessWidget {
  const SectionHeader(this.title, this.count, {super.key});

  final String title;
  final int count;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 18, 16, 6),
      child: Text(
        '$title · $count',
        style: const TextStyle(
          color: Orbita.faint,
          fontSize: 12,
          fontWeight: FontWeight.w700,
          letterSpacing: 0.6,
        ),
      ),
    );
  }
}
