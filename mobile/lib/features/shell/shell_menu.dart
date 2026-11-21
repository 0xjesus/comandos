import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// One action of the «Más» popover.
@immutable
class MenuAction {
  const MenuAction(this.label, this.onTap, {this.icon, this.detail});

  final String label;
  final VoidCallback onTap;

  /// Rows with an icon are `.mp-row` (36 px key + label); the rest `.mp-tools` buttons.
  final WebIconData? icon;

  /// Faint text after the label (e.g. a count).
  final String? detail;
}

/// `#menu-panel` («Más», opened by `#btn-menu`): a popover 8 px under the ☰ key,
/// `--panel` with a 1 px `--line2` border, radius 12, mono 13. [rows] are the
/// `.mp-row`s (key + label); each group of [groups] is a `.mp-tools` column of
/// 40 px buttons under a 1 px rule. Picking an item closes the popover first.
Future<void> showShellMenu(
  BuildContext context, {
  required GlobalKey anchor,
  required List<MenuAction> rows,
  required List<List<MenuAction>> groups,
}) {
  final RenderBox? box = anchor.currentContext?.findRenderObject() as RenderBox?;
  final Offset origin = box?.localToGlobal(Offset.zero) ?? const Offset(8, 0);
  final double bottom = box == null ? 8 : origin.dy + box.size.height + 8;
  return showGeneralDialog<void>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Cerrar',
    barrierColor: Colors.transparent,
    transitionDuration: Duration.zero,
    pageBuilder: (BuildContext ctx, Animation<double> a, Animation<double> b) {
      final double width = MediaQuery.sizeOf(ctx).width;
      const double menuWidth = 250;
      // `menu_left`: under the key, kept 8 px inside the window.
      final double left = math.max(8, math.min(origin.dx, width - menuWidth - 8));
      void pick(MenuAction m) {
        Navigator.of(ctx).pop();
        m.onTap();
      }

      return Stack(
        children: <Widget>[
          Positioned(
            top: bottom,
            left: left,
            child: Material(
              type: MaterialType.transparency,
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  minWidth: 230,
                  maxWidth: math.min(menuWidth, width - 16),
                  maxHeight: MediaQuery.sizeOf(ctx).height - bottom - 8,
                ),
                child: Container(
                  padding: const EdgeInsets.all(8),
                  decoration: BoxDecoration(
                    color: Orbita.panel,
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: Orbita.line2),
                    boxShadow: const <BoxShadow>[
                      BoxShadow(color: Color(0xBB000000), blurRadius: 40, offset: Offset(0, 18)),
                    ],
                  ),
                  child: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: <Widget>[
                        for (final MenuAction r in rows) ...<Widget>[
                          if (r != rows.first) const SizedBox(height: 6),
                          _MenuRow(action: r, onTap: () => pick(r)),
                        ],
                        for (final List<MenuAction> g in groups) ...<Widget>[
                          const SizedBox(height: 6),
                          Container(
                            padding: const EdgeInsets.only(top: 8),
                            decoration: const BoxDecoration(border: Border(top: BorderSide(color: Orbita.line))),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: <Widget>[
                                for (final MenuAction m in g) ...<Widget>[
                                  if (m != g.first) const SizedBox(height: 6),
                                  _ToolButton(action: m, onTap: () => pick(m)),
                                ],
                              ],
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      );
    },
  );
}

const TextStyle _menuText = TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.text);

/// `.mp-row`: a 36 px key and its label, 40 px high.
class _MenuRow extends StatelessWidget {
  const _MenuRow({required this.action, required this.onTap});

  final MenuAction action;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 44),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          child: Row(
            children: <Widget>[
              WebButton(
                size: 36,
                tooltip: action.label,
                onTap: onTap,
                child: WebIcon(action.icon ?? WebIcons.menu, size: 16),
              ),
              const SizedBox(width: 10),
              Expanded(child: Text(action.label, style: _menuText)),
            ],
          ),
        ),
      ),
    );
  }
}

/// `.mp-tools button`: 40 px (44 on touch), 1 px `--line`, radius 8, `--panel2`, left text.
class _ToolButton extends StatelessWidget {
  const _ToolButton({required this.action, required this.onTap});

  final MenuAction action;
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
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          alignment: Alignment.centerLeft,
          decoration: BoxDecoration(
            color: Orbita.panel2,
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: Orbita.line),
          ),
          child: Text.rich(
            TextSpan(children: <InlineSpan>[
              TextSpan(text: action.label),
              if (action.detail != null)
                TextSpan(text: '  ${action.detail}', style: const TextStyle(color: Orbita.faint)),
            ]),
            style: _menuText,
          ),
        ),
      ),
    );
  }
}
