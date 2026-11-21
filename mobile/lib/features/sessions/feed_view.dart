import 'package:flutter/material.dart';

import '../../core/api/api_errors.dart';
import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'feed_row.dart';
import 'sections.dart';
import 'sessions_controller.dart';

/// `#command-sidebar .cs-feed`: filter chips, the pinned «TE ESPERA · N» and the
/// time-grouped scroll of [FeedRow]s, from the shared [SessionsController].
class FeedView extends StatefulWidget {
  const FeedView({
    super.key,
    required this.controller,
    required this.chip,
    required this.onOpen,
    this.active,
  });

  final SessionsController controller;
  final ValueNotifier<FeedChip> chip;
  final void Function(SessionSummary entry) onOpen;

  /// Session open right now (its row gets `.on`).
  final String? active;

  @override
  State<FeedView> createState() => _FeedViewState();
}

class _FeedViewState extends State<FeedView> {
  /// session → (signature, widget): an unchanged row is the same instance.
  final Map<String, (String, Widget)> _rows = <String, (String, Widget)>{};

  Widget _row(SessionSummary e, double now) {
    final String ago = feedAgo(now, e.ts);
    final bool active = e.session == widget.active;
    final String sig = '${SessionsController.feedSignature(e)}\u0002$ago\u0002$active';
    final (String, Widget)? cached = _rows[e.session];
    if (cached != null && cached.$1 == sig) return cached.$2;
    final Widget w = FeedRow(
      key: ValueKey<String>('r:${e.session}'),
      entry: e,
      ago: ago,
      active: active,
      onTap: () => widget.onOpen(e),
    );
    _rows[e.session] = (sig, w);
    return w;
  }

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: Orbita.panel,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 0),
        child: ListenableBuilder(
          listenable: Listenable.merge(<Listenable>[widget.controller, widget.chip]),
          builder: (BuildContext context, Widget? _) => _build(),
        ),
      ),
    );
  }

  Widget _build() {
    final SessionFeed? feed = widget.controller.feed;
    final ApiException? error = widget.controller.error;
    if (feed == null) {
      if (error == null) return const SizedBox.expand();
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Text(error.userMessage,
                  textAlign: TextAlign.center,
                  style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, color: Orbita.faint)),
              const SizedBox(height: 14),
              WebButton(
                onTap: widget.controller.refresh,
                lip: 2,
                child: const Text('Reintentar'),
              ),
            ],
          ),
        ),
      );
    }
    final FeedChip chip = widget.chip.value;
    final double dayStart = localDayStart(feed.now);
    final FeedLayout lay = feedLayout(feed.sessions, chip, feed.now, dayStart);
    final FeedCounts counts = feedCounts(feed.sessions);
    final Set<String> live = <String>{for (final SessionSummary s in feed.sessions) s.session};
    _rows.removeWhere((String k, (String, Widget) _) => !live.contains(k));

    final List<Object> items = <Object>[];
    for (final (FeedGroup g, List<SessionSummary> rows) in lay.groups) {
      items.add(g);
      items.addAll(rows);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _Chips(counts: counts, chip: chip, onChip: (FeedChip c) => widget.chip.value = c),
        if (lay.pinned.isNotEmpty)
          LayoutBuilder(
            builder: (BuildContext context, BoxConstraints c) => ConstrainedBox(
              constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(context).height * .3),
              child: Container(
                margin: const EdgeInsets.only(bottom: 4),
                padding: const EdgeInsets.only(bottom: 6),
                decoration: const BoxDecoration(border: Border(bottom: BorderSide(color: Orbita.line))),
                child: ListView.builder(
                  shrinkWrap: true,
                  padding: EdgeInsets.zero,
                  itemCount: lay.pinned.length + 1,
                  itemBuilder: (BuildContext context, int i) => i == 0
                      ? FeedGroupHeader('Te espera · ${lay.pinned.length}', first: true, pinned: true)
                      : _row(lay.pinned[i - 1], feed.now),
                ),
              ),
            ),
          ),
        Expanded(
          child: lay.isEmpty
              ? const Padding(
                  padding: EdgeInsets.fromLTRB(8, 24, 8, 24),
                  child: Text(
                    'Nada por aquí.',
                    textAlign: TextAlign.center,
                    style: TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, color: Orbita.faint),
                  ),
                )
              : ListView.builder(
                  padding: const EdgeInsets.only(bottom: 10),
                  itemCount: items.length,
                  itemBuilder: (BuildContext context, int i) {
                    final Object it = items[i];
                    if (it is FeedGroup) {
                      return FeedGroupHeader(it.label, key: ValueKey<String>('g:${it.name}'), first: i == 0);
                    }
                    return _row(it as SessionSummary, feed.now);
                  },
                ),
        ),
      ],
    );
  }
}

/// `.fs-chips`: «Todas 42 · Sin leer 2 · Permisos 0 · Trabajando 3».
class _Chips extends StatelessWidget {
  const _Chips({required this.counts, required this.chip, required this.onChip});

  final FeedCounts counts;
  final FeedChip chip;
  final ValueChanged<FeedChip> onChip;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: <Widget>[
            for (final FeedChip c in FeedChip.values) ...<Widget>[
              if (c != FeedChip.all) const SizedBox(width: 6),
              WebChip(
                style: WebChipStyle.filter,
                on: c == chip,
                onTap: () => onChip(c),
                child: Text.rich(TextSpan(children: <InlineSpan>[
                  TextSpan(text: c.label),
                  TextSpan(
                    text: '  ${counts.of(c)}',
                    style: TextStyle(color: c == chip ? Orbita.dim : Orbita.faint),
                  ),
                ])),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// `.fs-gh`: 600 10.5 px, uppercase, letter-spaced .08em, `--faint` (`--brand` pinned).
class FeedGroupHeader extends StatelessWidget {
  const FeedGroupHeader(this.text, {super.key, this.first = false, this.pinned = false});

  final String text;
  final bool first;
  final bool pinned;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.fromLTRB(4, first ? 2 : 12, 4, 6),
      child: Text(
        text.toUpperCase(),
        style: TextStyle(
          fontFamily: Orbita.sans,
          fontSize: 10.5,
          fontWeight: FontWeight.w600,
          letterSpacing: .84,
          color: pinned ? Orbita.brand : Orbita.faint,
        ),
      ),
    );
  }
}
