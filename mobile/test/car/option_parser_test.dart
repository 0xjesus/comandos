import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/features/car/option_parser.dart';
import 'package:flutter_test/flutter_test.dart';

/// Claude Code's usual permission menu.
const List<AskOption> three = <AskOption>[
  AskOption(key: '1', label: 'Yes'),
  AskOption(key: '2', label: "Yes, and don't ask again for git"),
  AskOption(key: '3', label: 'No, and tell Claude what to do differently'),
];

const List<AskOption> two = <AskOption>[
  AskOption(key: '1', label: 'Yes'),
  AskOption(key: '2', label: 'No'),
];

void main() {
  // (heard, expected key or null) against [three]. null = ask again.
  const List<(String, String?)> table = <(String, String?)>[
    // The review's examples: a "no" or a "wait" never approves.
    ('No, espera una cosa', null),
    ('dame un segundo', null),
    ('espera', null),
    ('espérame tantito', null),
    ('para', null),
    ('alto', null),
    ('detente', null),
    ('cancela', null),
    ('ninguna', null),
    ('ninguna de esas', null),
    ('no, primero revisa los logs', null),
    ('primero revisa el archivo', null),
    ('la segunda vez que pasa', null),
    ('un momento', null),
    ('una', null),
    ('a la una', null),
    ('segunda', null),
    ('primero', null),
    ('uno o dos', null),
    ('dos tres', null),
    ('la uno, no, la dos', null),
    ('no la dos', null),
    ('la dos no', null),
    ('nel, la uno', null),
    ('sí, la dos', null),
    ('sí no', null),
    ('no sí', null),
    ('pues no sé, tal vez', null),
    ('creo que la opción dos', null),
    ('cinco', null),
    ('mándalo a producción', null),
    ('', null),
    ('wait', null),
    ('hold on', null),
    ('stop', null),
    ('no, wait', null),
    ('give me a second', null),
    ('one second', null),
    ('not now', null),
    ('yes but first check the logs', null),
    ('none of them', null),
    // Unambiguous answers.
    ('2', '2'),
    ('Dos.', '2'),
    ('uno', '1'),
    ('three', '3'),
    ('opción uno', '1'),
    ('la opción dos', '2'),
    ('número tres', '3'),
    ('la tercera', '3'),
    ('la dos, por favor', '2'),
    ('pues la uno', '1'),
    ('option two', '2'),
    ('the third', '3'),
    ('Sí', '1'),
    ('claro que sí', '1'),
    ('yes', '1'),
    ('no', '3'),
    ('No.', '3'),
    ('nel', '3'),
    ('nope', '3'),
    ('no, no', '3'),
    ('no, la tres', '3'),
  ];

  test('table: ${table.length} phrases against [1 Yes, 2 Yes-don\'t-ask-again, 3 No]', () {
    final List<String> wrong = <String>[];
    for (final (String heard, String? want) in table) {
      final String? got = parseOptionChoice(heard, three)?.key;
      if (got != want) wrong.add('«$heard» → $got (want $want)');
    }
    expect(wrong, isEmpty);
    expect(table.length, greaterThanOrEqualTo(40));
  });

  test('a no-word anywhere never resolves to an approving option', () {
    for (final String heard in <String>[
      'no, la uno',
      'no uno',
      'nel, opción dos',
      'no, sí',
      'never, option one',
      'no yes',
    ]) {
      final AskOption? o = parseOptionChoice(heard, three);
      expect(o == null || labelIsNo(o.label), isTrue, reason: heard);
    }
    // Even an approving option whose label itself has a "no" in it.
    const List<AskOption> odd = <AskOption>[
      AskOption(key: '1', label: 'Allow, no limits'),
      AskOption(key: '2', label: 'Deny'),
    ];
    expect(parseOptionChoice('allow no limits', odd), isNull);
  });

  test('two options: sí / no / dale / ok', () {
    expect(parseOptionChoice('Sí', two)!.key, '1');
    expect(parseOptionChoice('dale', two)!.key, '1');
    expect(parseOptionChoice('ok', two)!.key, '1');
    expect(parseOptionChoice('no', two)!.key, '2');
    expect(parseOptionChoice('espera', two), isNull);
    expect(parseOptionChoice('dame un segundo', two), isNull);
    expect(parseOptionChoice('la primera', two)!.key, '1');
  });

  test('describeOptions', () {
    expect(describeOptions(three), startsWith('Opción uno: sí. Opción dos: Yes, and'));
  });
}
