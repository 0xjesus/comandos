import '../../core/models/ask_prompt.dart';

/// Spoken answers to a permission menu. Pure Dart.

const Map<String, int> _numbers = <String, int>{
  'uno': 1, 'una': 1, 'primero': 1, 'primera': 1, 'one': 1, 'first': 1, //
  'dos': 2, 'segundo': 2, 'segunda': 2, 'two': 2, 'second': 2,
  'tres': 3, 'tercero': 3, 'tercera': 3, 'three': 3, 'third': 3,
  'cuatro': 4, 'cuarto': 4, 'cuarta': 4, 'four': 4, 'fourth': 4,
  'cinco': 5, 'quinto': 5, 'quinta': 5, 'five': 5, 'fifth': 5,
  'seis': 6, 'sexto': 6, 'sexta': 6, 'six': 6,
  'siete': 7, 'septimo': 7, 'septima': 7, 'seven': 7,
  'ocho': 8, 'octavo': 8, 'octava': 8, 'eight': 8,
  'nueve': 9, 'noveno': 9, 'novena': 9, 'nine': 9,
};

const Set<String> _yes = <String>{'si', 'sip', 'claro', 'dale', 'adelante', 'yes', 'ok', 'okay', 'vale', 'acepto'};
const Set<String> _no = <String>{'no', 'nop', 'nel', 'rechaza', 'rechazo', 'cancela'};

const List<String> _words = <String>['', 'uno', 'dos', 'tres', 'cuatro', 'cinco', 'seis', 'siete', 'ocho', 'nueve'];

/// "uno".."nueve" for option keys.
String optionWord(String key) {
  final int? n = int.tryParse(key);
  return n != null && n > 0 && n < _words.length ? _words[n] : key;
}

/// Lowercase, no accents, no punctuation.
String foldSpoken(String s) {
  const String from = 'áàäâéèëêíìïîóòöôúùüûñ';
  const String to = 'aaaaeeeeiiiioooouuuun';
  final StringBuffer b = StringBuffer();
  for (final int c in s.toLowerCase().runes) {
    final String ch = String.fromCharCode(c);
    final int i = from.indexOf(ch);
    b.write(i >= 0 ? to[i] : ch);
  }
  return b.toString().replaceAll(RegExp(r'[^a-z0-9 ]'), ' ').replaceAll(RegExp(r'\s+'), ' ').trim();
}

bool _labelIsYes(String label) => RegExp(r'^(yes|si)\b').hasMatch(foldSpoken(label));
bool _labelIsNo(String label) => RegExp(r'^no\b').hasMatch(foldSpoken(label));

/// Speakable label: "Yes" → "sí", "No" → "no", otherwise as written.
String speakableLabel(String label) {
  final String f = foldSpoken(label);
  if (f == 'yes') return 'sí';
  if (f == 'no') return 'no';
  return label.trim();
}

/// Picks the option meant by [heard], or null when it is ambiguous or
/// unrelated (then the caller asks again: never guess).
///
/// Understands "2", "dos", "opción dos", "la segunda", "sí" / "no" (only in
/// a short answer and when exactly one option is a yes / no), and an
/// option's exact label.
AskOption? parseOptionChoice(String heard, List<AskOption> options) {
  final String f = foldSpoken(heard);
  if (f.isEmpty || options.isEmpty) return null;
  final List<String> words = f.split(' ');
  final Set<String> keys = <String>{};

  for (final String w in words) {
    final int? n = int.tryParse(w) ?? _numbers[w];
    if (n != null) keys.add('$n');
  }
  // "sí" / "no" only count in a short answer: "pues no sé, tal vez" is not a "no".
  final bool short = words.length <= 3;
  final bool saidYes = short && words.any(_yes.contains);
  final bool saidNo = short && words.any(_no.contains);
  if (saidYes != saidNo) {
    final List<AskOption> match =
        options.where((AskOption o) => saidYes ? _labelIsYes(o.label) : _labelIsNo(o.label)).toList();
    if (match.length == 1) {
      keys.add(match.single.key);
    } else if (match.length > 1) {
      // Several "Yes…" options: only the bare "Yes" counts, if present.
      final List<AskOption> bare =
          match.where((AskOption o) => foldSpoken(o.label) == (saidYes ? 'yes' : 'no') || foldSpoken(o.label) == 'si').toList();
      if (bare.length == 1 && words.length <= 2) {
        keys.add(bare.single.key);
      } else if (keys.isEmpty) {
        return null;
      }
    }
  } else if (saidYes && saidNo) {
    return null;
  }
  if (keys.isEmpty) {
    for (final AskOption o in options) {
      final String l = foldSpoken(o.label);
      if (l.isNotEmpty && f == l) keys.add(o.key);
    }
  }
  if (keys.length != 1) return null;
  final String key = keys.single;
  for (final AskOption o in options) {
    if (o.key == key) return o;
  }
  return null;
}

/// "Opción uno: sí. Opción dos: no."
String describeOptions(List<AskOption> options) => options
    .map((AskOption o) => 'Opción ${optionWord(o.key)}: ${speakableLabel(o.label)}.')
    .join(' ');
