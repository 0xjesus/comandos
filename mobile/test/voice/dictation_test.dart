import 'dart:convert';
import 'dart:io';

import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/openai_client.dart';
import 'package:comandos_mobile/features/voice/recorder.dart';
import 'package:comandos_mobile/features/voice/reply_watcher.dart';
import 'package:flutter_test/flutter_test.dart';

import 'voice_fakes.dart';

void main() {
  late Directory dir;
  late FakeRecorder rec;
  late FakeTimers clock;
  late List<Object> results; // String or OpenAiException, consumed in order
  late List<(String, String)> drafts;
  late int calls;
  late DictationController d;
  late List<Duration> waits;

  DictationController build({String? precheck}) => DictationController(
        recorder: rec,
        transcribe: (String path, Future<void> abort) async {
          calls++;
          expect(File(path).existsSync(), isTrue, reason: 'audio must exist while transcribing');
          final Object r = results.isEmpty ? 'texto' : results.removeAt(0);
          if (r is OpenAiException) throw r;
          return r as String;
        },
        dir: () async => '${dir.path}/takes',
        fallback: (String owner, String text) async => drafts.add((owner, text)),
        precheck: () async => precheck,
        now: clock.now,
        timer: clock.call,
        delay: (Duration w) async => waits.add(w),
      );

  /// Starts a dictation and writes a fake recording to its file.
  Future<Future<DictationOutcome>> record(String owner) async {
    final int before = rec.starts;
    final Future<DictationOutcome> f = d.start(owner);
    await waitFor(() => rec.starts > before && d.phase == DictationPhase.recording);
    File(rec.path!).writeAsBytesSync(<int>[1, 2, 3]);
    return f;
  }

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('dict');
    rec = FakeRecorder();
    clock = FakeTimers();
    results = <Object>[];
    drafts = <(String, String)>[];
    calls = 0;
    waits = <Duration>[];
    d = build();
  });

  tearDown(() async => dir.delete(recursive: true));

  List<String> files() => Directory('${dir.path}/takes').listSync().map((FileSystemEntity e) => e.path).toList();

  test('happy path: text goes to the attached sink, then the audio is deleted', () async {
    final List<String> got = <String>[];
    d.attach('s1', (String t) async => got.add(t));
    results.add('  hola agente  ');
    final Future<DictationOutcome> f = await record('s1');
    expect(d.phase, DictationPhase.recording);
    await d.stop();
    final DictationOutcome o = await f;
    await waitFor(() => files().isEmpty);
    expect(o, isA<DictationText>().having((DictationText t) => t.text, 'text', 'hola agente'));
    expect(got, <String>['hola agente']);
    expect(files(), isEmpty);
    expect(d.phase, DictationPhase.idle);
  });

  test('no screen for the owner: the text lands in the draft fallback', () async {
    final Future<DictationOutcome> f = await record('s2');
    await d.stop();
    await f;
    await waitFor(() => drafts.isNotEmpty);
    expect(drafts, <(String, String)>[('s2', 'texto')]);
  });

  test('network errors retry 3 times, then keep the recording for a manual retry', () async {
    results.addAll(<Object>[
      const OpenAiException(OpenAiErrorKind.network),
      const OpenAiException(OpenAiErrorKind.timeout),
      const OpenAiException(OpenAiErrorKind.server, status: 502),
    ]);
    final Future<DictationOutcome> f = await record('s1');
    await d.stop();
    final DictationOutcome o = await f;
    expect(o, isA<DictationFailed>());
    expect(calls, 3);
    expect(waits, const <Duration>[Duration(seconds: 1), Duration(seconds: 3)]);
    final PendingTake take = d.pendingFor('s1').single;
    expect(File(take.path).existsSync(), isTrue, reason: 'the recording is kept');
    expect(take.message, 'OpenAI tuvo un problema. Inténtalo de nuevo.');

    // The network is back: Reintentar transcribes the same audio.
    final List<String> got = <String>[];
    d.attach('s1', (String t) async => got.add(t));
    results.add('ya funciona');
    final DictationOutcome again = await d.retry(take.id);
    await waitFor(() => files().isEmpty);
    expect(again, isA<DictationText>());
    expect(got, <String>['ya funciona']);
    expect(d.pendingFor('s1'), isEmpty);
    expect(files(), isEmpty);
  });

  test('401 and quota are not retried; the recording is kept', () async {
    results.add(const OpenAiException(OpenAiErrorKind.unauthorized, status: 401));
    final Future<DictationOutcome> f = await record('s1');
    await d.stop();
    final DictationOutcome o = await f;
    expect(calls, 1);
    expect((o as DictationFailed).kind, OpenAiErrorKind.unauthorized);
    expect(File(o.take.path).existsSync(), isTrue);
  });

  test('a pending recording survives a restart; orphans older than 24 h are deleted', () async {
    results.addAll(<Object>[
      const OpenAiException(OpenAiErrorKind.network),
      const OpenAiException(OpenAiErrorKind.network),
      const OpenAiException(OpenAiErrorKind.network),
    ]);
    final Future<DictationOutcome> f = await record('s1');
    await d.stop();
    await f;
    final File orphan = File('${dir.path}/takes/take-1.m4a')..writeAsBytesSync(<int>[1]);
    orphan.setLastModifiedSync(DateTime.fromMillisecondsSinceEpoch(clock.nowMs - const Duration(hours: 25).inMilliseconds));
    final File young = File('${dir.path}/takes/take-2.m4a')..writeAsBytesSync(<int>[1]);
    young.setLastModifiedSync(DateTime.fromMillisecondsSinceEpoch(clock.nowMs));

    final DictationController fresh = build();
    await fresh.load();
    expect(fresh.pendingFor('s1'), hasLength(1));
    expect(fresh.pendingFor('s1').single.message, 'Grabación sin transcribir.');
    expect(orphan.existsSync(), isFalse);
    expect(young.existsSync(), isTrue);
  });

  test('empty transcription: "No se oyó nada", audio deleted', () async {
    results.add('   ');
    final Future<DictationOutcome> f = await record('s1');
    await d.stop();
    expect(await f, isA<DictationNothing>());
    expect(d.noticeFor('s1'), 'No se oyó nada.');
    expect(files(), isEmpty);
  });

  test('cancel while recording discards; mic denied explains how to enable', () async {
    final Future<DictationOutcome> f = await record('s1');
    await d.cancel();
    expect(await f, isA<DictationCancelled>());
    expect(rec.cancels, 1);
    expect(calls, 0);
    expect(files(), isEmpty);

    rec.allow = false;
    final DictationOutcome denied = await d.start('s1');
    expect(denied, isA<DictationDenied>());
    expect(d.noticeFor('s1'), contains('Permisos › Micrófono'));
  });

  test('missing key blocks before touching the mic', () async {
    d = build(precheck: 'Falta la llave de OpenAI. Agrégala en Ajustes › Voz.');
    final DictationOutcome o = await d.start('s1');
    expect(o, isA<DictationDenied>());
    expect(rec.starts, 0);
  });

  test('silence after speech auto-stops; the 3-minute cap too', () async {
    final Future<DictationOutcome> f = await record('s1');
    void level(double db) => rec.levelsCtl.add(db);
    for (int i = 0; i < 5; i++) {
      clock.nowMs += 100;
      level(-60);
    }
    for (int i = 0; i < 10; i++) {
      clock.nowMs += 100;
      level(-20); // speech
    }
    expect(d.phase, DictationPhase.recording);
    for (int i = 0; i < 26; i++) {
      clock.nowMs += 100;
      level(-60);
      await flush(2);
    }
    await settle();
    expect(await f, isA<DictationText>());

    final Future<DictationOutcome> g = await record('s1');
    await clock.elapse(const Duration(minutes: 3));
    await settle();
    expect(await g, isA<DictationText>());
  });

  test('deliver skips a given sink (car → composer) and falls back to the draft', () async {
    final List<String> composer = <String>[];
    Future<void> car(String t) async {}
    d.attach('s1', (String t) async => composer.add(t));
    d.attach('s1', car);
    await d.deliver('s1', 'hola', except: car);
    expect(composer, <String>['hola']);
    await d.deliver('nadie', 'x');
    expect(drafts.single, ('nadie', 'x'));
  });

  group('SilenceDetector', () {
    test('adapts to background noise and ends after the silence', () {
      final SilenceDetector s = SilenceDetector(silence: const Duration(milliseconds: 2500));
      int t = 0;
      bool feed(double db) => s.add(db, t += 100);
      for (int i = 0; i < 20; i++) {
        expect(feed(-35), isFalse); // a noisy car is not speech
      }
      expect(s.heardSpeech, isFalse);
      for (int i = 0; i < 10; i++) {
        feed(-12);
      }
      expect(s.heardSpeech, isTrue);
      bool stopped = false;
      for (int i = 0; i < 24; i++) {
        stopped = feed(-35);
      }
      expect(stopped, isFalse);
      expect(feed(-35), isTrue);
    });

    test('nothing heard: ends after the no-speech timeout', () {
      final SilenceDetector s = SilenceDetector(silence: const Duration(seconds: 2), noSpeech: const Duration(seconds: 3));
      expect(s.add(-70, 0), isFalse);
      expect(s.add(-70, 2900), isFalse);
      expect(s.add(-70, 3000), isTrue);
      expect(SilenceDetector.meter(-60), 0);
      expect(SilenceDetector.meter(0), 1);
    });
  });

  group('ReplyWatcher', () {
    ChatMessage u(String t) => ChatMessage(ChatRole.user, t);
    ChatMessage a(String t) => ChatMessage(ChatRole.assistant, t);
    ChatMessage tool(String t) => ChatMessage(ChatRole.tool, t);

    test('reports a completed reply once, after it settles; never partial', () {
      final FakeTimers c = FakeTimers();
      final ReplyWatcher w = ReplyWatcher(now: c.now);
      w.baseline(<ChatMessage>[u('hola'), a('viejo')]);
      // Old reply: never read.
      expect(w.observe(<ChatMessage>[u('hola'), a('viejo')], working: false), isNull);
      c.nowMs += 10000;
      expect(w.observe(<ChatMessage>[u('hola'), a('viejo')], working: false), isNull);

      // New question; the agent writes a sentence, then a tool call follows.
      List<ChatMessage> m = <ChatMessage>[u('hola'), a('viejo'), u('haz X'), a('Voy a revisar.')];
      expect(w.observe(m, working: false), isNull);
      expect(w.pending, isTrue);
      c.nowMs += 1000;
      m = <ChatMessage>[...m, tool('Bash(ls)')];
      expect(w.observe(m, working: true), isNull);
      expect(w.pending, isFalse);
      m = <ChatMessage>[...m, a('Listo, X hecho.')];
      expect(w.observe(m, working: false), isNull);
      c.nowMs += 4000;
      final ReplyRef? r = w.observe(m, working: false);
      expect(r, isNotNull);
      expect(r!.text, 'Voy a revisar.\n\nListo, X hecho.');
      c.nowMs += 10000;
      expect(w.observe(m, working: false), isNull, reason: 'never read twice');
    });

    test('the feed saying "work" holds it back; a persisted read id is honoured', () {
      final FakeTimers c = FakeTimers();
      final List<ChatMessage> m = <ChatMessage>[u('q'), a('r')];
      final String id = currentReply(m)!.id;
      final ReplyWatcher w = ReplyWatcher(now: c.now, alreadyRead: <String>{id});
      w.baseline(const <ChatMessage>[]);
      c.nowMs += 9000;
      expect(w.observe(m, working: false), isNull);
      final List<ChatMessage> m2 = <ChatMessage>[...m, u('q2'), a('r2')];
      expect(w.observe(m2, working: false, feedWorking: true), isNull);
      c.nowMs += 9000;
      expect(w.observe(m2, working: false, feedWorking: true), isNull);
      expect(w.observe(m2, working: false), isNull);
      c.nowMs += 4000;
      expect(w.observe(m2, working: false)?.text, 'r2');
    });
  });

  test('take meta is plain JSON with the owner', () async {
    final Future<DictationOutcome> f = await record('s9');
    final File meta = File(files().firstWhere((String p) => p.endsWith('.json')));
    expect((jsonDecode(meta.readAsStringSync()) as Map<String, Object?>)['owner'], 's9');
    await d.cancel();
    await f;
  });
}
