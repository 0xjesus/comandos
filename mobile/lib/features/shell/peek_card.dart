import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../sessions/feed_row.dart';
import '../sessions/sections.dart';

/// The phone's B4 preview card (`.fs-peek.touch`), opened by a long press on a tab
/// chip: the session's feed row and, over a permission menu, its options as 44 px
/// rows. Tapping an option answers it like a notification (`POST /key` digit, then
/// Enter 250 ms later); tapping elsewhere on the card opens the session; tapping
/// outside closes it. Placed under the nav, 8 px from both sides.
Future<void> showPeekCard(
  BuildContext context, {
  required SessionSummary entry,
  required double now,
  required double top,
  required VoidCallback onOpen,
}) {
  return showGeneralDialog<void>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Cerrar',
    barrierColor: Colors.transparent,
    transitionDuration: Duration.zero,
    pageBuilder: (BuildContext context, Animation<double> a, Animation<double> b) =>
        _PeekCard(entry: entry, now: now, top: top, onOpen: onOpen),
  );
}

class _PeekCard extends StatelessWidget {
  const _PeekCard({required this.entry, required this.now, required this.top, required this.onOpen});

  final SessionSummary entry;
  final double now;
  final double top;
  final VoidCallback onOpen;

  List<AskOption> get _options => entry.state == SessionState.ask
      ? <AskOption>[
          for (final AskOption o in entry.ask?.options ?? const <AskOption>[])
            if (RegExp(r'^[1-9]$').hasMatch(o.key)) o,
        ]
      : const <AskOption>[];

  void _choose(BuildContext context, String digit) {
    Navigator.of(context).pop();
    final Future<void>? sent =
        sl.maybe<Connection>()?.api?.answerAsk(session: entry.session, pane: entry.pane, option: digit);
    if (sent == null) return;
    final BuildContext? root = Navigator.maybeOf(context, rootNavigator: true)?.context;
    unawaited(sent.catchError((Object _) {
      if (root != null && root.mounted) showWebToast(root, 'No se pudo enviar la tecla', error: true);
    }));
  }

  @override
  Widget build(BuildContext context) {
    final List<AskOption> options = _options;
    return Material(
      type: MaterialType.transparency,
      child: Stack(
        children: <Widget>[
          Positioned(
            top: top,
            left: 8,
            right: 8,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: () {
                Navigator.of(context).pop();
                onOpen();
              },
              child: Container(
                padding: const EdgeInsets.fromLTRB(4, 8, 4, 10),
                decoration: BoxDecoration(
                  color: Orbita.panel2,
                  borderRadius: BorderRadius.circular(14),
                  border: Border.all(color: Orbita.line2),
                  boxShadow: <BoxShadow>[
                    BoxShadow(color: Orbita.bg.withValues(alpha: .6), blurRadius: 36, offset: const Offset(0, 14)),
                  ],
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    IgnorePointer(
                      child: FeedRow(entry: entry, ago: feedAgo(now, entry.ts), quoteLines: 4, onTap: () {}),
                    ),
                    if (options.isNotEmpty)
                      Padding(
                        padding: const EdgeInsets.fromLTRB(4, 4, 4, 0),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: <Widget>[
                            for (final AskOption o in options) ...<Widget>[
                              if (o != options.first) const SizedBox(height: 6),
                              _Option(option: o, onTap: () => _choose(context, o.key)),
                            ],
                          ],
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// `.fs-opt`: 44 px row, `--panel` face, 1 px `--line`, radius 10, `<kbd>` + label.
class _Option extends StatelessWidget {
  const _Option({required this.option, required this.onTap});

  final AskOption option;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          decoration: BoxDecoration(
            color: Orbita.panel,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: Orbita.line),
          ),
          child: Row(
            children: <Widget>[
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 5),
                margin: const EdgeInsets.only(right: 3),
                decoration: BoxDecoration(
                  color: Orbita.panel2,
                  borderRadius: BorderRadius.circular(4),
                  border: Border.all(color: Orbita.line),
                ),
                child: Text(option.key,
                    style: const TextStyle(
                        fontFamily: Orbita.sans, fontSize: 10, fontWeight: FontWeight.w600, color: Orbita.dim)),
              ),
              const SizedBox(width: 7),
              Expanded(
                child: Text(
                  option.label.split(RegExp(r'\s+')).join(' '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, height: 1.3, color: Orbita.text),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
