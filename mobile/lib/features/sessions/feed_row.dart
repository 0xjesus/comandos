import 'package:flutter/material.dart';

import '../../core/models/session_feed.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'sections.dart';

/// One G4 row of the «Sesiones» feed (`.fs-row`, coarse pointer): avatar with
/// initials and agent badge, name + verb + age + K5 mark, «Tú: …» in one line and
/// the quoted question or reply (3 lines). Immutable: the feed reuses the same
/// instance while nothing it draws changed, so Flutter skips its rebuild.
class FeedRow extends StatelessWidget {
  const FeedRow({
    super.key,
    required this.entry,
    required this.ago,
    required this.onTap,
    this.active = false,
    this.quoteLines = 3,
  });

  final SessionSummary entry;

  /// Already formatted («ahora», «5 min», «3 h»).
  final String ago;
  final bool active;
  final VoidCallback onTap;

  /// `-webkit-line-clamp` of the quote: 3 in the feed, 4 in the preview card.
  final int quoteLines;

  @override
  Widget build(BuildContext context) {
    final SessionSummary e = entry;
    final FeedMark mark = feedMark(e);
    final bool ask = mark == FeedMark.waiting;
    final bool strong = e.unread || ask;
    final String quote = feedQuote(e);
    final String prompt = feedPrompt(e);
    final Color quoteBar = switch (mark) {
      FeedMark.waiting => Orbita.brand,
      FeedMark.unread => Orbita.brand.withValues(alpha: .55),
      _ => Orbita.line2,
    };
    final Widget head = Row(
      crossAxisAlignment: CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: <Widget>[
        Expanded(
          child: Text.rich(
            TextSpan(children: <InlineSpan>[
              TextSpan(
                text: e.label,
                style: TextStyle(
                  color: strong ? Orbita.text : webMix(Orbita.text, .78, Orbita.dim),
                  fontWeight: strong ? FontWeight.w600 : FontWeight.w500,
                ),
              ),
              const TextSpan(text: ' '),
              TextSpan(
                text: feedVerb(e),
                style: ask ? const TextStyle(color: Orbita.brand, fontWeight: FontWeight.w600) : null,
              ),
            ]),
            style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, height: 1.35, color: Orbita.dim),
          ),
        ),
        const SizedBox(width: 8),
        Text(
          ago,
          style: TextStyle(
            fontFamily: Orbita.sans,
            fontSize: 11.5,
            color: e.unread
                ? Orbita.text
                : mark == FeedMark.working
                    ? Orbita.dim
                    : Orbita.faint,
          ),
        ),
        if (mark == FeedMark.unread) ...<Widget>[
          const SizedBox(width: 6),
          const _Dot(size: 6),
        ] else if (mark == FeedMark.working) ...<Widget>[
          const SizedBox(width: 6),
          const WorkRing(size: 8),
        ],
      ],
    );
    return Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 10),
          decoration: active
              ? BoxDecoration(
                  color: Orbita.text.withValues(alpha: .05),
                  borderRadius: BorderRadius.circular(10),
                )
              : null,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              FeedAvatar(label: e.label, agent: e.agent),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    head,
                    if (prompt.isNotEmpty) ...<Widget>[
                      const SizedBox(height: 5),
                      Text.rich(
                        TextSpan(children: <InlineSpan>[
                          const TextSpan(text: 'Tú: '),
                          TextSpan(text: prompt, style: const TextStyle(color: Orbita.dim)),
                        ]),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                            fontFamily: Orbita.sans, fontSize: 11.5, height: 1.35, color: Orbita.faint),
                      ),
                    ],
                    if (quote.isNotEmpty) ...<Widget>[
                      const SizedBox(height: 5),
                      Container(
                        padding: const EdgeInsets.fromLTRB(9, 1, 0, 1),
                        decoration: BoxDecoration(border: Border(left: BorderSide(color: quoteBar, width: 2))),
                        child: Text(
                          quote,
                          maxLines: quoteLines,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: Orbita.sans,
                            fontSize: 12.5,
                            height: 1.4,
                            color: strong ? Orbita.text : Orbita.dim,
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// `.fs-av`: 28 px rounded square with the initials and the agent glyph badge.
class FeedAvatar extends StatelessWidget {
  const FeedAvatar({super.key, required this.label, required this.agent});

  final String label;
  final String agent;

  @override
  Widget build(BuildContext context) {
    final String? glyph = feedGlyph(agent);
    return SizedBox(
      width: 28,
      height: 28,
      child: Stack(
        clipBehavior: Clip.none,
        children: <Widget>[
          Container(
            width: 28,
            height: 28,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: webMix(Orbita.text, .09, Orbita.panel2),
              borderRadius: BorderRadius.circular(9),
            ),
            child: Text(
              feedInitials(label),
              style: const TextStyle(
                fontFamily: Orbita.sans,
                fontSize: 10,
                fontWeight: FontWeight.w600,
                letterSpacing: .2,
                color: Orbita.dim,
              ),
            ),
          ),
          if (glyph != null)
            Positioned(
              right: -4,
              bottom: -4,
              child: Container(
                width: 15,
                height: 15,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: Orbita.panel,
                  shape: BoxShape.circle,
                  border: Border.all(color: Orbita.panel, width: 1.5),
                ),
                child: Text(
                  glyph,
                  style: const TextStyle(
                      fontFamily: Orbita.sans, fontSize: 9, height: 1, fontWeight: FontWeight.w600, color: Orbita.faint),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _Dot extends StatelessWidget {
  const _Dot({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) => Container(
        width: size,
        height: size,
        decoration: const BoxDecoration(color: Orbita.brand, shape: BoxShape.circle),
      );
}

/// The «working» K5 mark: a faint violet ring with a gap at the top. The web
/// spins it; here it is still (no ticker: an idle feed schedules no frames).
class WorkRing extends StatelessWidget {
  const WorkRing({super.key, this.size = 8, this.color});

  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) => SizedBox.square(
        dimension: size,
        child: CustomPaint(painter: _RingPainter(color ?? Orbita.brand.withValues(alpha: .6))),
      );
}

class _RingPainter extends CustomPainter {
  const _RingPainter(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    const double w = 1.5;
    final Rect r = (Offset.zero & size).deflate(w / 2);
    final Paint p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = w;
    // border-top transparent: the quarter around 12 o'clock is missing.
    const double pi = 3.141592653589793;
    canvas.drawArc(r, -pi / 4, pi * 1.5, false, p);
  }

  @override
  bool shouldRepaint(_RingPainter old) => old.color != color;
}
