import 'package:flutter/material.dart';

import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../sessions/feed_view.dart';
import '../sessions/sections.dart';
import '../sessions/sessions_controller.dart';

/// `#command-sidebar .cs-tools` + content: «Sesiones» (with the violet «te
/// espera» badge) | «Consumos» | the servers key, then the feed or [usage].
class SidePanelBody extends StatelessWidget {
  const SidePanelBody({
    super.key,
    required this.feed,
    required this.chip,
    required this.usageTab,
    required this.onOpen,
    required this.onServers,
    required this.usage,
    this.active,
  });

  final SessionsController feed;
  final ValueNotifier<FeedChip> chip;
  final ValueNotifier<bool> usageTab;
  final void Function(SessionSummary entry) onOpen;
  final VoidCallback onServers;
  final String? active;

  /// The «Consumos» view (agent FORMS' `UsageSummary`).
  final Widget usage;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: Orbita.panel,
      child: ValueListenableBuilder<bool>(
        valueListenable: usageTab,
        builder: (BuildContext context, bool onUsage, Widget? _) => Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              decoration: const BoxDecoration(
                color: Orbita.panel,
                border: Border(bottom: BorderSide(color: Orbita.line)),
              ),
              child: Row(
                children: <Widget>[
                  Expanded(
                    child: _Tool(
                      key: const ValueKey<String>('tb-sessions'),
                      on: !onUsage,
                      onTap: () => usageTab.value = false,
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: <Widget>[
                          const Flexible(child: Text('Sesiones', overflow: TextOverflow.ellipsis)),
                          ListenableBuilder(
                            listenable: feed,
                            builder: (BuildContext context, Widget? _) {
                              final SessionFeed? f = feed.feed;
                              final int n = f == null ? 0 : feedCounts(f.sessions).waiting;
                              if (n == 0) return const SizedBox.shrink();
                              return Padding(
                                padding: const EdgeInsets.only(left: 6),
                                child: _Badge(n),
                              );
                            },
                          ),
                        ],
                      ),
                    ),
                  ),
                  const SizedBox(width: 4),
                  Expanded(
                    child: _Tool(
                      key: const ValueKey<String>('tb-usage'),
                      on: onUsage,
                      onTap: () => usageTab.value = true,
                      child: const Text('Consumos', overflow: TextOverflow.ellipsis),
                    ),
                  ),
                  const SizedBox(width: 4),
                  SizedBox(
                    width: 44,
                    child: _Tool(
                      key: const ValueKey<String>('tb-servers'),
                      on: false,
                      onTap: onServers,
                      tooltip: 'Servidores',
                      child: const WebIcon(WebIcons.server, size: 15),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: onUsage
                  ? usage
                  : FeedView(controller: feed, chip: chip, onOpen: onOpen, active: active),
            ),
          ],
        ),
      ),
    );
  }
}

/// `.cs-tools .tb`: 38 px (phone), radius 8, 12 px 600, `--dim`; on = `--brand`
/// text, brand-tinted border and face.
class _Tool extends StatelessWidget {
  const _Tool({super.key, required this.on, required this.onTap, required this.child, this.tooltip});

  final bool on;
  final VoidCallback onTap;
  final Widget child;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final Color ink = on ? Orbita.brand : Orbita.dim;
    Widget w = Semantics(
      button: true,
      selected: on,
      label: tooltip,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          height: 38,
          alignment: Alignment.center,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            color: on ? webMix(Orbita.brand, .11, Orbita.panel2) : Colors.transparent,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: on ? webMix(Orbita.brand, .45, Orbita.line) : Colors.transparent),
          ),
          child: IconTheme.merge(
            data: IconThemeData(color: ink),
            child: DefaultTextStyle.merge(
              style: TextStyle(fontFamily: Orbita.sans, fontSize: 12, fontWeight: FontWeight.w600, color: ink),
              maxLines: 1,
              child: child,
            ),
          ),
        ),
      ),
    );
    if (tooltip != null) w = Tooltip(message: tooltip!, excludeFromSemantics: true, child: w);
    return w;
  }
}

/// `.fs-n`: 17 px violet pill with the number of sessions waiting for you.
class _Badge extends StatelessWidget {
  const _Badge(this.n);

  final int n;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'Sesiones que te esperan: $n',
      child: Container(
        constraints: const BoxConstraints(minWidth: 17),
        height: 17,
        padding: const EdgeInsets.symmetric(horizontal: 5),
        alignment: Alignment.center,
        decoration: BoxDecoration(color: Orbita.brand, borderRadius: BorderRadius.circular(999)),
        child: Text(
          '$n',
          style: const TextStyle(
              fontFamily: Orbita.sans, fontSize: 10.5, height: 1, fontWeight: FontWeight.w700, color: Orbita.onBrand),
        ),
      ),
    );
  }
}
