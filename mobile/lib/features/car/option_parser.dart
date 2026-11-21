import '../../core/models/ask_prompt.dart';

/// Spoken answers to a permission menu. Pure Dart.
///
/// Safety rule: when in doubt, return null (the caller asks again). A
/// spoken answer must be short and unambiguous to count; anything that
/// sounds like "no", "wait" or "stop" can never pick an approving option.

/// Cardinals: count when they are the whole answer ("dos") or follow a
/// marker ("opción dos", "la dos"). `una` / `un` are left out on purpose:
/// "espera una cosa", "a la una".
const Map<String, int> _cardinals = <String, int>{
  'uno': 1, 'one': 1, //
  'dos': 2, 'two': 2,
  'tres': 3, 'three': 3,
  'cuatro': 4, 'four': 4,
  'cinco': 5, 'five': 5,
  'seis': 6, 'six': 6,
  'siete': 7, 'seven': 7,
  'ocho': 8, 'eight': 8,
  'nueve': 9, 'nine': 9,
};

/// Ordinals: only after a marker ("la segunda", "the second"). Never on
/// their own: "primero revisa…", "dame un segundo".
const Map<String, int> _ordinals = <String, int>{
  'primero': 1, 'primera': 1, 'first': 1, //
  'segundo': 2, 'segunda': 2, 'second': 2,
  'tercero': 3, 'tercera': 3, 'third': 3,
  'cuarto': 4, 'cuarta': 4, 'fourth': 4,
  'quinto': 5, 'quinta': 5, 'fifth': 5,
  'sexto': 6, 'sexta': 6, 'sixth': 6,
  'septimo': 7, 'septima': 7, 'seventh': 7,
  'octavo': 8, 'octava': 8, 'eighth': 8,
  'noveno': 9, 'novena': 9, 'ninth': 9,
};

/// Words that introduce a number: "opción dos", "la dos", "número tres".
const Set<String> _markers = <String>{'opcion', 'option', 'numero', 'number', 'la', 'el', 'the'};

/// Words that carry no meaning in an answer ("pues la dos, por favor").
const Set<String> _filler = <String>{
  'pues', 'bueno', 'eh', 'em', 'mm', 'mmm', 'este', 'a', 'ver', 'que', 'por', 'favor', 'please',
};

const Set<String> _yes = <String>{'si', 'sip', 'claro', 'dale', 'adelante', 'yes', 'yeah', 'yep', 'ok', 'okay', 'vale', 'acepto'};

/// A plain refusal: may pick the (single) "No" option.
const Set<String> _no = <String>{'no', 'nop', 'nope', 'nel', 'negativo', 'rechaza', 'rechazo', 'not', 'never', 'nunca'};

/// "Wait / stop / none": never answers anything (the caller asks again).
const Set<String> _hold = <String>{
  'espera', 'esperate', 'esperame', 'esperen', 'para', 'parale', 'alto', 'detente', 'deten', 'cancela',
  'cancelar', 'cancel', 'ninguna', 'ninguno', 'nada', 'none', 'pausa', 'wait', 'hold', 'stop', 'momento',
};

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

/// A refusing option ("No", "No, and tell Claude…").
bool labelIsNo(String label) => RegExp(r'^no\b').hasMatch(foldSpoken(label));

/// Speakable label: "Yes" → "sí", "No" → "no", otherwise as written.
String speakableLabel(String label) {
  final String f = foldSpoken(label);
  if (f == 'yes') return 'sí';
  if (f == 'no') return 'no';
  return label.trim();
}

int? _digit(String w) {
  final int? n = int.tryParse(w);
  return n != null && n >= 1 && n <= 9 && w.length == 1 ? n : null;
}

/// The number an answer names, if the WHOLE answer (filler aside) is
/// `<n>` or `<marker> <n>`. Null when there is none or anything else.
int? _wholeNumber(List<String> words) {
  final List<String> w = <String>[for (final String x in words) if (!_filler.contains(x)) x];
  if (w.length == 1) return _digit(w.single) ?? _cardinals[w.single];
  if (w.length == 2 && _markers.contains(w.first)) {
    return _digit(w.last) ?? _cardinals[w.last] ?? _ordinals[w.last];
  }
  // "la opción dos", "the option two".
  final bool article = w.length == 3 && (w[0] == 'la' || w[0] == 'the');
  if (article && (w[1] == 'opcion' || w[1] == 'option')) return _digit(w.last) ?? _cardinals[w.last];
  return null;
}

/// Picks the option meant by [heard], or null when it is ambiguous or
/// unrelated (then the caller asks again: never guess).
///
/// Understands, as the whole answer: "2", "dos", "opción dos", "la dos",
/// "número dos", "la segunda"; "sí" / "no" alone (when exactly one option
/// is a yes / no); and an option's exact label. A "no"-word anywhere can
/// only ever pick a "No" option; "espera" / "para" / "cancela" / "ninguna"
/// anywhere pick nothing.
AskOption? parseOptionChoice(String heard, List<AskOption> options) {
  final AskOption? o = _parse(heard, options);
  if (o == null) return null;
  final List<String> words = foldSpoken(heard).split(' ');
  // Last line of defence: a refusal word never approves.
  if (words.any(_no.contains) && !labelIsNo(o.label)) return null;
  return o;
}

AskOption? _parse(String heard, List<AskOption> options) {
  final String f = foldSpoken(heard);
  if (f.isEmpty || options.isEmpty) return null;
  final List<String> words = f.split(' ');
  if (words.any(_hold.contains)) return null;

  AskOption? byKey(String key) {
    for (final AskOption o in options) {
      if (o.key == key) return o;
    }
    return null;
  }

  // An option's exact label ("yes", "no").
  final List<AskOption> exact =
      options.where((AskOption o) => foldSpoken(o.label).isNotEmpty && foldSpoken(o.label) == f).toList();
  if (exact.length == 1) return exact.single;

  final bool saidNo = words.any(_no.contains);
  final bool saidYes = words.any(_yes.contains);
  if (saidNo && saidYes) return null;

  // "no, la tres": the number must be the rest of the answer.
  final List<String> rest = <String>[for (final String w in words) if (!_no.contains(w)) w];
  final int? n = _wholeNumber(rest);
  if (n != null) return byKey('$n');

  // Only yes-words (or only no-words), filler aside.
  final List<String> meaningful = <String>[for (final String w in words) if (!_filler.contains(w)) w];
  if (meaningful.isEmpty) return null;
  if (saidYes && meaningful.every(_yes.contains)) {
    final List<AskOption> match = options.where((AskOption o) => _labelIsYes(o.label)).toList();
    if (match.length == 1) return match.single;
    // Several "Yes…" options: only the bare "Yes" counts, if present.
    final List<AskOption> bare = match.where((AskOption o) {
      final String l = foldSpoken(o.label);
      return l == 'yes' || l == 'si';
    }).toList();
    return bare.length == 1 ? bare.single : null;
  }
  if (saidNo && meaningful.every(_no.contains)) {
    final List<AskOption> match = options.where((AskOption o) => labelIsNo(o.label)).toList();
    return match.length == 1 ? match.single : null;
  }
  return null;
}

/// "Opción uno: sí. Opción dos: no."
String describeOptions(List<AskOption> options) => options
    .map((AskOption o) => 'Opción ${optionWord(o.key)}: ${speakableLabel(o.label)}.')
    .join(' ');
