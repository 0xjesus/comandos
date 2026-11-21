import 'package:flutter/material.dart';

import '../../core/models/session_feed.dart';
import '../theme.dart';

/// Small coloured label for a session state.
class StateChip extends StatelessWidget {
  const StateChip(this.state, {super.key});

  final SessionState state;

  static (String, Color) describe(SessionState s) {
    switch (s) {
      case SessionState.ask:
        return ('Te pregunta', Orbita.waiting);
      case SessionState.limit:
        return ('Sin cuota', Orbita.waiting);
      case SessionState.work:
        return ('Trabajando', Orbita.working);
      case SessionState.idle:
        return ('Lista', Orbita.done);
    }
  }

  @override
  Widget build(BuildContext context) {
    final (String label, Color color) = describe(state);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        child: Text(
          label,
          style: TextStyle(color: color, fontSize: 12, fontWeight: FontWeight.w600),
        ),
      ),
    );
  }
}
