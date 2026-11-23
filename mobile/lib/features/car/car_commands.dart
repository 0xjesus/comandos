import 'option_parser.dart' show foldSpoken;

/// Spoken commands of the Modo auto "call". Pure Dart.
///
/// A command counts only when it is the WHOLE utterance (filler aside):
/// "siguiente", "el siguiente", "oye, repite". Anything longer is a
/// message for the agent ("repite el test", "sigue con el deploy").
/// Which commands apply depends on the moment (the machine decides): a
/// bare "no" or "sí" is a message in an open turn, a cancel / confirm
/// during a read-back.
enum CarCommand {
  /// Next split.
  next,

  /// Previous split.
  previous,

  /// Read again what was last read (prompt or reply).
  repeat,

  /// A bare refusal ("no").
  no,

  /// "Cancela", "no lo mandes", "olvídalo".
  cancel,

  /// "Sí", "envíalo", "dale": skip a countdown.
  send,

  /// End the call («para», «cuelga»).
  hangUp,

  /// Pause the call («espera»).
  pause,

  /// Resume a paused call («sigue», «continúa»).
  resume,

  /// Open a turn to dictate («mensaje», «responder»).
  talk,
}

const Map<CarCommand, Set<String>> _phrases = <CarCommand, Set<String>>{
  CarCommand.next: <String>{
    'siguiente', 'el siguiente', 'la siguiente', 'al siguiente', 'pasa al siguiente', 've al siguiente', //
    'siguiente split', 'el siguiente split', 'split siguiente', 'otro split', 'proximo', 'el proximo',
    'pasa', 'pasale', 'salta', 'saltalo', 'next',
  },
  CarCommand.previous: <String>{
    'anterior', 'el anterior', 'la anterior', 'al anterior', 've al anterior', 'regresa al anterior', //
    'split anterior', 'el split anterior', 'regresa', 'atras', 'previous', 'back',
  },
  CarCommand.repeat: <String>{
    'repite', 'repitelo', 'repitemelo', 'repetir', 'repite eso', 'otra vez', 'de nuevo', 'que dijo', //
    'que dijiste', 'repeat',
  },
  CarCommand.no: <String>{'no', 'nop', 'nel', 'no gracias', 'negativo'},
  CarCommand.cancel: <String>{
    'cancela', 'cancelar', 'cancelalo', 'cancela eso', 'no lo mandes', 'no lo envies', 'no mandes', //
    'no envies', 'no lo mandes todavia', 'olvidalo', 'borralo', 'cancel',
  },
  CarCommand.send: <String>{
    'si', 'envia', 'envialo', 'enviar', 'envialo ya', 'manda', 'mandalo', 'mandalo ya', 'dale', 'ya', //
    'ok', 'okay', 'send',
  },
  CarCommand.hangUp: <String>{
    'para', 'parale', 'cuelga', 'colgar', 'cuelgale', 'cuelga la llamada', 'termina', 'terminar', //
    'termina la llamada', 'adios', 'salir', 'sal', 'fin', 'alto', 'stop', 'hang up',
  },
  CarCommand.pause: <String>{
    'espera', 'esperate', 'esperame', 'pausa', 'un momento', 'un segundo', 'dame un segundo', //
    'dame un momento', 'wait',
  },
  CarCommand.resume: <String>{'sigue', 'continua', 'continuar', 'reanuda', 'seguimos', 'resume', 'continue'},
  CarCommand.talk: <String>{
    'habla', 'hablar', 'mensaje', 'un mensaje', 'responder', 'responde', 'dictar', 'dicta', 'escribe', //
    'quiero decir algo', 'te escucho',
  },
};

/// Words that may surround a command without changing it.
const Set<String> _filler = <String>{'oye', 'porfa', 'pues', 'eh', 'em', 'mm', 'bueno', 'este', 'favor'};

/// The command [heard] is, or null when it is anything else.
CarCommand? parseCarCommand(String heard) {
  final List<String> words = foldSpoken(heard).split(' ').where((String w) => w.isNotEmpty).toList();
  // "por favor" at either end.
  while (words.length > 1 && words.first == 'por' && words[1] == 'favor') {
    words.removeRange(0, 2);
  }
  while (words.length > 1 && words[words.length - 2] == 'por' && words.last == 'favor') {
    words.removeRange(words.length - 2, words.length);
  }
  while (words.isNotEmpty && _filler.contains(words.first)) {
    words.removeAt(0);
  }
  while (words.isNotEmpty && _filler.contains(words.last)) {
    words.removeLast();
  }
  if (words.isEmpty || words.length > 5) return null;
  final String said = words.join(' ');
  for (final MapEntry<CarCommand, Set<String>> e in _phrases.entries) {
    if (e.value.contains(said)) return e.key;
  }
  return null;
}
