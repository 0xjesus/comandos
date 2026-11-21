import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/openai_key_store.dart';
import 'package:comandos_mobile/features/voice/text_prep.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('prepareForSpeech', () {
    test('strips markdown markers', () {
      expect(prepareForSpeech('## Listo\n\nHice **dos** cambios en _el_ `composer`.'),
          'Listo.\n\nHice dos cambios en el composer.');
      expect(prepareForSpeech('> cita ~~vieja~~'), 'cita vieja');
    });

    test('code blocks become "bloque de código de N líneas"', () {
      const String md = 'Antes.\n\n```dart\nvoid main() {\n  print(1);\n}\n```\n\nDespués.';
      expect(prepareForSpeech(md), 'Antes.\n\nBloque de código de 3 líneas.\n\nDespués.');
      expect(prepareForSpeech('```\nuna\n```'), 'Bloque de código.');
      // An unclosed fence swallows the rest as code, not as speech.
      expect(prepareForSpeech('Mira:\n```\na\nb'), 'Mira:\n\nBloque de código de 2 líneas.');
    });

    test('links, URLs, paths and hashes are shortened or dropped', () {
      final String out = prepareForSpeech(
          'Ver [la guía](https://example.com/x) y https://example.com/a/b. '
          'Cambié lib/features/voice/voice.dart en el commit 70b056ad.');
      expect(out, contains('Ver la guía y un enlace'));
      expect(out, contains('voice.dart'));
      expect(out, isNot(contains('lib/features')));
      expect(out, isNot(contains('70b056ad')));
      expect(out, isNot(contains('https')));
    });

    test('tool-call lines and table separators are not read', () {
      const String md = '⏺ Bash(git status)\nRead(lib/a.dart)\nTodo en orden.\n\n| a | b |\n|---|---|\n| 1 | 2 |';
      expect(prepareForSpeech(md), 'Todo en orden.\n\na, b. 1, 2.');
    });

    test('list items get a pause; whitespace collapses', () {
      expect(prepareForSpeech('- uno\n- dos\n\n\n  tres   cuatro  '), 'uno. dos.\n\ntres cuatro');
    });

    test('snake_case identifiers keep their underscores', () {
      expect(prepareForSpeech('usa read_db y _privado_'), 'usa read_db y privado');
    });
  });

  group('selectForReading', () {
    final String long = List<String>.generate(6, (int i) => 'Párrafo $i con texto suficiente para ser largo de verdad, más de ochenta caracteres aquí.').join('\n\n');

    test('full and short replies are read entirely', () {
      expect(selectForReading(long, ReadMode.full), long);
      expect(selectForReading('Hola.\n\nAdiós.', ReadMode.ending), 'Hola.\n\nAdiós.');
    });

    test('ending = last paragraph (plus the previous one when it is tiny)', () {
      expect(selectForReading(long, ReadMode.ending), startsWith('Párrafo 5'));
      expect(selectForReading('$long\n\n¿Sigo?', ReadMode.ending), 'Párrafo 5 con texto suficiente para ser largo de verdad, más de ochenta caracteres aquí.\n\n¿Sigo?');
    });

    test('summary = first sentences', () {
      final String s = selectForReading(long, ReadMode.summary);
      expect(s, startsWith('Párrafo 0'));
      expect(s.length, lessThanOrEqualTo(300));
    });
  });

  group('chunkForSpeech', () {
    test('splits at sentence boundaries, short first chunk, all under the limit', () {
      final String text = List<String>.generate(60, (int i) => 'Esta es la oración número $i.').join(' ');
      final List<String> chunks = chunkForSpeech(text);
      expect(chunks.first.length, lessThanOrEqualTo(kFirstChunkChars));
      for (final String c in chunks) {
        expect(c.length, lessThanOrEqualTo(kChunkChars));
        expect(c.endsWith('.'), isTrue, reason: c);
      }
      expect(chunks.join(' '), text);
    });

    test('a huge sentence is cut at a comma or space, never lost', () {
      final String s = '${List<String>.filled(400, 'palabra').join(', ')}.';
      final List<String> chunks = chunkForSpeech(s, first: 100, rest: 500);
      expect(chunks.every((String c) => c.length <= 500), isTrue);
      expect(chunks.join(' ').replaceAll(RegExp(r'\s+'), ' '), s);
      expect(chunks.first.length, lessThanOrEqualTo(100));
    });

    test('empty text gives no chunks', () {
      expect(chunkForSpeech('   '), isEmpty);
      expect(speechChunksFor('```\ncode\n```', ReadMode.full), <String>['Bloque de código.']);
    });
  });

  test('fnv64 is stable and spreads', () {
    expect(fnv64('hola'), fnv64('hola'));
    expect(fnv64('hola'), isNot(fnv64('holb')));
    expect(fnv64('hola'), hasLength(16));
  });

  group('dictation text helpers', () {
    test('inserts at the cursor with spaces as needed', () {
      expect(insertDictation('hola mundo', 5, 5, 'gran'), ('hola gran mundo', 9));
      expect(insertDictation('hola', 4, 4, 'mundo'), ('hola mundo', 10));
      expect(insertDictation('', -1, -1, ' texto '), ('texto', 5));
      // No cursor (field never focused): append.
      expect(insertDictation('ya escrito', -1, -1, 'más'), ('ya escrito más', 14));
      // A selection is replaced.
      expect(insertDictation('borra ESTO ya', 6, 10, 'eso'), ('borra eso ya', 9));
    });

    test('appendToDraft', () {
      expect(appendToDraft('', ' hola '), 'hola');
      expect(appendToDraft('previo  ', 'nuevo'), 'previo nuevo');
    });
  });

  group('extractOpenAiKey', () {
    const String k = 'sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-abcd';
    test('trims whitespace, newlines, quotes and env prefixes', () {
      expect(extractOpenAiKey('  $k\n'), k);
      expect(extractOpenAiKey('OPENAI_API_KEY="$k"\r\n'), k);
    });
    test('rejects nothing / several keys', () {
      expect(extractOpenAiKey('hola'), isNull);
      expect(extractOpenAiKey('$k sk-otraLlaveDistinta1234567890'), isNull);
    });
  });
}
