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

/// `.ns-label`: mono 10/700, faint, letter-spacing .1em, upper-case.
class NsLabel extends StatelessWidget {
  const NsLabel(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Text(
          text.toUpperCase(),
          style: const TextStyle(
            fontFamily: Orbita.mono,
            fontSize: 10,
            fontWeight: FontWeight.w700,
            letterSpacing: 1,
            color: Orbita.faint,
          ),
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
        children: <Widget>[NsLabel(label), const SizedBox(height: 4), child],
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

/// `.ns-chips button`: mono 11/700 pill, 7×14 padding; `.on` = brand ink on a
/// 12 % brand tint with the green lip; disabled = 42 % opacity, no lip.
class NsChip extends StatelessWidget {
  const NsChip({super.key, required this.item, required this.on, required this.onTap});

  final WizardChip item;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final Color ink = on ? Orbita.brand : Orbita.dim;
    return Semantics(
      selected: on,
      child: _Key(
        onTap: item.disabled ? null : onTap,
        radius: 99,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 7),
        face: on ? webMix(Orbita.brand, .12, Orbita.panel) : Orbita.panel,
        lip: on ? _onLip : Orbita.line2,
        disabledOpacity: .42,
        tooltip: item.title,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            if (item.icon.isNotEmpty) ...<Widget>[
              SizedBox(
                height: 13,
                child: Center(widthFactor: 1, child: ProviderMark(agent: item.icon, model: item.model, color: ink)),
              ),
              const SizedBox(width: 5),
            ],
            Flexible(
              child: Text(
                item.label,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: Orbita.mono,
                  fontSize: 11,
                  fontWeight: FontWeight.w700,
                  height: 1.36,
                  color: ink,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// `.ns-chips`: wrapping row, gap 6.
class NsChips extends StatelessWidget {
  const NsChips({super.key, required this.items, required this.current, required this.onPick});

  final List<WizardChip> items;
  final String current;
  final ValueChanged<String> onPick;

  @override
  Widget build(BuildContext context) => Wrap(
        spacing: 6,
        runSpacing: 6,
        children: <Widget>[
          for (final WizardChip c in items)
            NsChip(key: ValueKey<String>(c.id), item: c, on: c.id == current, onTap: () => onPick(c.id)),
        ],
      );
}

/// `.ns-crumb`: mono 11 faint (the last one: text, 600), 1×3 padding.
class NsCrumb extends StatelessWidget {
  const NsCrumb({super.key, required this.label, required this.last, required this.onTap});

  final String label;
  final bool last;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => _Key(
        onTap: onTap,
        radius: 5,
        padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 1),
        child: Text(
          label,
          style: TextStyle(
            fontFamily: Orbita.mono,
            fontSize: 11,
            fontWeight: last ? FontWeight.w600 : FontWeight.w400,
            color: last ? Orbita.text : Orbita.faint,
          ),
        ),
      );
}

/// `.ns-dir`: a child folder, layers icon + name, 11 px on `--panel`.
class NsDir extends StatelessWidget {
  const NsDir({super.key, required this.name, required this.onTap});

  final String name;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => _Key(
        onTap: onTap,
        radius: 7,
        face: Orbita.panel,
        padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            const WebIcon(WebIcons.layers, size: 11, color: Orbita.text),
            const SizedBox(width: 4),
            Flexible(
              child: Text(name, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 11, color: Orbita.text)),
            ),
          ],
        ),
      );
}

/// `.ns-ok`: «✓ carpeta existente».
class FsOk extends StatelessWidget {
  const FsOk({super.key});

  @override
  Widget build(BuildContext context) => const Align(
        alignment: Alignment.centerLeft,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            WebIcon(WebIcons.check, size: 11, color: Orbita.done),
            SizedBox(width: 4),
            Text('carpeta existente', style: TextStyle(fontSize: 11, color: Orbita.done)),
          ],
        ),
      );
}

/// `button.test` («Crear carpeta»): mono 11 dim on `--panel`, radius 6.
class NsTestButton extends StatelessWidget {
  const NsTestButton({super.key, required this.child, required this.onTap, this.tooltip});

  final Widget child;
  final VoidCallback? onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) => _Key(
        onTap: onTap,
        radius: 6,
        face: Orbita.panel,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
        tooltip: tooltip,
        child: IconTheme.merge(
          data: const IconThemeData(color: Orbita.dim),
          child: DefaultTextStyle.merge(
            style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.dim),
            child: child,
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

/// `.ns-actions button`: mono 13/700, 9×18, radius 10; `.primary` = brand face
/// with #0A0D13 ink. At least 44 px high (coarse pointer).
class NsActionButton extends StatelessWidget {
  const NsActionButton({super.key, required this.label, required this.onTap, this.primary = false, this.tooltip});

  final String label;
  final VoidCallback? onTap;
  final bool primary;
  final String? tooltip;

  @override
  Widget build(BuildContext context) => _Key(
        onTap: onTap,
        radius: 10,
        minHeight: 44,
        face: primary ? Orbita.brand : Orbita.panel,
        border: primary ? Orbita.brand : Orbita.line,
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 9),
        tooltip: tooltip,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          mainAxisAlignment: MainAxisAlignment.center,
          children: <Widget>[
            Text(
              label,
              style: TextStyle(
                fontFamily: Orbita.mono,
                fontSize: 13,
                fontWeight: FontWeight.w700,
                color: primary ? const Color(0xFF0A0D13) : Orbita.text,
              ),
            ),
          ],
        ),
      );
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
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  child: Text(
                    '${open ? '▼' : '▶'} $summary',
                    style: const TextStyle(fontFamily: Orbita.mono, fontSize: 10, color: Orbita.faint, height: 1.4),
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

/// The `<datalist id="ns-dirs">` drop-down: the folders of the live sessions.
class RecentDirsList extends StatelessWidget {
  const RecentDirsList({super.key, required this.dirs, required this.onPick});

  final List<String> dirs;
  final ValueChanged<String> onPick;

  @override
  Widget build(BuildContext context) => Container(
        key: const ValueKey<String>('ns-dirs'),
        constraints: const BoxConstraints(maxHeight: 220),
        decoration: BoxDecoration(
          color: Orbita.panel2,
          border: Border.all(color: Orbita.line2),
          borderRadius: BorderRadius.circular(8),
        ),
        child: ListView.builder(
          padding: const EdgeInsets.symmetric(vertical: 4),
          shrinkWrap: true,
          itemCount: dirs.length,
          itemBuilder: (BuildContext context, int i) => GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => onPick(dirs[i]),
            child: Container(
              constraints: const BoxConstraints(minHeight: 40),
              alignment: Alignment.centerLeft,
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: Text(
                dirs[i],
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.text),
              ),
            ),
          ),
        ),
      );
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
