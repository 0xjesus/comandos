import '../../core/models/ask_prompt.dart';
import '../car/option_parser.dart';
import '../voice/text_prep.dart';

// What Android Auto reads aloud for a notice, and what a spoken reply means.
// Pure Dart; unit-tested. The car reads the notification text as written, so
// it is built to be heard: short, no code, no paths, never a secret.

/// Words the car reads for one agent reply at most (~20 s of speech).
const int kCarWords = 45;

/// Lines of a permission's detail (the command or file) read before asking.
const int kCarDetailLines = 2;

final List<(RegExp, String)> _secrets = <(RegExp, String)>[
  // Keys and tokens with a known shape.
  (RegExp(r'\bsk-[A-Za-z0-9_-]{12,}'), 'una llave'),
  (RegExp(r'\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{16,}|\bgithub_pat_[A-Za-z0-9_]{20,}'), 'un token'),
  (RegExp(r'\bxox[abprs]-[A-Za-z0-9-]{10,}'), 'un token'),
  (RegExp(r'\bAKIA[0-9A-Z]{16}\b'), 'una llave'),
  (RegExp(r'\bAIza[0-9A-Za-z_-]{30,}'), 'una llave'),
  (RegExp(r'\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}'), 'un token'),
  (RegExp(r'\bBearer\s+\S{8,}', caseSensitive: false), 'un token'),
  (RegExp(r'-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)'), 'una llave privada'),
  // `password: x`, `API_KEY=x`, `token = "x"`.
  (
    RegExp(r'\b(password|passwd|contrase(?:ñ|n)a|secret|token|api[_-]?key|apikey)\b(\s*[:=]\s*)("[^"]*"|' "'[^']*'" r'|\S+)',
        caseSensitive: false),
    r'$1 oculto',
  ),
  // People and places: e-mails, IPs, long numbers (phones, cards, accounts).
  (RegExp(r'\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b'), 'un correo'),
  (RegExp(r'\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b'), 'una IP'),
];

/// Phones, cards, accounts: 10+ digits in one run (a date or a version is not).
final RegExp _longNumber = RegExp(r'(?<![\w.])\+?\d[\d -]{8,}\d(?![\w.])');

/// Hides what nobody else in the car should hear (or see on its screen).
String redactSecrets(String text) {
  String out = text;
  for (final (RegExp re, String with_) in _secrets) {
    out = out.replaceAllMapped(re, (Match m) {
      if (!with_.contains(r'$1')) return with_;
      return with_.replaceAll(r'$1', m.group(1) ?? '');
    });
  }
  return out.replaceAllMapped(
    _longNumber,
    (Match m) => RegExp(r'\d').allMatches(m.group(0)!).length >= 10 ? 'un número' : m.group(0)!,
  );
}

/// A reply ready for the car: markdown, code, paths and secrets out, at most
/// [maxWords] words (cut at a sentence when possible).
String carText(String raw, {int maxWords = kCarWords}) {
  final String spoken = prepareForSpeech(redactSecrets(raw)).replaceAll('\n\n', ' ').trim();
  if (spoken.isEmpty) return '';
  final List<String> words = spoken.split(RegExp(r'\s+'));
  if (words.length <= maxWords) return spoken;
  final String cut = words.take(maxWords).join(' ');
  final int end = cut.lastIndexOf(RegExp(r'[.!?…]'));
  // Keep a whole sentence when it carries most of the cut.
  if (end > cut.length * 0.6) return cut.substring(0, end + 1);
  return '${cut.replaceFirst(RegExp(r'[,;:]$'), '')}…';
}

/// What the car reads for a permission prompt: the question, the command it
/// approves (so a «sí» is never blind) and the options by number.
String askCarText(AskPrompt ask) {
  final List<String> detail = <String>[
    for (final String l in ask.detail)
      if (l.trim().isNotEmpty) l.trim(),
  ];
  final String what = detail.take(kCarDetailLines).map((String l) => carText(l, maxWords: 16)).join('. ');
  final String question = carText(ask.question.isEmpty ? 'Espera tu respuesta' : ask.question, maxWords: 20);
  final String options = ask.options
      .map((AskOption o) => '${optionWord(o.key)}, ${carText(speakableLabel(o.label), maxWords: 8)}')
      .join('; ');
  final StringBuffer b = StringBuffer('Pide permiso. ');
  if (what.isNotEmpty) b.write('${_end(what)} ');
  b.write('${_end(question)} Responde con el número: $options.');
  return b.toString();
}

String _end(String s) {
  final String t = s.trim();
  return RegExp(r'[.!?…:]$').hasMatch(t) ? t : '$t.';
}

/// What to do with a reply dictated in the car.
sealed class CarReplyPlan {
  const CarReplyPlan();
}

/// Type this text to the agent (no menu is open).
class CarSend extends CarReplyPlan {
  const CarSend(this.text);
  final String text;
}

/// Answer the open permission with this option.
class CarAnswer extends CarReplyPlan {
  const CarAnswer(this.option);
  final AskOption option;
}

/// Do nothing and say why (read back to the driver in the conversation).
class CarRefuse extends CarReplyPlan {
  const CarRefuse(this.why);
  final String why;
}

/// Decides a car reply against what the pane shows *now* ([open], re-read
/// just before) and what the driver heard ([heardSig], the prompt the
/// notification read; null for a plain reply). Never guesses:
/// * a prompt open now is only ever answered by a clear option of the very
///   prompt the driver heard;
/// * a plain message never goes to a pane showing a prompt (it would pick
///   the highlighted option), nor when the prompt the driver heard is gone.
CarReplyPlan planCarReply(String heard, {AskPrompt? open, String? heardSig}) {
  final String text = heard.trim();
  if (text.isEmpty) return const CarRefuse('No escuché nada; no envié nada.');
  if (open != null) {
    if (heardSig == null || heardSig != open.signature) {
      return CarRefuse('Hay un permiso abierto que no escuchaste: ${carText(open.question, maxWords: 16)} '
          'No envié nada.');
    }
    final AskOption? o = parseOptionChoice(text, open.options);
    if (o == null) {
      return CarRefuse('No entendí la opción; no respondí. Contesta con el número: '
          '${open.options.map((AskOption x) => optionWord(x.key)).join(', ')}.');
    }
    return CarAnswer(o);
  }
  if (heardSig != null) return const CarRefuse('Ese permiso ya no está abierto; no envié nada.');
  return CarSend(text);
}
