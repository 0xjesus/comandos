import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../../ui/theme.dart';

/// A deliberately small markdown subset for agent replies: paragraphs,
/// headings, **bold**, *italic*, `code`, fenced code blocks, bullet and
/// numbered lists, quotes, rules, and pipe tables (shown monospaced).
/// No HTML, no images, no WebView. Parsing is pure and cached per message.

sealed class MdBlock {
  const MdBlock();
}

class MdParagraph extends MdBlock {
  const MdParagraph(this.text, {this.heading = 0, this.quote = false});
  final String text;
  final int heading;
  final bool quote;
}

class MdCode extends MdBlock {
  const MdCode(this.code, {this.table = false});
  final String code;
  final bool table;
}

class MdListItem extends MdBlock {
  const MdListItem(this.text, {required this.marker, this.indent = 0});
  final String text;

  /// "•" or "3."
  final String marker;
  final int indent;
}

class MdRule extends MdBlock {
  const MdRule();
}

final RegExp _fence = RegExp(r'^\s*(```|~~~)');
final RegExp _heading = RegExp(r'^(#{1,6})\s+(.*)$');
final RegExp _bullet = RegExp(r'^(\s*)[-*+]\s+(.*)$');
final RegExp _ordered = RegExp(r'^(\s*)(\d{1,3})[.)]\s+(.*)$');
final RegExp _rule = RegExp(r'^\s*([-*_])(\s*\1){2,}\s*$');

/// Splits markdown into blocks.
List<MdBlock> parseMarkdown(String src) {
  final List<MdBlock> out = <MdBlock>[];
  final List<String> lines = src.replaceAll('\r\n', '\n').split('\n');
  final List<String> para = <String>[];
  bool paraQuote = false;

  void flushPara() {
    if (para.isEmpty) return;
    out.add(MdParagraph(para.join('\n'), quote: paraQuote));
    para.clear();
    paraQuote = false;
  }

  int i = 0;
  while (i < lines.length) {
    final String line = lines[i];
    if (_fence.hasMatch(line)) {
      flushPara();
      final String fence = _fence.firstMatch(line)!.group(1)!;
      final List<String> code = <String>[];
      i++;
      while (i < lines.length && !lines[i].trimLeft().startsWith(fence)) {
        code.add(lines[i]);
        i++;
      }
      i++; // closing fence (or end)
      out.add(MdCode(code.join('\n')));
      continue;
    }
    if (line.trimLeft().startsWith('|')) {
      flushPara();
      final List<String> rows = <String>[];
      while (i < lines.length && lines[i].trimLeft().startsWith('|')) {
        rows.add(lines[i].trim());
        i++;
      }
      out.add(MdCode(rows.join('\n'), table: true));
      continue;
    }
    if (line.trim().isEmpty) {
      flushPara();
      i++;
      continue;
    }
    final RegExpMatch? h = _heading.firstMatch(line);
    if (h != null) {
      flushPara();
      out.add(MdParagraph(h.group(2)!, heading: h.group(1)!.length));
      i++;
      continue;
    }
    if (_rule.hasMatch(line)) {
      flushPara();
      out.add(const MdRule());
      i++;
      continue;
    }
    final RegExpMatch? b = _bullet.firstMatch(line);
    if (b != null) {
      flushPara();
      out.add(MdListItem(b.group(2)!, marker: '•', indent: (b.group(1)!.length / 2).floor().clamp(0, 4)));
      i++;
      continue;
    }
    final RegExpMatch? o = _ordered.firstMatch(line);
    if (o != null) {
      flushPara();
      out.add(MdListItem(o.group(3)!,
          marker: '${o.group(2)}.', indent: (o.group(1)!.length / 2).floor().clamp(0, 4)));
      i++;
      continue;
    }
    final bool quote = line.trimLeft().startsWith('>');
    if (para.isNotEmpty && quote != paraQuote) flushPara();
    paraQuote = quote;
    para.add(quote ? line.trimLeft().substring(1).trimLeft() : line);
    i++;
  }
  flushPara();
  return out;
}

final RegExp _inline = RegExp(r'(`[^`\n]+`)|(\*\*[^*\n]+?\*\*)|(__[^_\n]+?__)|(\*[^*\s][^*\n]*?\*)|(\[[^\]\n]+\]\([^)\s]+\))');

/// Inline spans for one block of text.
List<InlineSpan> inlineSpans(String text, TextStyle base) {
  final List<InlineSpan> spans = <InlineSpan>[];
  int last = 0;
  for (final RegExpMatch m in _inline.allMatches(text)) {
    if (m.start > last) spans.add(TextSpan(text: text.substring(last, m.start)));
    final String tok = m.group(0)!;
    if (m.group(1) != null) {
      spans.add(TextSpan(
        text: tok.substring(1, tok.length - 1),
        style: const TextStyle(
          fontFamily: Orbita.mono,
          color: Color(0xFFCFC8FF),
          backgroundColor: Orbita.panel2,
        ),
      ));
    } else if (m.group(2) != null || m.group(3) != null) {
      spans.add(TextSpan(
          text: tok.substring(2, tok.length - 2),
          style: const TextStyle(fontWeight: FontWeight.w700)));
    } else if (m.group(4) != null) {
      spans.add(TextSpan(
          text: tok.substring(1, tok.length - 1),
          style: const TextStyle(fontStyle: FontStyle.italic)));
    } else {
      final int close = tok.indexOf('](');
      spans.add(TextSpan(
        text: tok.substring(1, close),
        style: const TextStyle(color: Orbita.working, decoration: TextDecoration.underline),
      ));
    }
    last = m.end;
  }
  if (last < text.length) spans.add(TextSpan(text: text.substring(last)));
  return spans;
}

/// Renders parsed blocks. Stateless and allocation-light.
class MarkdownView extends StatelessWidget {
  const MarkdownView(this.blocks, {super.key, this.color = Orbita.text});

  final List<MdBlock> blocks;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final TextStyle base = TextStyle(color: color, fontSize: 15, height: 1.4);
    final List<Widget> children = <Widget>[];
    for (final MdBlock b in blocks) {
      if (children.isNotEmpty) children.add(const SizedBox(height: 8));
      switch (b) {
        case MdParagraph(:final String text, :final int heading, :final bool quote):
          final TextStyle style = heading == 0
              ? base
              : base.copyWith(
                  fontSize: heading <= 1 ? 19 : (heading == 2 ? 17 : 16),
                  fontWeight: FontWeight.w700);
          final Widget t = Text.rich(TextSpan(style: style, children: inlineSpans(text, style)));
          children.add(quote
              ? DecoratedBox(
                  decoration: const BoxDecoration(
                      border: Border(left: BorderSide(color: Orbita.line2, width: 3))),
                  child: Padding(padding: const EdgeInsets.only(left: 10), child: t),
                )
              : t);
        case MdListItem(:final String text, :final String marker, :final int indent):
          children.add(Padding(
            padding: EdgeInsets.only(left: 4.0 + indent * 16),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                SizedBox(
                  width: marker.length > 2 ? 28 : 18,
                  child: Text(marker, style: base.copyWith(color: Orbita.dim)),
                ),
                Expanded(child: Text.rich(TextSpan(style: base, children: inlineSpans(text, base)))),
              ],
            ),
          ));
        case MdCode(:final String code):
          children.add(_CodeBlock(code));
        case MdRule():
          children.add(const Divider(color: Orbita.line2));
      }
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: children);
  }
}

class _CodeBlock extends StatelessWidget {
  const _CodeBlock(this.code);

  final String code;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: Orbita.panel,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: Orbita.line),
      ),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        dragStartBehavior: DragStartBehavior.down,
        padding: const EdgeInsets.all(10),
        child: Text(
          code,
          softWrap: false,
          style: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.text, height: 1.35),
        ),
      ),
    );
  }
}
