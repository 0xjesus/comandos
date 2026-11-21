import 'voice_settings.dart';

/// Turns an assistant reply (markdown) into text worth hearing, picks the
/// part to read, and splits it for the speech API. Pure Dart.

/// `/v1/audio/speech` accepts at most 4096 characters per request.
const int kTtsInputLimit = 4096;

/// The first chunk is short so the first audio arrives fast; the rest are
/// larger (fewer requests) but well under [kTtsInputLimit].
const int kFirstChunkChars = 220;
const int kChunkChars = 1200;

/// Replies longer than this are trimmed by [ReadMode.ending] / [ReadMode.summary].
const int kLongReplyChars = 400;

final RegExp _fence = RegExp(r'^\s*(```|~~~)');
final RegExp _toolLine = RegExp(
    r'^\s*([⏺●⎿✻✳⚒]|(Bash|Read|Edit|MultiEdit|Write|Grep|Glob|Task|Agent|WebFetch|WebSearch|TodoWrite|NotebookEdit|Update|Search)\()');
final RegExp _tableSep = RegExp(r'^\s*\|?\s*:?-{2,}');
final RegExp _heading = RegExp(r'^\s{0,3}#{1,6}\s+');
final RegExp _quote = RegExp(r'^\s*>\s?');
final RegExp _bullet = RegExp(r'^\s*([-*+•]|\d{1,3}[.)])\s+');
final RegExp _image = RegExp(r'!\[([^\]]*)\]\([^)]*\)');
final RegExp _link = RegExp(r'\[([^\]]+)\]\([^)]*\)');
final RegExp _url = RegExp(r'\b(?:https?|ftp|file)://\S+|\bwww\.\S+', caseSensitive: false);
final RegExp _html = RegExp(r'</?[a-zA-Z][^>]{0,80}>');
final RegExp _inlineCode = RegExp(r'`([^`\n]+)`');
final RegExp _bold = RegExp(r'(\*\*|__)(?=\S)(.+?)(?<=\S)\1');
final RegExp _italicStar = RegExp(r'(?<![\w*])\*(?=\S)([^*\n]+?)(?<=\S)\*(?![\w*])');
final RegExp _italicUnder = RegExp(r'(?<![\w_])_(?=\S)([^_\n]+?)(?<=\S)_(?![\w_])');
final RegExp _strike = RegExp(r'~~(.+?)~~');
final RegExp _uuid = RegExp(r'\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b');
final RegExp _hexHash = RegExp(r'\b(?=[0-9a-f]*\d)(?=[0-9a-f]*[a-f])[0-9a-f]{7,64}\b');
final RegExp _path = RegExp(r'(?:~|\.{1,2})?(?:/[\w.@+-]+){2,}/?|(?:[\w.@+-]+/){2,}[\w.@+-]+');
final RegExp _longToken = RegExp(r'\S{41,}');
final RegExp _spaces = RegExp(r'[ \t ]+');
final RegExp _sentenceEnd = RegExp(r'[.!?…:;]$');

/// Markdown → plain speakable text. Paragraphs are separated by "\n\n".
String prepareForSpeech(String markdown) {
  final List<String> paragraphs = <String>[];
  final StringBuffer para = StringBuffer();

  void flush() {
    final String p = para.toString().replaceAll(_spaces, ' ').trim();
    para.clear();
    if (p.isNotEmpty && RegExp(r'[\p{L}\p{N}]', unicode: true).hasMatch(p)) paragraphs.add(p);
  }

  void addSentence(String s) {
    final String t = s.trim();
    if (t.isEmpty) return;
    if (para.isNotEmpty) para.write(' ');
    para.write(t);
  }

  final List<String> lines = markdown.replaceAll('\r\n', '\n').split('\n');
  int i = 0;
  while (i < lines.length) {
    final String raw = lines[i];
    if (_fence.hasMatch(raw)) {
      int j = i + 1;
      while (j < lines.length && !_fence.hasMatch(lines[j])) {
        j++;
      }
      final int n = j - i - 1;
      flush();
      paragraphs.add(n > 1 ? 'Bloque de código de $n líneas.' : 'Bloque de código.');
      i = j + 1;
      continue;
    }
    i++;
    if (raw.trim().isEmpty) {
      flush();
      continue;
    }
    if (_toolLine.hasMatch(raw) || _tableSep.hasMatch(raw)) continue;
    String line = raw;
    final bool isItem = _bullet.hasMatch(line) || _heading.hasMatch(line) || line.trimLeft().startsWith('|');
    if (_heading.hasMatch(line)) {
      flush();
      line = line.replaceFirst(_heading, '');
    }
    line = line.replaceFirst(_quote, '').replaceFirst(_bullet, '');
    if (line.trimLeft().startsWith('|')) {
      line = line
          .split('|')
          .map((String c) => c.trim())
          .where((String c) => c.isNotEmpty)
          .join(', ');
    }
    line = _inline(line);
    if (isItem && line.isNotEmpty && !_sentenceEnd.hasMatch(line)) line = '$line.';
    addSentence(line);
  }
  flush();
  return paragraphs.join('\n\n');
}

String _inline(String s) {
  String t = s
      .replaceAllMapped(_image, (Match m) => m[1] ?? '')
      .replaceAllMapped(_link, (Match m) => m[1] ?? '')
      .replaceAll(_url, 'un enlace')
      .replaceAll(_html, '');
  t = t.replaceAllMapped(_inlineCode, (Match m) {
    final String code = m[1]!.trim();
    if (_path.hasMatch(code) && code.contains('/')) return _shortPath(code);
    return code.length <= 40 ? code : 'código';
  });
  t = t
      .replaceAllMapped(_bold, (Match m) => m[2]!)
      .replaceAllMapped(_strike, (Match m) => m[1]!)
      .replaceAllMapped(_italicStar, (Match m) => m[1]!)
      .replaceAllMapped(_italicUnder, (Match m) => m[1]!)
      .replaceAll(_uuid, '')
      .replaceAll(_hexHash, '')
      .replaceAllMapped(_path, (Match m) => _shortPath(m[0]!))
      .replaceAll(_longToken, '')
      .replaceAll(RegExp(r'[*_`#]{2,}'), '')
      .replaceAll(_spaces, ' ')
      .replaceAll(RegExp(r' ([,.;:!?])'), r'$1');
  return t.trim();
}

/// `lib/features/voice/voice.dart` → `voice.dart`.
String _shortPath(String p) {
  final List<String> parts = p.split('/').where((String s) => s.isNotEmpty).toList();
  return parts.isEmpty ? '' : parts.last;
}

/// Picks what to read. [mode] only trims replies longer than [kLongReplyChars].
String selectForReading(String prepared, ReadMode mode) {
  final String text = prepared.trim();
  if (mode == ReadMode.full || text.length <= kLongReplyChars) return text;
  final List<String> paras = text.split('\n\n').where((String p) => p.trim().isNotEmpty).toList();
  if (paras.isEmpty) return '';
  switch (mode) {
    case ReadMode.ending:
      // A short closing line ("¿Sigo?") alone says little: add the one before.
      String last = paras.last;
      if (last.length < 80 && paras.length > 1) last = '${paras[paras.length - 2]}\n\n$last';
      return last;
    case ReadMode.summary:
      final List<String> sentences = splitSentences(text.replaceAll('\n\n', ' '));
      final StringBuffer b = StringBuffer();
      for (final String s in sentences) {
        if (b.isNotEmpty && b.length + s.length > 300) break;
        if (b.isNotEmpty) b.write(' ');
        b.write(s);
        if (b.length >= 160) break;
      }
      return b.toString();
    case ReadMode.full:
      return text;
  }
}

/// Sentences, keeping their final punctuation.
List<String> splitSentences(String text) => text
    .split(RegExp(r'(?<=[.!?…])\s+|\n+'))
    .map((String s) => s.trim())
    .where((String s) => s.isNotEmpty)
    .toList();

/// Splits [text] at sentence boundaries into chunks: the first at most
/// [first] chars, the rest at most [rest]. A sentence longer than a chunk is
/// cut at a comma or space, never mid-word unless a single word is longer.
List<String> chunkForSpeech(String text, {int first = kFirstChunkChars, int rest = kChunkChars}) {
  assert(first > 0 && rest > 0 && rest <= kTtsInputLimit);
  final List<String> out = <String>[];
  final StringBuffer cur = StringBuffer();
  int limit() => out.isEmpty ? first : rest;

  void emit() {
    final String c = cur.toString().trim();
    cur.clear();
    if (c.isNotEmpty) out.add(c);
  }

  for (String s in splitSentences(text)) {
    while (s.length > limit()) {
      if (cur.isNotEmpty) emit();
      final int max = limit();
      int cut = s.lastIndexOf(RegExp(r'[,;:]\s'), max - 1);
      if (cut < max ~/ 3) cut = s.lastIndexOf(' ', max);
      if (cut < max ~/ 3) cut = max - 1;
      out.add(s.substring(0, cut + 1).trim());
      s = s.substring(cut + 1).trim();
    }
    if (s.isEmpty) continue;
    if (cur.isNotEmpty && cur.length + 1 + s.length > limit()) emit();
    if (cur.isNotEmpty) cur.write(' ');
    cur.write(s);
  }
  emit();
  return out;
}

/// Everything at once: prepare, select, chunk.
List<String> speechChunksFor(String markdown, ReadMode mode) =>
    chunkForSpeech(selectForReading(prepareForSpeech(markdown), mode));

/// FNV-1a 64-bit hex digest (cache keys / reply ids; not cryptographic).
String fnv64(String s) {
  // Two 32-bit halves keep this exact on every platform.
  int h1 = 0x811c9dc5, h2 = 0x050c5d1f;
  for (final int c in s.codeUnits) {
    h1 = ((h1 ^ c) * 0x01000193) & 0xffffffff;
    h2 = ((h2 ^ (c + 0x9e)) * 0x01000193) & 0xffffffff;
  }
  return '${h1.toRadixString(16).padLeft(8, '0')}${h2.toRadixString(16).padLeft(8, '0')}';
}
