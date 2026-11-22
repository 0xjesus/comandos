import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../sessions/feed_row.dart';
import '../sessions/sections.dart';
import 'tabs_controller.dart';

/// `#app-navigation` at phone width (max-width:747px): one row of 42 px keys —
/// panel, «+» … chat/terminal, «Ver todas las sesiones» — with the tab chips
/// (`#tabbar`) on their own scrolling row underneath.
class NavKeys extends StatelessWidget {
  const NavKeys({
    super.key,
    required this.panelOpen,
    required this.chatMode,
    required this.onPanel,
    required this.onNew,
    required this.onChat,
    required this.onOpen,
    this.onCar,
  });

  final bool panelOpen;
  final bool chatMode;
  final VoidCallback onPanel;
  final VoidCallback onNew;
  final VoidCallback onChat;
  final VoidCallback onOpen;

  /// Modo auto (the phone's own key, no web equivalent): hidden when null.
  final VoidCallback? onCar;

  static Widget _key({
    required WebIconData icon,
    required double iconSize,
    required String tooltip,
    required VoidCallback onTap,
    bool brand = false,
    bool on = false,
    Key? key,
  }) =>
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: WebButton(
          key: key,
          size: 42,
          radius: 12,
          lip: 2,
          face: Orbita.panel,
          ink: brand ? Colors.white : Orbita.text,
          brand: brand,
          on: on,
          tooltip: tooltip,
          onTap: onTap,
          child: WebIcon(icon, size: iconSize),
        ),
      );

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 4, 8, 4),
      child: Row(
        children: <Widget>[
          _key(
            key: const ValueKey<String>('tab-panel'),
            icon: WebIcons.panel,
            iconSize: 20,
            tooltip: 'Panel de sesiones',
            onTap: onPanel,
          ),
          const SizedBox(width: 6),
          _key(
            key: const ValueKey<String>('tab-new'),
            icon: WebIcons.plusBold,
            iconSize: 17,
            tooltip: 'Nueva sesión',
            onTap: onNew,
          ),
          const Spacer(),
          if (onCar != null) ...<Widget>[
            _key(
              key: const ValueKey<String>('tab-car'),
              icon: WebIcons.car,
              iconSize: 19,
              tooltip: 'Modo auto',
              onTap: onCar!,
            ),
            const SizedBox(width: 6),
          ],
          _key(
            key: const ValueKey<String>('tab-chat'),
            icon: chatMode ? WebIcons.prompt : WebIcons.bubble,
            iconSize: 18,
            tooltip: chatMode ? 'Ver terminal' : 'Ver como chat',
            brand: chatMode,
            on: chatMode,
            onTap: onChat,
          ),
          const SizedBox(width: 6),
          _key(
            key: const ValueKey<String>('tab-open'),
            icon: WebIcons.tabOpen,
            iconSize: 20,
            tooltip: 'Ver todas las sesiones',
            onTap: onOpen,
          ),
        ],
      ),
    );
  }
}

/// `#tabbar` on a phone: a strip of 44 px pill chips that slides sideways, with a
/// 1 px `--line` rule on top. The open session's chip has the `--brand` border and
/// its «×»; the feed paints the K5 marks (ask = solid violet chip, unread = dot,
/// working = ring). The active chip is centred whenever it changes.
class TabChips extends StatefulWidget {
  const TabChips({
    super.key,
    required this.tabs,
    required this.feed,
    required this.active,
    required this.onTap,
    required this.onClose,
    required this.onLongPress,
  });

  final List<TabEntry> tabs;
  final SessionFeed? feed;
  final String? active;
  final ValueChanged<String> onTap;
  final ValueChanged<TabEntry> onClose;
  final ValueChanged<String> onLongPress;

  @override
  State<TabChips> createState() => _TabChipsState();
}

class _TabChipsState extends State<TabChips> {
  final Map<String, GlobalKey> _keys = <String, GlobalKey>{};
  final ScrollController _scroll = ScrollController();
  String? _centered;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _reveal());
  }

  @override
  void didUpdateWidget(TabChips old) {
    super.didUpdateWidget(old);
    if (widget.active != _centered) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _reveal());
    }
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  /// Centre the active chip (`reveal` in app_coordinator_tabs.rs).
  void _reveal() {
    if (!mounted) return;
    final String? a = widget.active;
    final BuildContext? ctx = a == null ? null : _keys[a]?.currentContext;
    if (ctx == null) return;
    _centered = a;
    Scrollable.ensureVisible(ctx, alignment: .5);
  }

  SessionSummary? _entry(String session) {
    for (final SessionSummary s in widget.feed?.sessions ?? const <SessionSummary>[]) {
      if (s.session == session) return s;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final Set<String> live = <String>{for (final TabEntry t in widget.tabs) t.session};
    _keys.removeWhere((String k, GlobalKey _) => !live.contains(k));
    return Container(
      decoration: const BoxDecoration(border: Border(top: BorderSide(color: Orbita.line))),
      height: 62,
      child: ListView.separated(
        key: const PageStorageKey<String>('tabbar'),
        controller: _scroll,
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 10),
        itemCount: widget.tabs.length,
        separatorBuilder: (BuildContext context, int i) => const SizedBox(width: 8),
        itemBuilder: (BuildContext context, int i) {
          final TabEntry t = widget.tabs[i];
          final SessionSummary? e = _entry(t.session);
          return TabChip(
            key: _keys.putIfAbsent(t.session, GlobalKey.new),
            tab: t,
            mark: e == null ? FeedMark.none : feedMark(e),
            on: t.session == widget.active,
            onTap: () => widget.onTap(t.session),
            onClose: () => widget.onClose(t),
            onLongPress: () => widget.onLongPress(t.session),
          );
        },
      ),
    );
  }
}

/// One `.apptab` chip.
class TabChip extends StatelessWidget {
  const TabChip({
    super.key,
    required this.tab,
    required this.mark,
    required this.on,
    required this.onTap,
    required this.onClose,
    required this.onLongPress,
  });

  final TabEntry tab;
  final FeedMark mark;
  final bool on;
  final VoidCallback onTap;
  final VoidCallback onClose;
  final VoidCallback onLongPress;

  @override
  Widget build(BuildContext context) {
    final bool ask = mark == FeedMark.waiting;
    final Color ink = ask
        ? Orbita.onBrand
        : (on || mark == FeedMark.unread)
            ? Orbita.text
            : Orbita.dim;
    final Color face = ask
        ? Orbita.brand
        : on
            ? webMix(Orbita.panel, .65, Orbita.line2)
            : Colors.transparent;
    final Widget? markWidget = switch (mark) {
      FeedMark.waiting => const _MarkDot(color: Orbita.onBrand),
      FeedMark.unread => const _MarkDot(color: Orbita.brand),
      FeedMark.working => const WorkRing(size: 9),
      FeedMark.none => null,
    };
    return Semantics(
      button: true,
      selected: on,
      label: tab.label,
      child: RawGestureDetector(
        behavior: HitTestBehavior.opaque,
        gestures: <Type, GestureRecognizerFactory>{
          TapGestureRecognizer: GestureRecognizerFactoryWithHandlers<TapGestureRecognizer>(
            TapGestureRecognizer.new,
            (TapGestureRecognizer r) => r.onTap = onTap,
          ),
          // The web's LONG_PRESS_MS.
          LongPressGestureRecognizer: GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
            () => LongPressGestureRecognizer(duration: const Duration(milliseconds: 450)),
            (LongPressGestureRecognizer r) => r.onLongPress = onLongPress,
          ),
        },
        child: Container(
          height: 44,
          padding: EdgeInsets.only(left: 18, right: on && tab.closable ? 10 : 18),
          decoration: BoxDecoration(
            color: face,
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: on ? Orbita.brand : Orbita.line),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              ExcludeSemantics(
                child: Text(
                  tab.label,
                  maxLines: 1,
                  style: TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600, color: ink),
                ),
              ),
              if (markWidget != null) ...<Widget>[
                const SizedBox(width: 6),
                SizedBox.square(dimension: 9, child: Center(child: markWidget)),
              ],
              if (on && tab.closable) ...<Widget>[
                const SizedBox(width: 4),
                Semantics(
                  button: true,
                  label: 'Cerrar pestaña',
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: onClose,
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 10),
                      child: Text(
                        '×',
                        style: TextStyle(
                          fontSize: 16,
                          height: 1,
                          color: ask ? Orbita.onBrand.withValues(alpha: .62) : Orbita.faint,
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _MarkDot extends StatelessWidget {
  const _MarkDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) =>
      Container(width: 7, height: 7, decoration: BoxDecoration(color: color, shape: BoxShape.circle));
}
