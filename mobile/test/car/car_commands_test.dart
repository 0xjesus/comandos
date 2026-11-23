import 'package:comandos_mobile/features/car/car_commands.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('whole-utterance commands, with filler and punctuation', () {
    const Map<String, CarCommand> table = <String, CarCommand>{
      'Siguiente.': CarCommand.next,
      'oye, el siguiente por favor': CarCommand.next,
      'Próximo': CarCommand.next,
      'anterior': CarCommand.previous,
      'Regresa': CarCommand.previous,
      'repite': CarCommand.repeat,
      'Repítelo': CarCommand.repeat,
      'otra vez': CarCommand.repeat,
      'No.': CarCommand.no,
      'cancela': CarCommand.cancel,
      'No lo mandes': CarCommand.cancel,
      'olvídalo': CarCommand.cancel,
      'Sí': CarCommand.send,
      'envíalo': CarCommand.send,
      'mándalo ya': CarCommand.send,
      'Para': CarCommand.hangUp,
      'cuelga': CarCommand.hangUp,
      'termina la llamada': CarCommand.hangUp,
      'Espera': CarCommand.pause,
      'dame un segundo': CarCommand.pause,
      'sigue': CarCommand.resume,
      'continúa': CarCommand.resume,
      'mensaje': CarCommand.talk,
      'responder': CarCommand.talk,
    };
    table.forEach((String said, CarCommand want) => expect(parseCarCommand(said), want, reason: said));
  });

  test('anything longer is a message, never a command', () {
    for (final String said in <String>[
      'sigue con el deploy',
      'repite el test de pagos',
      'no, primero revisa los logs',
      'para el servidor de staging',
      'siguiente paso: haz el commit',
      'espera a que termine el build y luego despliega',
      '',
    ]) {
      expect(parseCarCommand(said), isNull, reason: said);
    }
  });
}
