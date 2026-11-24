import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'chat_media.dart';
import 'code_tokens.dart';

/// Agent markdown as the web chat draws it (`comandos_core::chat_markdown`
/// rendered with the `#chat-view .cv-body` rules of `index_page.css`):
/// headings h1–h6, paragraphs, **bold**, *italic*, ~~strike~~, `code` chips,
/// links (http/https/mailto only; other targets stay text with a dotted
/// underline), bare addresses, bullet/numbered/task lists with brand markers,
/// quotes, `> [!NOTE]` alerts, rules, fenced code with its language and a
/// Copiar button and coloured code, and pipe tables that scroll sideways.
/// Internet images (GIFs move) are drawn under the text that names them;
/// videos, PDFs and Mermaid diagrams are cards that open them full screen.
/// Parsing is pure and cached per message; no HTML, no WebView in the feed.

sealed class MdBlock {
  const MdBlock();
}

class MdParagraph extends MdBlock {
  const MdParagraph(this.text, {this.heading = 0});
  final String text;
  final int heading;
}

class MdQuote extends MdBlock {
  const MdQuote(this.blocks, {this.alert});
  final List<MdBlock> blocks;

  /// `note` | `tip` | `important` | `warning` | `caution`, or null.
  final String? alert;
}

class MdCode extends MdBlock {
  const MdCode(this.code, {this.lang = ''});
  final String code;
  final String lang;
}

class MdListItem extends MdBlock {
  const MdListItem(this.text, {required this.marker, this.indent = 0, this.checked});
  final String text;

  /// "•" or "3."
  final String marker;
  final int indent;

  /// Task list box: true ☑, false ☐, null none.
  final bool? checked;
}

enum MdAlign { left, center, right }

class MdTable extends MdBlock {
  const MdTable(this.header, this.rows, this.align);
  final List<String> header;
  final List<List<String>> rows;
  final List<MdAlign> align;
}

class MdRule extends MdBlock {
  const MdRule();
}

final RegExp _fence = RegExp(r'^\s{0,3}(```+|~~~+)\s*([^`\s]*)');
final RegExp _heading = RegExp(r'^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$');
final RegExp _bullet = RegExp(r'^(\s*)[-*+]\s+(.*)$');
final RegExp _ordered = RegExp(r'^(\s*)(\d{1,9})[.)]\s+(.*)$');
final RegExp _rule = RegExp(r'^\s{0,3}([-*_])(\s*\1){2,}\s*$');
final RegExp _task = RegExp(r'^\[([ xX])\]\s+(.*)$');
final RegExp _alert = RegExp(r'^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*$', caseSensitive: false);
final RegExp _tableSep = RegExp(r'^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$');

/// Language of a fence info string: «rust» from «Rust title="x"» (≤24 chars).
String codeLang(String info) {
  String first = info.split(RegExp(r'\s+')).first.split(',').first;
  first = first.replaceAll(RegExp(r'[{}.]'), '');
  final String clean = first.replaceAll(RegExp(r'[^A-Za-z0-9_+#-]'), '').toLowerCase();
  return clean.length > 24 ? clean.substring(0, 24) : clean;
}

List<String> _cells(String row) {
  String r = row.trim();
  if (r.startsWith('|')) r = r.substring(1);
  if (r.endsWith('|') && !r.endsWith(r'\|')) r = r.substring(0, r.length - 1);
  final List<String> out = <String>[];
  final StringBuffer cur = StringBuffer();
  bool inCode = false;
  for (int i = 0; i < r.length; i++) {
    final String c = r[i];
    if (c == '\\' && i + 1 < r.length && r[i + 1] == '|') {
      cur.write('|');
      i++;
    } else if (c == '`') {
      inCode = !inCode;
      cur.write(c);
    } else if (c == '|' && !inCode) {
      out.add(cur.toString().trim());
      cur.clear();
    } else {
      cur.write(c);
    }
  }
  out.add(cur.toString().trim());
  return out;
}

/// Splits markdown into blocks.
List<MdBlock> parseMarkdown(String src) {
  final List<String> lines = src.replaceAll('\r\n', '\n').split('\n');
  return _parseLines(lines, 0);
}

List<MdBlock> _parseLines(List<String> lines, int depth) {
  final List<MdBlock> out = <MdBlock>[];
  final List<String> para = <String>[];

  void flushPara() {
    if (para.isEmpty) return;
    out.add(MdParagraph(para.join('\n')));
    para.clear();
  }

  int i = 0;
  while (i < lines.length) {
    final String line = lines[i];
    final RegExpMatch? f = _fence.firstMatch(line);
    if (f != null) {
      flushPara();
      final String fence = f.group(1)!;
      final String lang = codeLang(line.trim().substring(fence.length));
      final List<String> code = <String>[];
      i++;
      while (i < lines.length && !lines[i].trimLeft().startsWith(fence)) {
        code.add(lines[i]);
        i++;
      }
      i++; // closing fence (or end)
      out.add(MdCode(code.join('\n'), lang: lang));
      continue;
    }
    if (line.trimLeft().startsWith('>') && depth < 4) {
      flushPara();
      final List<String> inner = <String>[];
      while (i < lines.length && lines[i].trimLeft().startsWith('>')) {
        String l = lines[i].trimLeft().substring(1);
        if (l.startsWith(' ')) l = l.substring(1);
        inner.add(l);
        i++;
      }
      String? alert;
      if (inner.isNotEmpty) {
        final RegExpMatch? a = _alert.firstMatch(inner.first.trim());
        if (a != null) {
          alert = a.group(1)!.toLowerCase();
          inner.removeAt(0);
        }
      }
      out.add(MdQuote(_parseLines(inner, depth + 1), alert: alert));
      continue;
    }
    if (line.contains('|') &&
        i + 1 < lines.length &&
        _tableSep.hasMatch(lines[i + 1]) &&
        lines[i + 1].contains('-')) {
      flushPara();
      final List<String> header = _cells(line);
      final List<MdAlign> align = <MdAlign>[
        for (final String c in _cells(lines[i + 1]))
          c.startsWith(':') && c.endsWith(':')
              ? MdAlign.center
              : c.endsWith(':')
                  ? MdAlign.right
                  : MdAlign.left,
      ];
      i += 2;
      final List<List<String>> rows = <List<String>>[];
      while (i < lines.length && lines[i].contains('|') && lines[i].trim().isNotEmpty) {
        rows.add(_cells(lines[i]));
        i++;
      }
      out.add(MdTable(header, rows, align));
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
    final RegExpMatch? o = b == null ? _ordered.firstMatch(line) : null;
    if (b != null || o != null) {
      flushPara();
      final String lead = (b ?? o)!.group(1)!;
      String text = b != null ? b.group(2)! : o!.group(3)!;
      final int contentAt = line.length - text.length;
      // Lazy continuation lines indented under the item belong to it.
      i++;
      while (i < lines.length &&
          lines[i].trim().isNotEmpty &&
          lines[i].length - lines[i].trimLeft().length >= contentAt &&
          !_bullet.hasMatch(lines[i]) &&
          !_ordered.hasMatch(lines[i]) &&
          _fence.firstMatch(lines[i]) == null) {
        text = '$text\n${lines[i].trim()}';
        i++;
      }
      bool? checked;
      final RegExpMatch? t = _task.firstMatch(text);
      if (t != null) {
        checked = t.group(1) != ' ';
        text = t.group(2)!;
      }
      out.add(MdListItem(
        text,
        marker: b != null ? '•' : '${o!.group(2)}.',
        indent: (lead.replaceAll('\t', '    ').length / 2).floor().clamp(0, 5),
        checked: checked,
      ));
      continue;
    }
    para.add(line);
    i++;
  }
  flushPara();
  return out;
}

/// Plain text of the blocks, for a clipboard (`chat_markdown::to_plain`):
/// no `**`, bullets as «•», tables as tab-separated rows.
String markdownPlain(List<MdBlock> blocks) {
  final List<String> parts = <String>[];
  for (final MdBlock b in blocks) {
    switch (b) {
      case MdParagraph(:final String text):
        parts.add(inlinePlain(text));
      case MdListItem(:final String text, :final String marker, :final int indent, :final bool? checked):
        final String box = checked == null ? '$marker ' : (checked ? '☑ ' : '☐ ');
        parts.add('${'  ' * indent}$box${inlinePlain(text)}');
      case MdQuote(:final List<MdBlock> blocks):
        parts.add(markdownPlain(blocks).split('\n').map((String l) => '> $l').join('\n'));
      case MdCode(:final String code):
        parts.add(code);
      case MdTable(:final List<String> header, :final List<List<String>> rows):
        parts.add(<List<String>>[header, ...rows]
            .map((List<String> r) => r.map(inlinePlain).join('\t'))
            .join('\n'));
      case MdRule():
        parts.add('———');
    }
  }
  // Consecutive list items sit on adjacent lines.
  final StringBuffer sb = StringBuffer();
  MdBlock? prev;
  int k = 0;
  for (final MdBlock b in blocks) {
    if (k > 0) sb.write(prev is MdListItem && b is MdListItem ? '\n' : '\n\n');
    sb.write(parts[k]);
    prev = b;
    k++;
  }
  return sb.toString();
}

final RegExp _inline = RegExp(
  r'(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)' // 1,2 code
  r'|\*\*([\s\S]+?)\*\*' // 3 bold
  r'|__([\s\S]+?)__' // 4 bold
  r'|~~([\s\S]+?)~~' // 5 strike
  r'|\*(?=\S)([\s\S]*?\S)\*' // 6 italic
  r'|(?<![A-Za-z0-9])_(?=\S)([\s\S]*?\S)_(?![A-Za-z0-9])' // 7 italic
  r'|!?\[([^\]\n]*)\]\(\s*<?([^)\s>]*)>?(?:\s+"[^"]*")?\s*\)' // 8 text, 9 url
  r'|<((?:https?://|mailto:)[^>\s]+)>' // 10 autolink
  r'|(https?://[^\s<>"`]+)', // 11 bare
);

String? safeHref(String url) {
  final String u = url.trim();
  final String l = u.toLowerCase();
  if ((l.startsWith('http://') && l.length > 7) || (l.startsWith('https://') && l.length > 8) || l.startsWith('mailto:')) {
    return u;
  }
  return null;
}

/// A bare address without the punctuation that ends a sentence.
String _trimBare(String url) {
  String u = url;
  while (u.isNotEmpty && '.,;:!?)]\'*_'.contains(u[u.length - 1])) {
    if (u.endsWith(')') && '('.allMatches(u).length >= ')'.allMatches(u).length) break;
    u = u.substring(0, u.length - 1);
  }
  return u;
}

String inlinePlain(String text) {
  return text.replaceAllMapped(_inline, (Match m) {
    if (m.group(2) != null) return m.group(2)!;
    final String? inner = m.group(3) ?? m.group(4) ?? m.group(5) ?? m.group(6) ?? m.group(7);
    if (inner != null) return inlinePlain(inner);
    if (m.group(8) != null) {
      final String? href = safeHref(m.group(9)!);
      return href == null || href == m.group(8) ? m.group(8)! : '${m.group(8)} ($href)';
    }
    return m.group(0)!;
  });
}

/// Internet media named in [text] (images, links and bare addresses to an
/// image, a video or a PDF), each once, in order. Cached per block.
List<MediaRef> mediaIn(String text) {
  final List<MediaRef> out = <MediaRef>[];
  final Set<String> seen = <String>{};
  void add(String url, String alt) {
    final MediaKind? kind = mediaOf(url);
    if (kind != null && seen.add(url)) out.add(MediaRef(kind, url, alt));
  }

  for (final RegExpMatch m in _inline.allMatches(text)) {
    if (m.group(2) != null) continue; // code
    final String? inner = m.group(3) ?? m.group(4) ?? m.group(5) ?? m.group(6) ?? m.group(7);
    if (inner != null) {
      for (final MediaRef r in mediaIn(inner)) {
        if (seen.add(r.url)) out.add(r);
      }
    } else if (m.group(8) != null) {
      final String? href = safeHref(m.group(9)!);
      if (href != null) add(href, m.group(8)!);
    } else if (m.group(10) != null) {
      add(m.group(10)!, '');
    } else if (m.group(11) != null) {
      add(_trimBare(m.group(11)!), '');
    }
  }
  return out;
}

final Expando<List<MediaRef>> _mediaCache = Expando<List<MediaRef>>('media');

List<MediaRef> _mediaOf(MdBlock b, String text) => _mediaCache[b] ??= mediaIn(text);

/// Text left once its internet images are taken out (they are drawn below).
bool _onlyImages(String text) =>
    text.replaceAllMapped(_inline, (Match m) {
      final bool drawn = m.group(0)!.startsWith('!') &&
          m.group(9) != null &&
          safeHref(m.group(9)!) != null &&
          mediaOf(m.group(9)!) == MediaKind.image;
      return drawn ? '' : m.group(0)!;
    }).trim().isEmpty;

/// [text] followed by the media it names.
Widget _withMedia(MdBlock b, String source, Widget text) {
  final List<MediaRef> media = _mediaOf(b, source);
  if (media.isEmpty) return text;
  return Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    mainAxisSize: MainAxisSize.min,
    children: <Widget>[
      if (!_onlyImages(source)) ...<Widget>[text, const SizedBox(height: 8)],
      for (final (int i, MediaRef r) in media.indexed) ...<Widget>[
        if (i > 0) const SizedBox(height: 8),
        MediaPreview(r),
      ],
    ],
  );
}

/// Called when a link is tapped. The app has no browser hand-off yet, so the
/// view copies the address and says so; replaceable for tests.
void Function(String url) markdownLinkTap = (String url) {
  Clipboard.setData(ClipboardData(text: url));
};

/// Colours of the text around the inline marks (agent card vs. own bubble).
class MdInk {
  const MdInk({required this.text, required this.link, required this.codeBg, required this.marker, required this.dotted});

  final Color text;
  final Color link;
  final Color codeBg;
  final Color marker;
  final Color dotted;

  /// `.cv-ai`: links and list markers in `--brand`.
  static final MdInk ai = MdInk(
    text: Orbita.text,
    link: Orbita.brand,
    codeBg: Orbita.line2.withValues(alpha: .55),
    marker: Orbita.brand,
    dotted: Orbita.faint,
  );

  /// `.cv-me`: white on the brand bubble.
  static final MdInk me = MdInk(
    text: Colors.white,
    link: Colors.white,
    codeBg: Colors.black.withValues(alpha: .22),
    marker: Colors.white,
    dotted: Colors.white70,
  );
}

/// Inline spans for one block of text.
List<InlineSpan> inlineSpans(String text, TextStyle base, [MdInk? ink]) {
  final MdInk k = ink ?? MdInk.ai;
  final List<InlineSpan> spans = <InlineSpan>[];
  int last = 0;
  void plain(String s) {
    if (s.isNotEmpty) spans.add(TextSpan(text: s));
  }

  for (final RegExpMatch m in _inline.allMatches(text)) {
    if (m.start < last) continue;
    plain(text.substring(last, m.start));
    if (m.group(2) != null) {
      spans.add(TextSpan(
        text: ' ${m.group(2)!.trim()} ',
        style: TextStyle(
          fontFamily: Orbita.mono,
          fontSize: (base.fontSize ?? 15) * .86,
          backgroundColor: k.codeBg,
        ),
      ));
    } else if (m.group(3) != null || m.group(4) != null) {
      final TextStyle s = base.copyWith(fontWeight: FontWeight.w700);
      spans.add(TextSpan(style: const TextStyle(fontWeight: FontWeight.w700), children: inlineSpans(m.group(3) ?? m.group(4)!, s, k)));
    } else if (m.group(5) != null) {
      spans.add(TextSpan(
          style: const TextStyle(decoration: TextDecoration.lineThrough),
          children: inlineSpans(m.group(5)!, base, k)));
    } else if (m.group(6) != null || m.group(7) != null) {
      spans.add(TextSpan(
          style: const TextStyle(fontStyle: FontStyle.italic), children: inlineSpans(m.group(6) ?? m.group(7)!, base, k)));
    } else if (m.group(8) != null) {
      final String label = m.group(8)!.isEmpty ? m.group(9)! : m.group(8)!;
      final String? href = safeHref(m.group(9)!);
      final bool image = m.group(0)!.startsWith('!');
      // An internet image is drawn under the text (`MediaPreview`).
      if (!(image && href != null && mediaOf(href) == MediaKind.image)) {
        spans.add(_link(image ? '🖼 $label' : label, href, k));
      }
    } else if (m.group(10) != null) {
      spans.add(_link(m.group(10)!, m.group(10), k));
    } else {
      final String raw = m.group(11)!;
      final String url = _trimBare(raw);
      spans.add(_link(url, url, k));
      plain(raw.substring(url.length));
    }
    last = m.end;
  }
  plain(text.substring(last));
  return spans;
}

TextSpan _link(String label, String? href, MdInk k) {
  if (href == null) {
    // `.cv-link`: not an address that opens anything; dotted underline.
    return TextSpan(
      text: label,
      style: TextStyle(decoration: TextDecoration.underline, decorationStyle: TextDecorationStyle.dotted, decorationColor: k.dotted),
    );
  }
  return TextSpan(
    text: label,
    style: TextStyle(color: k.link, decoration: TextDecoration.underline, decorationColor: k.link),
    recognizer: TapGestureRecognizer()..onTap = () => markdownLinkTap(href),
  );
}

/// Renders parsed blocks with the `.cv-body` rules (gap 10 between blocks).
class MarkdownView extends StatelessWidget {
  const MarkdownView(this.blocks, {super.key, this.fontSize = 15, this.lineHeight = 1.6, this.me = false});

  final List<MdBlock> blocks;
  final double fontSize;
  final double lineHeight;

  /// Inside the user's own brand bubble.
  final bool me;

  @override
  Widget build(BuildContext context) {
    final MdInk ink = me ? MdInk.me : MdInk.ai;
    final TextStyle base = TextStyle(
      color: ink.text,
      fontFamily: Orbita.sans,
      fontSize: fontSize,
      height: lineHeight,
    );
    return _blocks(blocks, base, ink);
  }
}

const List<double> _headingSize = <double>[20, 18, 16.5, 15.5, 14.5, 13.5];

Widget _blocks(List<MdBlock> blocks, TextStyle base, MdInk ink, {double gap = 10}) {
  final List<Widget> children = <Widget>[];
  MdBlock? prev;
  for (final MdBlock b in blocks) {
    if (children.isNotEmpty) {
      // List items of one list are 5 px apart (`ul{gap:5px}`).
      children.add(SizedBox(height: prev is MdListItem && b is MdListItem ? 5 : gap));
    }
    children.add(_block(b, base, ink));
    prev = b;
  }
  return Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: children);
}

Widget _block(MdBlock b, TextStyle base, MdInk ink) {
  switch (b) {
    case MdParagraph(:final String text, :final int heading):
      if (heading == 0) return _withMedia(b, text, Text.rich(TextSpan(style: base, children: inlineSpans(text, base, ink))));
      final TextStyle s = base.copyWith(
        fontSize: _headingSize[heading - 1],
        fontWeight: FontWeight.w700,
        height: 1.3,
        color: heading == 6 ? Orbita.dim : base.color,
      );
      return Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Text.rich(TextSpan(style: s, children: inlineSpans(text, s, ink))),
      );
    case MdListItem(:final String text, :final String marker, :final int indent, :final bool? checked):
      final bool bullet = marker == '•';
      final Widget mark = checked != null
          ? Padding(
              padding: EdgeInsets.only(top: (base.fontSize! * base.height! - 17) / 2),
              child: _CheckBox(checked: checked),
            )
          : bullet
              ? SizedBox(
                  height: base.fontSize! * base.height!,
                  child: Center(
                    child: Container(
                      width: 6,
                      height: 6,
                      decoration: BoxDecoration(color: ink.marker, shape: BoxShape.circle),
                    ),
                  ),
                )
              : Text(marker, style: base.copyWith(color: ink.marker), textAlign: TextAlign.right);
      return Padding(
        padding: EdgeInsets.only(left: indent * 24.0),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            SizedBox(width: checked != null ? 24 : (bullet ? 24 : 26), child: Align(alignment: Alignment.topRight, child: Padding(padding: const EdgeInsets.only(right: 8), child: mark))),
            Expanded(child: _withMedia(b, text, Text.rich(TextSpan(style: base, children: inlineSpans(text, base, ink))))),
          ],
        ),
      );
    case MdQuote(:final List<MdBlock> blocks, :final String? alert):
      if (alert != null) return _Alert(kind: alert, blocks: blocks, base: base, ink: ink);
      final TextStyle s = base.copyWith(color: Orbita.dim);
      return Container(
        padding: const EdgeInsets.fromLTRB(12, 2, 0, 2),
        decoration: const BoxDecoration(border: Border(left: BorderSide(color: Orbita.brand, width: 3))),
        child: _blocks(blocks, s, ink, gap: 8),
      );
    case MdCode(:final String code, :final String lang):
      if (lang == 'mermaid') {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[MermaidCard(code), const SizedBox(height: 8), CodeBlockView(code: code, lang: lang)],
        );
      }
      return CodeBlockView(code: code, lang: lang);
    case MdTable():
      return _TableView(b, base: base, ink: ink);
    case MdRule():
      return Container(height: 1, color: Orbita.line);
  }
}

class _CheckBox extends StatelessWidget {
  const _CheckBox({required this.checked});
  final bool checked;

  @override
  Widget build(BuildContext context) => Container(
        width: 17,
        height: 17,
        decoration: BoxDecoration(
          color: checked ? Orbita.brand : Colors.transparent,
          borderRadius: BorderRadius.circular(3),
          border: Border.all(color: checked ? Orbita.brand : Orbita.dim, width: 1.5),
        ),
        child: checked ? const Icon(Icons.check, size: 13, color: Colors.white) : null,
      );
}

class _Alert extends StatelessWidget {
  const _Alert({required this.kind, required this.blocks, required this.base, required this.ink});
  final String kind;
  final List<MdBlock> blocks;
  final TextStyle base;
  final MdInk ink;

  @override
  Widget build(BuildContext context) {
    final (Color c, String title, double mix) = switch (kind) {
      'warning' => (Orbita.waiting, 'Advertencia', .09),
      'caution' => (Orbita.waiting, 'Precaución', .09),
      'tip' => (Orbita.done, 'Consejo', .08),
      'important' => (Orbita.working, 'Importante', .08),
      _ => (Orbita.working, 'Nota', .08),
    };
    final TextStyle s = base.copyWith(color: Orbita.text);
    return Container(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: Orbita.line),
        color: c.withValues(alpha: mix),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(10),
        child: DecoratedBox(
          decoration: BoxDecoration(border: Border(left: BorderSide(color: c, width: 4))),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                Text(title, style: s.copyWith(fontWeight: FontWeight.w700)),
                if (blocks.isNotEmpty) const SizedBox(height: 8),
                if (blocks.isNotEmpty) _blocks(blocks, s, ink, gap: 8),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// `.cv-code`: language and Copiar on top, the code scrolls sideways.
class CodeBlockView extends StatefulWidget {
  const CodeBlockView({super.key, required this.code, this.lang = ''});
  final String code;
  final String lang;

  @override
  State<CodeBlockView> createState() => _CodeBlockViewState();
}

class _CodeBlockViewState extends State<CodeBlockView> {
  bool _ok = false;
  late List<TextSpan> _spans = codeSpans(widget.lang, widget.code);

  @override
  void didUpdateWidget(CodeBlockView old) {
    super.didUpdateWidget(old);
    if (old.code != widget.code || old.lang != widget.lang) _spans = codeSpans(widget.lang, widget.code);
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.code));
    if (!mounted) return;
    setState(() => _ok = true);
    await Future<void>.delayed(const Duration(milliseconds: 1600));
    if (mounted) setState(() => _ok = false);
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: Orbita.bg,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: Orbita.line),
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: <Widget>[
          Container(
            constraints: const BoxConstraints(minHeight: 44),
            padding: const EdgeInsets.fromLTRB(12, 0, 4, 0),
            decoration: BoxDecoration(
              color: webMix(Orbita.panel, .55, Orbita.bg),
              border: const Border(bottom: BorderSide(color: Orbita.line)),
            ),
            child: Row(
              children: <Widget>[
                Expanded(
                  child: SelectionContainer.disabled(
                    child: Text(
                      widget.lang.isEmpty ? 'código' : widget.lang,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12, fontWeight: FontWeight.w600, color: Orbita.faint),
                    ),
                  ),
                ),
                SelectionContainer.disabled(
                  child: WebButton(
                    onTap: _copy,
                    tooltip: 'Copiar código',
                    face: Orbita.bg,
                    ink: _ok ? Orbita.done : Orbita.text,
                    on: _ok,
                    radius: 9,
                    lip: 2,
                    minHeight: 36,
                    padding: const EdgeInsets.symmetric(horizontal: 14),
                    child: Text(_ok ? 'Copiado ✓' : 'Copiar', style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600)),
                  ),
                ),
              ],
            ),
          ),
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            dragStartBehavior: DragStartBehavior.down,
            padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 11),
            child: Text.rich(
              TextSpan(children: _spans),
              softWrap: false,
              style: const TextStyle(fontFamily: Orbita.mono, fontSize: 12.5, height: 1.55, color: Orbita.text),
            ),
          ),
        ],
      ),
    );
  }
}

class _TableView extends StatelessWidget {
  const _TableView(this.t, {required this.base, required this.ink});
  final MdTable t;
  final TextStyle base;
  final MdInk ink;

  @override
  Widget build(BuildContext context) {
    final int cols = <int>[t.header.length, for (final List<String> r in t.rows) r.length].reduce((int a, int b) => a > b ? a : b);
    final TextStyle s = base.copyWith(fontSize: 13.5, height: 1.4);
    TableCell cell(String text, int c, {bool head = false}) {
      final MdAlign a = c < t.align.length ? t.align[c] : MdAlign.left;
      final TextStyle st = head ? s.copyWith(fontWeight: FontWeight.w600) : s;
      return TableCell(
        child: Container(
          constraints: const BoxConstraints(minWidth: 94),
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
          color: head ? Orbita.line2.withValues(alpha: .35) : null,
          child: Text.rich(
            TextSpan(style: st, children: inlineSpans(text, st, ink)),
            textAlign: switch (a) { MdAlign.center => TextAlign.center, MdAlign.right => TextAlign.right, MdAlign.left => TextAlign.left },
          ),
        ),
      );
    }

    List<String> pad(List<String> r) => <String>[...r, for (int i = r.length; i < cols; i++) ''];
    return Container(
      decoration: BoxDecoration(borderRadius: BorderRadius.circular(10), border: Border.all(color: Orbita.line)),
      clipBehavior: Clip.antiAlias,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        dragStartBehavior: DragStartBehavior.down,
        child: Table(
          defaultColumnWidth: const IntrinsicColumnWidth(),
          border: const TableBorder(
            horizontalInside: BorderSide(color: Orbita.line),
            verticalInside: BorderSide(color: Orbita.line),
          ),
          children: <TableRow>[
            TableRow(children: <Widget>[for (final (int c, String h) in pad(t.header).indexed) cell(h, c, head: true)]),
            for (final List<String> r in t.rows)
              TableRow(children: <Widget>[for (final (int c, String v) in pad(r).indexed) cell(v, c)]),
          ],
        ),
      ),
    );
  }
}
