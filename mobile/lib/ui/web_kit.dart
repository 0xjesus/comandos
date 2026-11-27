// Shared controls that copy the ComandOS web page (phone width).
//
// Sources: dash/buttons.css («sutil», the default 3D style: 1 px border, a flat
// lip under the key, pressed = sinks 2 px), dash/workspace.css (`header .hdr-btn`,
// `.fs-chip`), crates/comandos-web-view/src/index_page.css (`.apptab` at
// max-width:747px, `.sec-label`) and the line icons of
// crates/comandos-web/src/components/foundation_icons.json, drawn here with
// CustomPaint from the same SVG data (no SVG package, no icon font).
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'theme.dart';

/// `color-mix(in srgb, a p%, b)`.
Color webMix(Color a, double p, Color b) => Color.lerp(b, a, p)!;

// ---------------------------------------------------------------------------
// WebButton
// ---------------------------------------------------------------------------

/// The raised key every ComandOS button uses («sutil» style).
///
/// - default: `--panel2` face, 1 px `--line` border, 3 px `--line` lip, `--dim` ink
///   (`header .hdr-btn`, radius 10).
/// - [on]: ink `--done`, border and lip tinted green (`.hdr-btn.on`, `aria-pressed`).
/// - [brand]: face and border `--brand`, white ink (`#tab-chat` in chat mode).
/// - pressed: sinks 2 px and loses the lip, like `:active`.
///
/// [size] makes it a square key (icon buttons); otherwise it grows with [child]
/// and [padding], never below 44 px high (coarse pointer).
class WebButton extends StatefulWidget {
  const WebButton({
    super.key,
    required this.child,
    this.onTap,
    this.on = false,
    this.brand = false,
    this.padding,
    this.size,
    this.tooltip,
    this.radius = 10,
    this.face,
    this.ink,
    this.lip = 3,
    this.minHeight = 44,
    this.onLongPress,
  });

  final Widget child;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final bool on;
  final bool brand;
  final EdgeInsets? padding;
  final double? size;
  final String? tooltip;
  final double radius;

  /// Face colour override (default `--panel2`).
  final Color? face;

  /// Ink colour override (default `--dim`, `--done` when [on], white when [brand]).
  final Color? ink;

  /// Height of the lip under the key (3 in the header, 2 for plain buttons).
  final double lip;
  final double minHeight;

  @override
  State<WebButton> createState() => _WebButtonState();
}

class _WebButtonState extends State<WebButton> {
  bool _down = false;

  void _set(bool v) {
    if (_down != v && mounted) setState(() => _down = v);
  }

  @override
  Widget build(BuildContext context) {
    final bool enabled = widget.onTap != null || widget.onLongPress != null;
    final Color face = widget.brand ? Orbita.brand : (widget.face ?? Orbita.panel2);
    final Color border = widget.brand
        ? Orbita.brand
        : widget.on
            ? webMix(Orbita.done, .45, Orbita.line)
            : Orbita.line;
    final Color lip = widget.on
        ? webMix(Orbita.done, .40, Orbita.line)
        : (widget.brand ? webMix(Orbita.brand, .55, Orbita.bg) : Orbita.line);
    final Color ink = widget.ink ??
        (widget.brand
            ? Colors.white
            : widget.on
                ? Orbita.done
                : Orbita.dim);
    final double dy = _down ? 2 : -1;
    Widget key = AnimatedContainer(
      duration: const Duration(milliseconds: 60),
      curve: Curves.linear,
      transform: Matrix4.translationValues(0, dy, 0),
      width: widget.size,
      height: widget.size,
      constraints: widget.size == null ? BoxConstraints(minHeight: widget.minHeight, minWidth: widget.minHeight) : null,
      padding: widget.size == null ? (widget.padding ?? const EdgeInsets.symmetric(horizontal: 12)) : widget.padding,
      decoration: BoxDecoration(
        color: face,
        borderRadius: BorderRadius.circular(widget.radius),
        border: Border.all(color: border),
        boxShadow: _down || !enabled
            ? const <BoxShadow>[]
            : <BoxShadow>[
                BoxShadow(color: lip, offset: Offset(0, widget.lip)),
                if (widget.lip >= 3)
                  const BoxShadow(color: Color(0x2E000000), offset: Offset(0, 4), blurRadius: 8),
              ],
      ),
      // Shrinks to its content under loose constraints (a row of keys), fills
      // under tight ones (a stretched button).
      child: Center(
        widthFactor: 1,
        heightFactor: 1,
        child: IconTheme.merge(
          data: IconThemeData(color: ink, size: 17),
          child: DefaultTextStyle.merge(
            style: TextStyle(color: ink, fontSize: 13, fontWeight: FontWeight.w600),
            child: widget.child,
          ),
        ),
      ),
    );
    key = Opacity(opacity: enabled ? 1 : .55, child: key);
    Widget out = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: enabled ? (_) => _set(true) : null,
      onTapUp: enabled ? (_) => _set(false) : null,
      onTapCancel: enabled ? () => _set(false) : null,
      onTap: widget.onTap,
      onLongPress: widget.onLongPress,
      child: key,
    );
    out = Semantics(button: true, enabled: enabled, selected: widget.on, label: widget.tooltip, child: out);
    if (widget.tooltip != null) {
      out = Tooltip(message: widget.tooltip!, excludeFromSemantics: true, child: out);
    }
    return out;
  }
}

// ---------------------------------------------------------------------------
// WebChip
// ---------------------------------------------------------------------------

/// Visual flavours of [WebChip].
enum WebChipStyle {
  /// `#tabbar .apptab` on a phone: 44 px pill, 1 px `--line`, 0 18 px, 14 px text;
  /// on = `--brand` border.
  tab,

  /// `.fs-chip` (feed filters, coarse pointer): ≥32 px pill, `--panel` face,
  /// transparent border, 11.5 px/500; on = `--line2` border, lighter face, `--text`.
  filter,

  /// Wizard option chips: like [tab] but 40 px and 13 px text.
  option,
}

/// Pill chip (.apptab / wizard options / filters).
class WebChip extends StatelessWidget {
  const WebChip({
    super.key,
    required this.child,
    this.on = false,
    this.onTap,
    this.leading,
    this.style = WebChipStyle.option,
    this.onLongPress,
    this.trailing,
  });

  final Widget child;
  final bool on;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final Widget? leading;
  final Widget? trailing;
  final WebChipStyle style;

  @override
  Widget build(BuildContext context) {
    final (double h, EdgeInsets pad, TextStyle ts, Color face, Color border) = switch (style) {
      WebChipStyle.tab => (
          44,
          const EdgeInsets.symmetric(horizontal: 18),
          TextStyle(
            fontFamily: Orbita.sans,
            fontSize: 14,
            fontWeight: FontWeight.w600,
            color: on ? Orbita.text : Orbita.dim,
          ),
          on ? webMix(Orbita.panel, .65, Orbita.line2) : Colors.transparent,
          on ? Orbita.brand : Orbita.line,
        ),
      WebChipStyle.filter => (
          32,
          const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
          TextStyle(
            fontFamily: Orbita.sans,
            fontSize: 11.5,
            height: 1.35,
            fontWeight: FontWeight.w500,
            color: on ? Orbita.text : Orbita.dim,
          ),
          on ? webMix(Orbita.text, .10, Orbita.panel2) : Orbita.panel,
          on ? Orbita.line2 : Colors.transparent,
        ),
      WebChipStyle.option => (
          40,
          const EdgeInsets.symmetric(horizontal: 14),
          TextStyle(
            fontFamily: Orbita.sans,
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: on ? Orbita.text : Orbita.dim,
          ),
          on ? webMix(Orbita.brand, .16, Orbita.panel) : Orbita.panel,
          on ? Orbita.brand : Orbita.line,
        ),
    };
    final List<Widget> parts = <Widget>[
      if (leading != null) ...<Widget>[leading!, const SizedBox(width: 8)],
      Flexible(child: DefaultTextStyle.merge(style: ts, maxLines: 1, overflow: TextOverflow.ellipsis, child: child)),
      if (trailing != null) ...<Widget>[const SizedBox(width: 8), trailing!],
    ];
    return Semantics(
      button: true,
      selected: on,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        onLongPress: onLongPress,
        child: Container(
          constraints: BoxConstraints(minHeight: h),
          padding: pad,
          decoration: BoxDecoration(
            color: face,
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: border),
          ),
          child: IconTheme.merge(
            data: IconThemeData(color: ts.color, size: 14),
            child: Row(mainAxisSize: MainAxisSize.min, children: parts),
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// WebSectionLabel
// ---------------------------------------------------------------------------

/// `.sec-label`: mono 10 px, 700, uppercase, letter-spacing .14em, `--dim`, with
/// the 1 px `--line` rule that fills the rest of the row (`::after`).
class WebSectionLabel extends StatelessWidget {
  const WebSectionLabel(this.text, {super.key, this.rule = true, this.color});

  final String text;
  final bool rule;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final Widget label = Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: Orbita.mono,
        fontSize: 10,
        fontWeight: FontWeight.w700,
        letterSpacing: 1.4,
        color: color ?? Orbita.dim,
      ),
    );
    if (!rule) return label;
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Row(
        children: <Widget>[
          label,
          const SizedBox(width: 8),
          const Expanded(child: SizedBox(height: 1, child: ColoredBox(color: Orbita.line))),
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------

OverlayEntry? _toast;
Timer? _toastTimer;

/// `.toast` of `#toasts`: top right under the nav (safe area + 50 px), `--panel2`,
/// 1 px `--line`, 3 px left accent (`--brand`, `--waiting` when [error]), mono 13.
/// One at a time; gone after 3.5 s.
void showWebToast(BuildContext context, String text, {bool error = false}) {
  final OverlayState? overlay = Overlay.maybeOf(context, rootOverlay: true);
  if (overlay == null) return;
  _toastTimer?.cancel();
  _toast?.remove();
  final double top = MediaQuery.paddingOf(context).top + 50;
  final OverlayEntry entry = OverlayEntry(
    builder: (BuildContext context) => Positioned(
      top: top,
      right: 12,
      child: IgnorePointer(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: math.min(380, MediaQuery.sizeOf(context).width * .9)),
          // A radius needs one border colour: the 3 px accent is a bar inside the clip.
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
                    Container(width: 3, color: error ? Orbita.waiting : Orbita.brand),
                    Flexible(
                      child: Padding(
                        padding: const EdgeInsets.fromLTRB(13, 10, 14, 10),
                        child: Text(
                          text,
                          style: const TextStyle(
                            fontFamily: Orbita.mono,
                            fontSize: 13,
                            color: Orbita.text,
                            decoration: TextDecoration.none,
                            fontWeight: FontWeight.w400,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
  _toast = entry;
  overlay.insert(entry);
  _toastTimer = Timer(const Duration(milliseconds: 3500), () {
    if (identical(_toast, entry)) {
      entry.remove();
      _toast = null;
    }
  });
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

/// One line icon: SVG primitives in a square view box.
///
/// Primitive strings: `p:<path d>`, `c:cx cy r`, `r:x y w h [rx]`, `l:x1 y1 x2 y2`,
/// `pl:<points>` (polyline), `pg:<points>` (polygon), `e:cx cy rx ry`.
/// A leading `~` on any primitive paints it at 35 % opacity.
@immutable
class WebIconData {
  const WebIconData(this.parts, {this.stroke = 2, this.fill = false, this.box = 24});

  final List<String> parts;
  final double stroke;
  final bool fill;
  final double box;
}

/// The icons of `foundation_icons.json` plus the inline ones of the page
/// (`#tab-panel`, `#tab-open`, the chat-bubble / terminal of `#tab-chat`).
abstract final class WebIcons {
  static const WebIconData chevronLeft = WebIconData(<String>['p:m15 18-6-6 6-6'], stroke: 2.4);
  static const WebIconData chevronRight = WebIconData(<String>['p:m9 18 6-6-6-6'], stroke: 2.4);
  static const WebIconData chevronDown = WebIconData(<String>['p:m6 9 6 6 6-6'], stroke: 2.4);
  static const WebIconData chevronUp = WebIconData(<String>['p:m18 15-6-6-6 6'], stroke: 2.4);
  static const WebIconData chevron = WebIconData(<String>['p:M6 9l6 6 6-6'], stroke: 2.2);
  static const WebIconData bell =
      WebIconData(<String>['p:M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9', 'p:M10.3 21a1.94 1.94 0 0 0 3.4 0']);
  static const WebIconData volumeOn = WebIconData(<String>[
    'pg:11 5 6 9 2 9 2 15 6 15 11 19 11 5',
    'p:M15.5 8.5a5 5 0 0 1 0 7',
    'p:M19 5a10 10 0 0 1 0 14',
  ]);
  static const WebIconData volumeOff = WebIconData(<String>[
    'pg:11 5 6 9 2 9 2 15 6 15 11 19 11 5',
    'l:22 9 16 15',
    'l:16 9 22 15',
  ]);
  static const WebIconData moon = WebIconData(<String>['p:M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z']);
  static const WebIconData sun = WebIconData(<String>[
    'c:12 12 4',
    'p:M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4',
  ]);
  static const WebIconData flame = WebIconData(<String>[
    'p:M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.4-.8-2-1.5-3-1.6-2.5.3-6 1.5-6.5 0 3 2 4 3 5.5s2.5 3 2.5 5.5a7 7 0 1 1-14 0c0-2.6 1.5-4.5 3-5.5',
  ]);
  static const WebIconData zap = WebIconData(<String>['p:m13 2-9 12h7l-1 8 9-12h-7z']);
  static const WebIconData bars = WebIconData(<String>['l:4 20 4 10', 'l:12 20 12 4', 'l:20 20 20 14']);
  static const WebIconData ring = WebIconData(<String>['~c:12 12 8', 'p:M12 4a8 8 0 0 1 8 8']);
  static const WebIconData rows = WebIconData(<String>['r:3 4 18 6 1.5', 'r:3 14 18 6 1.5']);
  static const WebIconData search = WebIconData(<String>['c:11 11 7', 'p:m20 20-3.5-3.5']);
  static const WebIconData copy =
      WebIconData(<String>['r:9 9 12 12 2', 'p:M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1']);
  static const WebIconData layers =
      WebIconData(<String>['p:M12 2 2 7l10 5 10-5-10-5z', 'p:m2 17 10 5 10-5', 'p:m2 12 10 5 10-5']);

  /// `#tab-open` (stroke 1.8, its own path).
  static const WebIconData tabOpen =
      WebIconData(<String>['p:m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5M3 16l9 5 9-5'], stroke: 1.8);

  /// `#tab-panel`: a framed sidebar (stroke 1.8).
  static const WebIconData panel = WebIconData(<String>['r:3 4 18 16 2', 'p:M9 4v16'], stroke: 1.8);
  static const WebIconData plus = WebIconData(<String>['p:M12 5v14M5 12h14'], stroke: 1.9);

  /// `#tab-new` uses stroke 2.
  static const WebIconData plusBold = WebIconData(<String>['p:M12 5v14M5 12h14']);
  static const WebIconData help =
      WebIconData(<String>['c:12 12 9', 'p:M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3', 'p:M12 17h.01']);
  static const WebIconData close = WebIconData(<String>['p:M18 6 6 18M6 6l12 12'], stroke: 1.8);
  static const WebIconData x = WebIconData(<String>['p:M18 6 6 18M6 6l12 12']);
  static const WebIconData refresh = WebIconData(<String>[
    'p:M3 12a9 9 0 0 1 15.7-6.1L21 8',
    'p:M21 3v5h-5',
    'p:M21 12a9 9 0 0 1-15.7 6.1L3 16',
    'p:M3 21v-5h5',
  ]);
  static const WebIconData pencil =
      WebIconData(<String>['p:M17 3a2.85 2.85 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z', 'p:m15 5 4 4']);
  static const WebIconData terminal = WebIconData(<String>['p:m4 7 5 5-5 5', 'p:M12 19h8']);

  /// `#tab-chat` while the chat is shown (">_", stroke 2).
  static const WebIconData prompt = WebIconData(<String>['p:m5 8 4 4-4 4M12 17h7']);

  /// `#tab-chat` while the terminal is shown (chat bubble, stroke 1.9).
  static const WebIconData bubble =
      WebIconData(<String>['p:M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z'], stroke: 1.9);
  static const WebIconData snippet = WebIconData(<String>['pl:16 18 22 12 16 6', 'pl:8 6 2 12 8 18']);
  static const WebIconData anthropic = WebIconData(
      <String>['p:M10.3 3h3.4l7.3 18h-3.5l-1.5-3.9H8L6.5 21H3l7.3-18zm1.7 4.2-2.9 7.4h5.8L12 7.2z'],
      fill: true);
  static const WebIconData gemini = WebIconData(
      <String>['p:M12 2c.6 5.4 4.6 9.4 10 10-5.4.6-9.4 4.6-10 10-.6-5.4-4.6-9.4-10-10 5.4-.6 9.4-4.6 10-10z'],
      fill: true);
  static const WebIconData pause = WebIconData(<String>['r:6 4 4 16 1', 'r:14 4 4 16 1'], fill: true);
  static const WebIconData play =
      WebIconData(<String>['p:M7 4.5v15a1 1 0 0 0 1.5.86l12-7.5a1 1 0 0 0 0-1.72l-12-7.5A1 1 0 0 0 7 4.5z'], fill: true);
  static const WebIconData plug = WebIconData(<String>['p:M9 2v6M15 2v6M6 8h12v3a6 6 0 0 1-12 0V8zM12 17v5']);
  static const WebIconData sparkles = WebIconData(
      <String>['p:M11 3l1.8 5.2L18 10l-5.2 1.8L11 17l-1.8-5.2L4 10l5.2-1.8L11 3zM19 14l.9 2.6 2.6.9-2.6.9L19 21l-.9-2.6-2.6-.9 2.6-.9L19 14z'],
      fill: true);
  static const WebIconData server =
      WebIconData(<String>['r:3 4 18 7 2', 'r:3 13 18 7 2', 'p:M7 7.5h.01M7 16.5h.01']);
  static const WebIconData lock = WebIconData(<String>['r:4 11 16 10 2', 'p:M8 11V7a4 4 0 0 1 8 0v4']);
  static const WebIconData database = WebIconData(
      <String>['e:12 5 8 3', 'p:M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3']);
  static const WebIconData check = WebIconData(<String>['p:m5 12 4 4L19 6'], stroke: 2.2);
  static const WebIconData timer = WebIconData(<String>['c:12 13 8', 'p:M12 9v4l2.5 2M9 2h6M12 5V2']);
  static const WebIconData menu = WebIconData(<String>['p:M4 6h16M4 12h16M4 18h16']);
  static const WebIconData smartphone = WebIconData(<String>['r:5 2 14 20 2', 'p:M12 18h.01']);
  static const WebIconData settings = WebIconData(<String>[
    'p:M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.2a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4a2 2 0 0 0 .7 2.7l.2.1a2 2 0 0 1 1 1.7v.5a2 2 0 0 1-1 1.7l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.2a2 2 0 0 1 1 1.7V20a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.2a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.7v-.5a2 2 0 0 1 1-1.7l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.2a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2z',
    'c:12 12 3',
  ]);
  static const WebIconData cycle = WebIconData(<String>[
    'p:M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8',
    'p:M21 3v5h-5',
    'p:M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16',
    'p:M8 16H3v5',
  ]);
  static const WebIconData key = WebIconData(<String>[
    'p:m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4',
    'p:m21 2-9.6 9.6',
    'c:7.5 15.5 5.5',
  ]);
  static const WebIconData arrowUpDown = WebIconData(<String>['p:M7 4v16M3 8l4-4 4 4M17 20V4M13 16l4 4 4-4']);
  static const WebIconData send = WebIconData(<String>['p:M5 12h14M13 6l6 6-6 6'], stroke: 2.2);
  static const WebIconData mic = WebIconData(<String>[
    'r:9 3 6 11 3',
    'p:M5 11a7 7 0 0 0 14 0',
    'p:M12 18v3',
  ]);
  static const WebIconData car = WebIconData(<String>[
    'p:M5 17h14M6 17v2M18 17v2M4 13l2-5a2 2 0 0 1 1.9-1.3h8.2A2 2 0 0 1 18 8l2 5v4H4v-4z',
    'p:M7.5 13.5h.01M16.5 13.5h.01',
  ]);
  /// «Ponerse al día»: a stack of cards.
  static const WebIconData deck = WebIconData(<String>['r:4 8 13 13 2', 'p:M8 4.5h10a2 2 0 0 1 2 2V16']);
  static const WebIconData openai = WebIconData(<String>[
    'p:M12 3.2l7.6 4.4v8.8L12 20.8l-7.6-4.4V7.6L12 3.2z',
    'p:M12 7.4l3.9 2.3v4.6L12 16.6l-3.9-2.3V9.7L12 7.4z',
    'p:M12 3.2v4.2M19.6 7.6l-3.7 2.1M19.6 16.4l-3.7-2.1M12 20.8v-4.2M4.4 16.4l3.7-2.1M4.4 7.6l3.7 2.1',
  ], stroke: 1.7);

  /// Lucide `folder`.
  static const WebIconData folder = WebIconData(<String>[
    'p:M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z',
  ], stroke: 1.9);

  /// Lucide `git-branch` (a folder that is a git repo).
  static const WebIconData gitBranch =
      WebIconData(<String>['l:6 3 6 15', 'c:18 6 3', 'c:6 18 3', 'p:M18 9a9 9 0 0 1-9 9'], stroke: 1.9);

  /// Oh My Pi (`dash/icons/omp.svg`): a π.
  static const WebIconData omp = WebIconData(<String>[
    'p:M4.5 8c1-1.6 2.4-2.5 4.2-2.5H19.5',
    'p:M9.5 5.5v13',
    'p:M15.5 5.5v9.5a3 3 0 0 0 3 3',
  ]);

  /// Names as in `foundation_icons.json`, for data-driven call sites.
  static const Map<String, WebIconData> byName = <String, WebIconData>{
    'chevron-left': chevronLeft,
    'chevron-right': chevronRight,
    'chevron-down': chevronDown,
    'chevron': chevron,
    'bell': bell,
    'volumeOn': volumeOn,
    'volumeOff': volumeOff,
    'moon': moon,
    'sun': sun,
    'flame': flame,
    'zap': zap,
    'bars': bars,
    'ring': ring,
    'rows': rows,
    'search': search,
    'copy': copy,
    'copySmall': copy,
    'layers': layers,
    'plus': plus,
    'help': help,
    'close': close,
    'x': x,
    'refresh': refresh,
    'pencil': pencil,
    'terminal': terminal,
    'snippet': snippet,
    'anthropic': anthropic,
    'openai': openai,
    'gemini': gemini,
    'pause': pause,
    'play': play,
    'plug': plug,
    'sparkles': sparkles,
    'server': server,
    'lock': lock,
    'database': database,
    'check': check,
    'timer': timer,
    'menu': menu,
    'smartphone': smartphone,
    'settings': settings,
    'cycle': cycle,
    'key': key,
    'arrow-up-down': arrowUpDown,
    'folder': folder,
    'git-branch': gitBranch,
    'omp': omp,
  };
}

/// Paints a [WebIconData] in the ambient icon colour (or [color]).
class WebIcon extends StatelessWidget {
  const WebIcon(this.data, {super.key, this.size, this.color, this.semanticLabel});

  final WebIconData data;
  final double? size;
  final Color? color;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final IconThemeData theme = IconTheme.of(context);
    final double s = size ?? theme.size ?? 16;
    final Color c = color ?? theme.color ?? Orbita.dim;
    final Widget icon = SizedBox.square(
      dimension: s,
      child: CustomPaint(painter: _IconPainter(data, c)),
    );
    if (semanticLabel == null) return ExcludeSemantics(child: icon);
    return Semantics(label: semanticLabel, child: icon);
  }
}

class _IconPainter extends CustomPainter {
  _IconPainter(this.data, this.color);

  final WebIconData data;
  final Color color;

  static final Expando<List<(Path, bool)>> _cache = Expando<List<(Path, bool)>>();

  List<(Path, bool)> _paths() =>
      _cache[data] ??= <(Path, bool)>[for (final String p in data.parts) _primitive(p)];

  @override
  void paint(Canvas canvas, Size size) {
    final double k = size.width / data.box;
    canvas.save();
    canvas.scale(k, k);
    final Paint paint = Paint()
      ..isAntiAlias = true
      ..style = data.fill ? PaintingStyle.fill : PaintingStyle.stroke
      ..strokeWidth = data.stroke
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    for (final (Path path, bool faint) in _paths()) {
      paint.color = faint ? color.withValues(alpha: color.a * .35) : color;
      canvas.drawPath(path, paint);
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_IconPainter old) => old.data != data || old.color != color;
}

(Path, bool) _primitive(String spec) {
  bool faint = false;
  String s = spec;
  if (s.startsWith('~')) {
    faint = true;
    s = s.substring(1);
  }
  final int colon = s.indexOf(':');
  final String kind = s.substring(0, colon);
  final String body = s.substring(colon + 1);
  List<double> nums() => _Tokens(body).allNumbers();
  final Path p = Path();
  switch (kind) {
    case 'p':
      return (svgPath(body), faint);
    case 'c':
      final List<double> n = nums();
      p.addOval(Rect.fromCircle(center: Offset(n[0], n[1]), radius: n[2]));
    case 'e':
      final List<double> n = nums();
      p.addOval(Rect.fromCenter(center: Offset(n[0], n[1]), width: n[2] * 2, height: n[3] * 2));
    case 'r':
      final List<double> n = nums();
      final Rect r = Rect.fromLTWH(n[0], n[1], n[2], n[3]);
      final double rx = n.length > 4 ? n[4] : 0;
      p.addRRect(RRect.fromRectAndRadius(r, Radius.circular(rx)));
    case 'l':
      final List<double> n = nums();
      p
        ..moveTo(n[0], n[1])
        ..lineTo(n[2], n[3]);
    case 'pl':
    case 'pg':
      final List<double> n = nums();
      for (int i = 0; i + 1 < n.length; i += 2) {
        if (i == 0) {
          p.moveTo(n[i], n[i + 1]);
        } else {
          p.lineTo(n[i], n[i + 1]);
        }
      }
      if (kind == 'pg') p.close();
  }
  return (p, faint);
}

class _Tokens {
  _Tokens(this.s);

  final String s;
  int i = 0;

  void _skip() {
    while (i < s.length) {
      final int c = s.codeUnitAt(i);
      if (c == 0x20 || c == 0x2C || c == 0x0A || c == 0x09 || c == 0x0D) {
        i++;
      } else {
        break;
      }
    }
  }

  bool get atNumber {
    _skip();
    if (i >= s.length) return false;
    final int c = s.codeUnitAt(i);
    return (c >= 0x30 && c <= 0x39) || c == 0x2D || c == 0x2B || c == 0x2E;
  }

  String? command() {
    _skip();
    if (i >= s.length) return null;
    final String c = s[i];
    if (RegExp(r'[A-Za-z]').hasMatch(c)) {
      i++;
      return c;
    }
    return null;
  }

  double flag() {
    _skip();
    final int c = s.codeUnitAt(i);
    i++;
    return c == 0x31 ? 1 : 0;
  }

  double number() {
    _skip();
    final int start = i;
    if (i < s.length && (s[i] == '-' || s[i] == '+')) i++;
    bool dot = false;
    while (i < s.length) {
      final String c = s[i];
      if (c == '.') {
        if (dot) break;
        dot = true;
        i++;
      } else if (c.codeUnitAt(0) >= 0x30 && c.codeUnitAt(0) <= 0x39) {
        i++;
      } else if ((c == 'e' || c == 'E') && i + 1 < s.length) {
        i++;
        if (s[i] == '-' || s[i] == '+') i++;
      } else {
        break;
      }
    }
    return double.parse(s.substring(start, i));
  }

  List<double> allNumbers() {
    final List<double> out = <double>[];
    while (atNumber) {
      out.add(number());
    }
    return out;
  }
}

/// Parses SVG path data (M L H V C S Q T A Z, absolute and relative).
Path svgPath(String d) {
  final Path path = Path();
  final _Tokens t = _Tokens(d);
  double cx = 0, cy = 0, sx = 0, sy = 0;
  double? lcx, lcy; // last control point (for S / T)
  String cmd = '';
  String prev = '';
  while (true) {
    final String? c = t.command();
    if (c != null) {
      cmd = c;
    } else if (!t.atNumber || cmd.isEmpty || cmd == 'Z' || cmd == 'z') {
      break;
    }
    final bool rel = cmd == cmd.toLowerCase();
    final double ox = rel ? cx : 0, oy = rel ? cy : 0;
    switch (cmd.toUpperCase()) {
      case 'M':
        cx = ox + t.number();
        cy = oy + t.number();
        sx = cx;
        sy = cy;
        path.moveTo(cx, cy);
        // Further pairs are implicit L / l.
        cmd = rel ? 'l' : 'L';
        lcx = null;
      case 'L':
        cx = ox + t.number();
        cy = oy + t.number();
        path.lineTo(cx, cy);
        lcx = null;
      case 'H':
        cx = ox + t.number();
        path.lineTo(cx, cy);
        lcx = null;
      case 'V':
        cy = (rel ? cy : 0) + t.number();
        path.lineTo(cx, cy);
        lcx = null;
      case 'C':
        final double x1 = ox + t.number(), y1 = oy + t.number();
        final double x2 = ox + t.number(), y2 = oy + t.number();
        cx = ox + t.number();
        cy = oy + t.number();
        path.cubicTo(x1, y1, x2, y2, cx, cy);
        lcx = x2;
        lcy = y2;
      case 'S':
        final bool smooth = 'CcSs'.contains(prev) && lcx != null;
        final double x1 = smooth ? 2 * cx - lcx : cx;
        final double y1 = smooth ? 2 * cy - lcy! : cy;
        final double x2 = ox + t.number(), y2 = oy + t.number();
        cx = ox + t.number();
        cy = oy + t.number();
        path.cubicTo(x1, y1, x2, y2, cx, cy);
        lcx = x2;
        lcy = y2;
      case 'Q':
        final double x1 = ox + t.number(), y1 = oy + t.number();
        cx = ox + t.number();
        cy = oy + t.number();
        path.quadraticBezierTo(x1, y1, cx, cy);
        lcx = x1;
        lcy = y1;
      case 'T':
        final bool smooth = 'QqTt'.contains(prev) && lcx != null;
        final double x1 = smooth ? 2 * cx - lcx : cx;
        final double y1 = smooth ? 2 * cy - lcy! : cy;
        cx = ox + t.number();
        cy = oy + t.number();
        path.quadraticBezierTo(x1, y1, cx, cy);
        lcx = x1;
        lcy = y1;
      case 'A':
        final double rx = t.number(), ry = t.number(), rot = t.number();
        final double large = t.flag(), sweep = t.flag();
        cx = ox + t.number();
        cy = oy + t.number();
        path.arcToPoint(
          Offset(cx, cy),
          radius: Radius.elliptical(rx.abs(), ry.abs()),
          rotation: rot,
          largeArc: large == 1,
          clockwise: sweep == 1,
        );
        lcx = null;
      case 'Z':
        path.close();
        cx = sx;
        cy = sy;
        lcx = null;
      default:
        return path;
    }
    prev = cmd;
  }
  return path;
}

/// Degrees → radians (kept for callers that rotate icons, e.g. the `.chev`).
double webDeg(double d) => d * math.pi / 180;
