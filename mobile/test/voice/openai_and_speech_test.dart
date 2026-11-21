import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:comandos_mobile/features/voice/openai_client.dart';
import 'package:comandos_mobile/features/voice/speech.dart';
import 'package:comandos_mobile/features/voice/text_prep.dart';
import 'package:comandos_mobile/features/voice/tts_cache.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'voice_fakes.dart';

const String fakeKey = 'sk-test-0000000000000000000000000000wxyz';

http.Response jsonRes(Object body, [int status = 200]) => http.Response.bytes(
      utf8.encode(jsonEncode(body)),
      status,
      headers: <String, String>{'content-type': 'application/json'},
    );

void main() {
  late Directory dir;

  setUp(() async => dir = await Directory.systemTemp.createTemp('voice'));
  tearDown(() async => dir.delete(recursive: true));

  group('OpenAiClient', () {
    test('transcription: multipart shape for gpt-transcribe (keywords[], languages[])', () async {
      late http.Request seen;
      final OpenAiClient c = OpenAiClient(
        key: () async => fakeKey,
        clientFactory: () => MockClient((http.Request r) async {
          seen = r;
          return jsonRes(<String, Object?>{'text': ' hola git push ', 'languages': <Object?>[]});
        }),
      );
      final File audio = File('${dir.path}/a.m4a')..writeAsBytesSync(<int>[1, 2, 3]);
      final String text =
          await c.transcribe(audio.path, const SttOptions(model: 'gpt-transcribe', language: SttLanguage.es));
      expect(text, ' hola git push ');
      expect(seen.method, 'POST');
      expect(seen.url.toString(), 'https://api.openai.com/v1/audio/transcriptions');
      expect(seen.headers['Authorization'], 'Bearer $fakeKey');
      expect(seen.headers['Content-Type'], startsWith('multipart/form-data; boundary='));
      final String body = latin1.decode(seen.bodyBytes);
      expect(body, contains('name="model"\r\n\r\ngpt-transcribe\r\n'));
      expect(body, contains('name="file"; filename="dictado.m4a"\r\ncontent-type: audio/mp4'));
      expect(body, contains('name="languages[]"\r\n\r\nes\r\n'));
      expect(body, isNot(contains('name="language"')));
      expect('name="keywords[]"'.allMatches(body).length, kSttKeywords.length);
      expect(body, contains('\r\n\r\ntmux\r\n'));
      expect(body, contains('name="prompt"'));
    });

    test('transcription: older models get language + vocabulary in the prompt', () async {
      late http.Request seen;
      final OpenAiClient c = OpenAiClient(
        key: () async => fakeKey,
        clientFactory: () => MockClient((http.Request r) async {
          seen = r;
          return jsonRes(<String, Object?>{'text': 'ok'});
        }),
      );
      final File audio = File('${dir.path}/a.m4a')..writeAsBytesSync(<int>[1]);
      await c.transcribe(audio.path, const SttOptions(model: 'whisper-1', language: SttLanguage.en));
      final String body = utf8.decode(seen.bodyBytes, allowMalformed: true);
      expect(body, contains('name="language"\r\n\r\nen\r\n'));
      expect(body, isNot(contains('keywords[]')));
      expect(body, contains('Terms: git, commit'));
      // Automatic language: no language field at all.
      await c.transcribe(audio.path, const SttOptions(model: 'whisper-1', language: SttLanguage.auto));
      expect(utf8.decode(seen.bodyBytes, allowMalformed: true), isNot(contains('name="language"')));
    });

    test('speech: JSON shape', () async {
      late http.Request seen;
      final OpenAiClient c = OpenAiClient(
        key: () async => fakeKey,
        clientFactory: () => MockClient((http.Request r) async {
          seen = r;
          return http.Response.bytes(<int>[9, 9, 9], 200, headers: <String, String>{'content-type': 'audio/mpeg'});
        }),
      );
      final Uint8List audio = await c.speech('Hola', const TtsOptions(model: 'gpt-4o-mini-tts', voice: 'marin'));
      expect(audio, <int>[9, 9, 9]);
      expect(seen.url.toString(), 'https://api.openai.com/v1/audio/speech');
      final Map<String, Object?> j = jsonDecode(seen.body) as Map<String, Object?>;
      expect(j['model'], 'gpt-4o-mini-tts');
      expect(j['input'], 'Hola');
      expect(j['voice'], 'marin');
      expect(j['response_format'], 'mp3');
      expect(j['instructions'], isA<String>());
      // tts-1 does not take instructions.
      await c.speech('Hola', const TtsOptions(model: 'tts-1', voice: 'nova'));
      expect((jsonDecode(seen.body) as Map<String, Object?>).containsKey('instructions'), isFalse);
    });

    test('key check hits GET /v1/models/{model} with the candidate key', () async {
      late http.Request seen;
      final OpenAiClient c = OpenAiClient(
        key: () async => null,
        clientFactory: () => MockClient((http.Request r) async {
          seen = r;
          return jsonRes(<String, Object?>{'id': 'gpt-transcribe'});
        }),
      );
      await c.check('gpt-transcribe', candidate: fakeKey);
      expect(seen.method, 'GET');
      expect(seen.url.toString(), 'https://api.openai.com/v1/models/gpt-transcribe');
      expect(seen.headers['Authorization'], 'Bearer $fakeKey');
    });

    test('error mapping; messages never contain the key or server text', () async {
      Future<OpenAiException> failWith(int status, [Object? body]) async {
        final OpenAiClient c = OpenAiClient(
          key: () async => fakeKey,
          clientFactory: () => MockClient((http.Request r) async => jsonRes(
              body ?? <String, Object?>{
                'error': <String, Object?>{'message': 'Incorrect API key provided: $fakeKey', 'code': 'x'},
              },
              status)),
        );
        try {
          await c.speech('x', const TtsOptions(model: 'gpt-4o-mini-tts', voice: 'marin'));
        } on OpenAiException catch (e) {
          return e;
        }
        throw StateError('no exception');
      }

      final Map<int, OpenAiErrorKind> cases = <int, OpenAiErrorKind>{
        401: OpenAiErrorKind.unauthorized,
        403: OpenAiErrorKind.forbidden,
        404: OpenAiErrorKind.notFound,
        400: OpenAiErrorKind.badRequest,
        413: OpenAiErrorKind.tooLarge,
        429: OpenAiErrorKind.rateLimited,
        500: OpenAiErrorKind.server,
        503: OpenAiErrorKind.server,
      };
      for (final MapEntry<int, OpenAiErrorKind> e in cases.entries) {
        final OpenAiException x = await failWith(e.key);
        expect(x.kind, e.value, reason: '${e.key}');
        expect(x.userMessage, isNot(contains('sk-')));
        expect(x.toString(), isNot(contains('sk-')));
      }
      final OpenAiException quota = await failWith(429, <String, Object?>{
        'error': <String, Object?>{'type': 'insufficient_quota', 'code': 'insufficient_quota'},
      });
      expect(quota.kind, OpenAiErrorKind.quota);
      expect(quota.retryable, isFalse);
      expect((await failWith(429)).retryable, isTrue);
      expect((await failWith(503)).retryable, isTrue);
      expect((await failWith(401)).retryable, isFalse);
    });

    test('network error, missing key and abort', () async {
      final OpenAiClient net = OpenAiClient(
        key: () async => fakeKey,
        clientFactory: () => MockClient((http.Request r) async => throw const SocketException('down')),
      );
      await expectLater(net.speech('x', const TtsOptions(model: 'tts-1', voice: 'nova')),
          throwsA(isA<OpenAiException>().having((OpenAiException e) => e.kind, 'kind', OpenAiErrorKind.network)));

      final OpenAiClient none = OpenAiClient(key: () async => null);
      await expectLater(none.speech('x', const TtsOptions(model: 'tts-1', voice: 'nova')),
          throwsA(isA<OpenAiException>().having((OpenAiException e) => e.kind, 'kind', OpenAiErrorKind.missingKey)));
    });
  });

  group('TtsCache', () {
    test('LRU eviction keeps the total under the cap', () async {
      final FakeTimers clock = FakeTimers();
      final TtsCache cache = TtsCache(() async => '${dir.path}/tts', maxBytes: 250, now: clock.now);
      await cache.put('a', List<int>.filled(100, 1));
      clock.nowMs += 10;
      await cache.put('b', List<int>.filled(100, 2));
      clock.nowMs += 10;
      expect(await cache.get('a'), isNotNull); // a is now the most recent
      clock.nowMs += 10;
      await cache.put('c', List<int>.filled(100, 3)); // over 250: evict LRU = b
      expect(await cache.get('b'), isNull);
      expect(await cache.get('a'), isNotNull);
      expect(await cache.get('c'), isNotNull);
      expect(cache.totalBytes, 200);
      expect(File('${dir.path}/tts/b${TtsCache.ext}').existsSync(), isFalse);
    });

    test('rebuilds its index from disk and drops temp files', () async {
      final TtsCache first = TtsCache(() async => '${dir.path}/tts');
      await first.put('k', <int>[1, 2, 3]);
      File('${dir.path}/tts/x.mp3.tmp').writeAsBytesSync(<int>[0]);
      final TtsCache again = TtsCache(() async => '${dir.path}/tts');
      expect((await again.get('k'))!.readAsBytesSync(), <int>[1, 2, 3]);
      expect(File('${dir.path}/tts/x.mp3.tmp').existsSync(), isFalse);
    });
  });

  group('SpeechService', () {
    late FakeAudioOut audio;
    late List<String> synthesized;
    late Map<String, Completer<Uint8List>> pending;
    late SpeechService speech;
    late FakeTimers timers;

    setUp(() {
      audio = FakeAudioOut()..autoComplete = false;
      synthesized = <String>[];
      pending = <String, Completer<Uint8List>>{};
      timers = FakeTimers();
      speech = SpeechService(
        synth: (String text, Future<void> abort) {
          synthesized.add(text);
          final Completer<Uint8List> c = pending[text] = Completer<Uint8List>();
          abort.then((_) {
            if (!c.isCompleted) c.completeError(const OpenAiException(OpenAiErrorKind.aborted));
          });
          return c.future;
        },
        cache: TtsCache(() async => '${dir.path}/tts'),
        voiceSignature: () => 'sig',
        audio: audio,
        settings: () => const VoiceSettings(),
        timer: timers.call,
      );
    });

    test('first chunk at once, next prefetched while playing, played in order', () async {
      final Future<SpeakResult> done = speech.speakChunks(<String>['uno.', 'dos.', 'tres.']);
      await waitFor(() => synthesized.isNotEmpty);
      await settle();
      expect(synthesized, <String>['uno.']); // only the first before any audio
      expect(speech.speaking, isTrue);
      pending['uno.']!.complete(Uint8List.fromList(<int>[1]));
      await waitFor(() => audio.played.length == 1 && synthesized.length == 2);
      expect(synthesized, <String>['uno.', 'dos.']); // prefetch while "uno" plays
      pending['dos.']!.complete(Uint8List.fromList(<int>[2]));
      await settle();
      expect(audio.played, hasLength(1), reason: 'waits for "uno" to end');
      audio.finish();
      await waitFor(() => audio.played.length == 2 && synthesized.length == 3);
      pending['tres.']!.complete(Uint8List.fromList(<int>[3]));
      await settle();
      audio.finish();
      await waitFor(() => audio.played.length == 3);
      audio.finish();
      expect(await done, SpeakResult.completed);
      expect(speech.speaking, isFalse);
      expect(audio.releases, 1); // audio focus given back
    });

    test('stop() cancels pending requests and playback at once', () async {
      final Future<SpeakResult> done = speech.speakChunks(<String>['a.', 'b.']);
      await waitFor(() => pending.containsKey('a.'));
      pending['a.']!.complete(Uint8List.fromList(<int>[1]));
      await waitFor(() => audio.current != null && pending.containsKey('b.'));
      await speech.stop();
      expect(await done, SpeakResult.stopped);
      expect(audio.stops, greaterThan(0));
      await expectLater(pending['b.']!.future, throwsA(isA<OpenAiException>()));
      expect(speech.speaking, isFalse);
    });

    test('replay comes from the cache (no second request)', () async {
      audio.autoComplete = true;
      final Future<SpeakResult> first = speech.speakChunks(<String>['hola.']);
      await waitFor(() => pending.isNotEmpty);
      pending['hola.']!.complete(Uint8List.fromList(<int>[1, 2]));
      expect(await first, SpeakResult.completed);
      expect(await speech.speakChunks(<String>['hola.']), SpeakResult.completed);
      expect(synthesized, <String>['hola.']);
      expect(audio.played, hasLength(2));
    });

    test('a failure reports failed with the error; the player is freed when idle', () async {
      final Future<SpeakResult> done = speech.speakChunks(<String>['x.']);
      await waitFor(() => pending.isNotEmpty);
      pending['x.']!.completeError(const OpenAiException(OpenAiErrorKind.unauthorized, status: 401));
      expect(await done, SpeakResult.failed);
      expect(speech.lastError!.kind, OpenAiErrorKind.unauthorized);
      await timers.elapse(const Duration(seconds: 31));
      expect(audio.disposes, 1);
    });

    test('speakReply applies text preparation and the read mode', () async {
      audio.autoComplete = true;
      final Future<SpeakResult> done = speech.speakReply('**Hecho.** Ver `lib/a/b/c.dart`.', mode: ReadMode.full);
      await waitFor(() => pending.isNotEmpty);
      expect(synthesized, <String>['Hecho. Ver c.dart.']);
      pending.values.single.complete(Uint8List(1));
      expect(await done, SpeakResult.completed);
      expect(speechChunksFor('', ReadMode.full), isEmpty);
    });
  });
}
