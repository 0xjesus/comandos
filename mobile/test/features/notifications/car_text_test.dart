import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/features/notifications/car_text.dart';
import 'package:flutter_test/flutter_test.dart';

const AskPrompt _rm = AskPrompt(
  question: 'Do you want to proceed?',
  detail: <String>['Bash command', '', '  rm -rf build', 'Borra la carpeta de compilación'],
  options: <AskOption>[
    AskOption(key: '1', label: 'Yes', highlighted: true),
    AskOption(key: '2', label: "Yes, and don't ask again"),
    AskOption(key: '3', label: 'No, and tell Claude what to do differently'),
  ],
);

void main() {
  test('secrets, people and machines never reach the car', () {
    final String out = redactSecrets(
      'Usé sk-proj-AbCdEf1234567890xyz y ghp_ABCDEFGHIJKLMNOPQRSTUVWX con API_KEY=hunter2. '
      'Escribe a ana.lopez@example.com o al +52 222 123 4567 desde 192.168.1.20:8080. '
      'Authorization: Bearer abcdefghijklmnop',
    );
    for (final String leaked in <String>[
      'sk-proj', 'ghp_', 'hunter2', 'ana.lopez', '222 123', '192.168', 'abcdefghijklmnop',
    ]) {
      expect(out, isNot(contains(leaked)), reason: leaked);
    }
    expect(out, contains('API_KEY oculto'));
    expect(redactSecrets('Listo el 2026-11-24, versión 2.0.6, 3 tests.'), 'Listo el 2026-11-24, versión 2.0.6, 3 tests.',
        reason: 'dates, versions and small numbers stay');
  });

  test('a reply is short, without code or paths', () {
    final String long = List<String>.filled(30, 'Arreglé el test del módulo.').join(' ');
    final String out = carText('**Listo.**\n\n```dart\nvoid main() {}\nprint(1);\n```\n\nCambié /srv/demo/lib/app.dart. $long');
    expect(out, isNot(contains('void main')));
    expect(out, isNot(contains('/srv/demo')));
    expect(out.split(' ').length, lessThanOrEqualTo(kCarWords));
    expect(out, startsWith('Listo.'));
    expect(out, endsWith('.'), reason: 'cut at a sentence when it can');
  });

  test('a permission reads the command it approves and the options by number', () {
    final String out = askCarText(_rm);
    expect(out, startsWith('Pide permiso.'));
    expect(out, contains('Bash command'));
    expect(out, contains('rm -rf build'), reason: 'never approve blind');
    expect(out, contains('Do you want to proceed?'));
    expect(out, contains('uno, sí'));
    expect(out, contains("dos, Yes, and don't ask again"));
  });

  test('a car reply only answers the very prompt the driver heard', () {
    expect(planCarReply('uno', open: _rm, heardSig: _rm.signature), isA<CarAnswer>());
    expect((planCarReply('la dos', open: _rm, heardSig: _rm.signature) as CarAnswer).option.key, '2');
    expect(planCarReply('no espera', open: _rm, heardSig: _rm.signature), isA<CarRefuse>(),
        reason: 'a hold word never approves');
    expect(planCarReply('arregla el test', open: _rm, heardSig: _rm.signature), isA<CarRefuse>());
    expect(planCarReply('uno', open: _rm), isA<CarRefuse>(), reason: 'a prompt the driver never heard');
    const AskPrompt other = AskPrompt(question: 'Edit main.rs?', options: <AskOption>[
      AskOption(key: '1', label: 'Yes'),
      AskOption(key: '2', label: 'No'),
    ]);
    expect(planCarReply('uno', open: other, heardSig: _rm.signature), isA<CarRefuse>(),
        reason: 'the prompt changed since it was read');
    expect(planCarReply('uno', heardSig: _rm.signature), isA<CarRefuse>(), reason: 'it was answered meanwhile');
    expect((planCarReply('  arregla el test  ') as CarSend).text, 'arregla el test');
    expect(planCarReply('   '), isA<CarRefuse>());
  });
}
