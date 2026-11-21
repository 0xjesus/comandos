/// "ahora", "hace 3 min", "hace 2 h", "ayer", "hace 4 d". [nowSec] and
/// [tsSec] come from the same (server) clock.
String relativeTime(double nowSec, double tsSec) {
  if (tsSec <= 0) return '';
  final int s = (nowSec - tsSec).round();
  if (s < 45) return 'ahora';
  final int m = (s / 60).round();
  if (m < 60) return 'hace $m min';
  final int h = (m / 60).floor();
  if (h < 24) return 'hace $h h';
  final int d = (h / 24).floor();
  if (d == 1) return 'ayer';
  return 'hace $d d';
}

/// Lowercase without accents, for instant search ("dueño" ≈ "dueno").
String foldForSearch(String s) {
  const String from = 'áàäâãéèëêíìïîóòöôõúùüûñç';
  const String to = 'aaaaaeeeeiiiiooooouuuunc';
  final StringBuffer b = StringBuffer();
  for (final int r in s.toLowerCase().runes) {
    final String ch = String.fromCharCode(r);
    final int i = from.indexOf(ch);
    b.write(i >= 0 ? to[i] : ch);
  }
  return b.toString();
}

/// "1 mensaje por enviar" / "3 mensajes por enviar".
String plural(int n, String one, String many) => n == 1 ? '1 $one' : '$n $many';
