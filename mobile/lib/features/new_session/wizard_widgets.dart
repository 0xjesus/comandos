import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'provider_mark.dart';
import 'wizard_model.dart';

// The controls of the «Nueva sesión» card, measured from index_page.css
// (`.ns-*` rules) and dash/buttons.css («sutil»: every <button> without
// data-flat gets a 1 px --line border, a flat 2 px --line2 lip and sits 1 px
// up; pressed it sinks and loses the lip; `.on` turns the lip green).
// `background:none` buttons sit on the card, so their face is `--panel`
// (opaque, or the lip would show through).

/// `.ns-note`: mono 10, faint.
const TextStyle nsNoteStyle = TextStyle(fontFamily: Orbita.mono, fontSize: 10, color: Orbita.faint, height: 1.4);

/// «sutil» lip for an `.on` key: `color-mix(--done 55%, --line)`.
final Color _onLip = webMix(Orbita.done, .55, Orbita.line);

/// `.ns-label`: mono 10.5/700, dim, letter-spacing .1em, upper-case.
class NsLabel extends StatelessWidget {
  const NsLabel(this.text, {super.key, this.trailing});

  final String text;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.only(bottom: 8),
        child: Row(
          children: <Widget>[
            Text(
              text.toUpperCase(),
              style: const TextStyle(
                fontFamily: Orbita.mono,
                fontSize: 10.5,
                fontWeight: FontWeight.w700,
                letterSpacing: 1.2,
                color: Orbita.dim,
              ),
            ),
            if (trailing != null) ...<Widget>[const SizedBox(width: 8), Expanded(child: trailing!)],
          ],
        ),
      );
}

/// `#ns-motor-wrap` & co.: an inline label right above its chips.
class NsWrap extends StatelessWidget {
  const NsWrap({super.key, required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[NsLabel(label), child],
      );
}

/// A «sutil» key: the raise/press behaviour shared by the wizard's buttons.
class _Key extends StatefulWidget {
  const _Key({
    required this.child,
    required this.onTap,
    required this.radius,
    required this.padding,
    this.face = Orbita.panel,
    this.border = Orbita.line,
    this.lip = Orbita.line2,
    this.minHeight = 0,
    this.disabledOpacity = .45,
    this.tooltip,
  });

  final Widget child;
  final VoidCallback? onTap;
  final double radius;
  final EdgeInsets padding;
  final Color face;
  final Color border;
  final Color lip;
  final double minHeight;
  final double disabledOpacity;
  final String? tooltip;

  @override
  State<_Key> createState() => _KeyState();
}

class _KeyState extends State<_Key> {
  bool _down = false;

  void _set(bool v) {
    if (_down != v && mounted) setState(() => _down = v);
  }

  @override
  Widget build(BuildContext context) {
    final bool enabled = widget.onTap != null;
    final double dy = !enabled ? 0 : (_down ? 1 : -1);
    Widget box = Transform.translate(
      offset: Offset(0, dy),
      child: Container(
        constraints: BoxConstraints(minHeight: widget.minHeight),
        padding: widget.padding,
        decoration: BoxDecoration(
          color: widget.face,
          borderRadius: BorderRadius.circular(widget.radius),
          border: Border.all(color: widget.border),
          boxShadow: _down || !enabled ? null : <BoxShadow>[BoxShadow(color: widget.lip, offset: const Offset(0, 2))],
        ),
        child: widget.child,
      ),
    );
    if (!enabled) box = Opacity(opacity: widget.disabledOpacity, child: box);
    Widget out = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: enabled ? (_) => _set(true) : null,
      onTapUp: enabled ? (_) => _set(false) : null,
      onTapCancel: enabled ? () => _set(false) : null,
      onTap: widget.onTap,
      child: box,
    );
    if (widget.tooltip != null && widget.tooltip!.isNotEmpty) {
      out = Tooltip(message: widget.tooltip!, excludeFromSemantics: true, child: out);
    }
    return Semantics(button: true, enabled: enabled, child: out);
  }
}

/// `.ns-chips button`: mono 12/700 pill, at least 44 px high (coarse pointer);
/// `.on` = brand ink on a 12 % brand tint with the green lip; disabled = 42 %
/// opacity, no lip. A short [WizardChip.tag] («nuevo») shows as a badge.
class NsChip extends StatelessWidget {
  const NsChip({super.key, required this.item, required this.on, required this.onTap});

  final WizardChip item;
  final bool on;
  final VoidCallback onTap;

  /// Longer tags («el más reciente · código y día a día») only go in the
  /// tooltip and the model list.
  static bool shortTag(String tag) => tag.isNotEmpty && tag.length <= 12;

  @override
  Widget build(BuildContext context) {
    final Color ink = on ? Orbita.brand : Orbita.dim;
    return Semantics(
      selected: on,
      child: _Key(
        onTap: item.disabled ? null : onTap,
        radius: 99,
        minHeight: 44,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
        face: on ? webMix(Orbita.brand, .12, Orbita.panel) : Orbita.panel,
        border: on ? webMix(Orbita.brand, .55, Orbita.line) : Orbita.line,
        lip: on ? _onLip : Orbita.line2,
        disabledOpacity: .42,
        tooltip: item.title,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            if (item.icon.isNotEmpty) ...<Widget>[
              SizedBox(
                height: 30,
                child: Center(
                  widthFactor: 1,
                  child: ProviderMark(agent: item.icon, model: item.model, size: 15, color: ink, chip: true),
                ),
              ),
              const SizedBox(width: 7),
            ],
            Flexible(
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 6),
                child: Text(
                  item.label,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: Orbita.mono,
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                    height: 1.36,
                    color: on ? Orbita.text : Orbita.dim,
                  ),
                ),
              ),
            ),
            if (shortTag(item.tag)) ...<Widget>[
              const SizedBox(width: 6),
              _Tag(item.tag),
            ],
          ],
        ),
      ),
    );
  }
}

class _Tag extends StatelessWidget {
  const _Tag(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
        decoration: BoxDecoration(
          color: webMix(Orbita.done, .14, Orbita.panel),
          borderRadius: BorderRadius.circular(99),
          border: Border.all(color: webMix(Orbita.done, .35, Orbita.line)),
        ),
        child: Text(
          text,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(fontFamily: Orbita.mono, fontSize: 10, fontWeight: FontWeight.w600, color: Orbita.done),
        ),
      );
}

/// `.ns-chips`: wrapping row, gap 8; a chip never grows wider than the row.
class NsChips extends StatelessWidget {
  const NsChips({super.key, required this.items, required this.current, required this.onPick});

  final List<WizardChip> items;
  final String current;
  final ValueChanged<String> onPick;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (BuildContext context, BoxConstraints box) => Wrap(
          spacing: 8,
          runSpacing: 8,
          children: <Widget>[
            for (final WizardChip c in items)
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: box.maxWidth.isFinite ? box.maxWidth : double.infinity),
                child: NsChip(key: ValueKey<String>(c.id), item: c, on: c.id == current, onTap: () => onPick(c.id)),
              ),
          ],
        ),
      );
}

/// One model of «Buscar modelo»: name, id dimmed, its tag, a check when chosen.
class NsModelRow extends StatelessWidget {
  const NsModelRow({super.key, required this.item, required this.on, required this.onTap});

  final WizardChip item;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
        selected: on,
        button: true,
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(
            onTap: onTap,
            borderRadius: BorderRadius.circular(10),
            child: Container(
              constraints: const BoxConstraints(minHeight: 50),
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              decoration: on
                  ? BoxDecoration(
                      color: webMix(Orbita.brand, .10, Orbita.panel),
                      borderRadius: BorderRadius.circular(10),
                    )
                  : null,
              child: Row(
                children: <Widget>[
                  SizedBox(
                    width: 20,
                    child: Center(
                      child: ProviderMark(agent: item.icon, model: item.model, size: 15, color: Orbita.dim, chip: true),
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: <Widget>[
                        Text(
                          item.label,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: Orbita.sans,
                            fontSize: 14,
                            fontWeight: FontWeight.w600,
                            color: on ? Orbita.brand : Orbita.text,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text.rich(
                          TextSpan(
                            children: <InlineSpan>[
                              TextSpan(text: item.id),
                              if (item.tag.isNotEmpty)
                                TextSpan(text: '  ·  ${item.tag}', style: const TextStyle(color: Orbita.done)),
                            ],
                          ),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint),
                        ),
                      ],
                    ),
                  ),
                  if (on) ...<Widget>[
                    const SizedBox(width: 8),
                    const WebIcon(WebIcons.check, size: 16, color: Orbita.brand),
                  ],
                ],
              ),
            ),
          ),
        ),
      );
}

/// `.sw`: 34×20 track (`--line2`, on: `--brand`), 16 px knob.
class NsSwitch extends StatelessWidget {
  const NsSwitch({super.key, required this.on});

  final bool on;

  @override
  Widget build(BuildContext context) => Transform.translate(
        offset: const Offset(0, -1),
        child: Container(
          width: 36,
          height: 22,
          decoration: BoxDecoration(
            color: on ? Orbita.brand : Orbita.line2,
            borderRadius: BorderRadius.circular(99),
            border: Border.all(color: Orbita.line),
            boxShadow: const <BoxShadow>[BoxShadow(color: Orbita.line2, offset: Offset(0, 2))],
          ),
          child: Stack(
            children: <Widget>[
              AnimatedPositioned(
                duration: const Duration(milliseconds: 180),
                left: on ? 16 : 2,
                top: 2,
                child: Container(
                  width: 16,
                  height: 16,
                  decoration: BoxDecoration(shape: BoxShape.circle, color: on ? Colors.white : Orbita.dim),
                ),
              ),
            ],
          ),
        ),
      );
}

/// `.ns-actions button`: 14/700, radius 12, 48 px high; `.primary` = brand
/// face with dark ink. The label is one line, cut with an ellipsis.
class NsActionButton extends StatelessWidget {
  const NsActionButton({
    super.key,
    required this.label,
    required this.onTap,
    this.primary = false,
    this.tooltip,
    this.icon,
  });

  final String label;
  final VoidCallback? onTap;
  final bool primary;
  final String? tooltip;
  final WebIconData? icon;

  @override
  Widget build(BuildContext context) {
    final Color ink = primary ? Orbita.onBrand : Orbita.text;
    return _Key(
      onTap: onTap,
      radius: 12,
      minHeight: 48,
      face: primary ? Orbita.brand : Orbita.panel2,
      border: primary ? Orbita.brand : Orbita.line2,
      lip: primary ? webMix(Orbita.brand, .55, Orbita.bg) : Orbita.line2,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      tooltip: tooltip,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        mainAxisAlignment: MainAxisAlignment.center,
        children: <Widget>[
          if (icon != null) ...<Widget>[WebIcon(icon!, size: 16, color: ink), const SizedBox(width: 8)],
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w700, color: ink),
            ),
          ),
        ],
      ),
    );
  }
}

/// `<details id="ns-unavailable">`: «▶ 12 combinaciones listas · 4 bloqueadas
/// (login / gateway)», opening the dashed list of blocked routes.
class BlockedRoutes extends StatelessWidget {
  const BlockedRoutes({
    super.key,
    required this.summary,
    required this.rows,
    required this.open,
    required this.onToggle,
  });

  final String summary;
  final List<BlockedRoute> rows;
  final bool open;
  final VoidCallback onToggle;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.only(top: 5),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            Semantics(
              button: true,
              expanded: open,
              child: GestureDetector(
                key: const ValueKey<String>('ns-unavailable'),
                behavior: HitTestBehavior.opaque,
                onTap: onToggle,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(minHeight: 44),
                  child: Row(
                    children: <Widget>[
                      WebIcon(open ? WebIcons.chevronDown : WebIcons.chevronRight, size: 12, color: Orbita.faint),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          summary,
                          style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint, height: 1.4),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            if (open) ...<Widget>[
              const SizedBox(height: 6),
              for (int i = 0; i < rows.length; i++) ...<Widget>[
                if (i > 0) const SizedBox(height: 5),
                CustomPaint(
                  painter: const _DashedBox(color: Orbita.line2, radius: 8),
                  child: Container(
                    decoration: BoxDecoration(color: Orbita.bg, borderRadius: BorderRadius.circular(8)),
                    padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 7),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: <Widget>[
                        Text(
                          rows[i].title,
                          style: const TextStyle(
                            fontFamily: Orbita.mono,
                            fontSize: 10,
                            fontWeight: FontWeight.w700,
                            color: Orbita.text,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          rows[i].reason,
                          style: const TextStyle(fontFamily: Orbita.mono, fontSize: 10, height: 1.35, color: Orbita.dim),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ],
          ],
        ),
      );
}

class _DashedBox extends CustomPainter {
  const _DashedBox({required this.color, required this.radius});

  final Color color;
  final double radius;

  @override
  void paint(Canvas canvas, Size size) {
    final Path border = Path()
      ..addRRect(RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(radius)).deflate(.5));
    final Paint p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    for (final ui.PathMetric metric in border.computeMetrics()) {
      double d = 0;
      while (d < metric.length) {
        canvas.drawPath(metric.extractPath(d, d + 3), p);
        d += 6;
      }
    }
  }

  @override
  bool shouldRepaint(_DashedBox old) => old.color != color || old.radius != radius;
}

/// `.toast` (`.err`: amber edge): mono 13 on `--panel2`, 3 px left edge.
class WizardToast extends StatelessWidget {
  const WizardToast({super.key, required this.text, this.error = false});

  final String text;
  final bool error;

  @override
  Widget build(BuildContext context) {
    final double max = MediaQuery.sizeOf(context).width * .9;
    return ConstrainedBox(
      constraints: BoxConstraints(maxWidth: max > 380 ? 380 : max),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(9),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: Orbita.panel2,
            borderRadius: BorderRadius.circular(9),
            border: Border.all(color: Orbita.line),
          ),
          child: IntrinsicHeight(
            child: Row(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                ColoredBox(color: error ? Orbita.waiting : Orbita.brand, child: const SizedBox(width: 3)),
                Flexible(
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(12, 10, 14, 10),
                    child: Text(
                      text,
                      style: const TextStyle(
                        fontFamily: Orbita.mono,
                        fontSize: 13,
                        color: Orbita.text,
                        decoration: TextDecoration.none,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
