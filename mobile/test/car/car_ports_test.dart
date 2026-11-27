import 'dart:async';
import 'dart:io';

import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_item.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/outbox/send_result.dart';
import 'package:comandos_mobile/features/car/car_mic.dart';
import 'package:comandos_mobile/features/car/car_ports.dart';
import 'package:comandos_mobile/features/chat/chat_controller.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/openai_client.dart';
import 'package:comandos_mobile/features/voice/text_prep.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../core/outbox_test.dart' show FakeTransport;
import '../voice/voice_fakes.dart';
import 'fake_ports.dart' show FakeCarMic;

class _NoTimer implements Timer {
  @override
  void cancel() {}
  @override
  bool get isActive => false;
  @override
  int get tick => 0;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory dir;
  late FakeTransport transport;
  late Outbox outbox;
  late ChatController chat;
  late FakeRecorder rec;
  late FakeAudioOut audio;
  late List<String> heard; // transcriptions, in order
  late List<(String, String)> drafts;
  late VoiceServices voice;
  late LiveCarPorts ports;
  late List<String> requests;
  late List<String> openai; // requests to OpenAI
  late FakeDeviceSpeech tts;
  late FakeDeviceListener stt;
  OpenAiException? sttError; // the next transcriptions fail with it

  VoiceServices voiceWith({String? key}) => testVoiceServices(
        dir: dir.path,
        key: key,
        recorder: rec,
        audio: audio,
        client: MockClient((http.Request r) async {
          openai.add(r.url.path);
          return http.Response('{}', 500);
        }),
        transcribe: (String path, Future<void> abort) async {
          openai.add('/v1/audio/transcriptions');
          final OpenAiException? e = sttError;
          if (e != null) throw e;
          return heard.removeAt(0);
        },
        fallback: (String owner, String text) async => drafts.add((owner, text)),
      );

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('car_ports');
    transport = FakeTransport();
    outbox = Outbox(
      store: MemoryOutboxStore(),
      transport: transport,
      timer: (Duration d, void Function() cb) => _NoTimer(),
    );
    await outbox.load();
    outbox.setHost('equipo.ejemplo.ts.net');
    requests = <String>[];
    final MemoryCredentialStore creds = MemoryCredentialStore()
      ..value = const PairedServer(host: 'equipo.ejemplo.ts.net', token: 'TOKEN');
    final Connection connection = Connection(creds,
        apiFactory: (PairedServer s) => ComandosApi(s, client: MockClient((http.Request r) async {
              requests.add(r.url.path);
              return http.Response('{"ok":true}', 200);
            })));
    await connection.load();
    // The chat reads pane %7 (the feed may name another one).
    chat = ChatController(session: 's1', pane: '%7', connection: connection, outbox: outbox);
    rec = FakeRecorder();
    audio = FakeAudioOut();
    heard = <String>[];
    drafts = <(String, String)>[];
    openai = <String>[];
    tts = FakeDeviceSpeech();
    stt = FakeDeviceListener();
    sttError = null;
    // No key: these tests dictate with the phone's recognizer.
    voice = voiceWith(key: null);
    ports = LiveCarPorts(voice: voice, outbox: outbox, chat: () => chat, tts: tts, stt: stt);
  });

  tearDown(() async {
    chat.dispose();
    outbox.dispose();
    await dir.delete(recursive: true);
  });

  test('5: a message goes to the pane the chat reads, not the feed\'s', () async {
    final SendOutcome r = await ports.send('s1', '%1', 'hola');
    expect(r.kind, SendKind.delivered);
    expect(outbox.items.single.pane, '%7');
  });

  test('7: behind a failed message it reports the block, not "sin conexión"', () async {
    transport.sendResults.add(const Rejected(FailCode.invalid));
    await outbox.enqueue(session: 's1', text: 'primero');
    await outbox.pump();
    expect(outbox.items.single.state, OutboxState.failed);
    final SendOutcome r = await ports.send('s1', null, 'segundo');
    expect(r.kind, SendKind.blocked);
  });

  test('15: a message refused for an open prompt is reported as waiting for it', () async {
    transport.sendResults.add(const PromptOpen());
    final SendOutcome r = await ports.send('s1', null, 'hola');
    expect(r.kind, SendKind.waitingPrompt);
    expect(outbox.items.single.state, OutboxState.queued);
  });

  test('9: a voice answer releases the dictation: no draft, nothing recorded', () async {
    stt.answers.add('dos');
    final DictationOutcome o = await ports.listen('s1', onTranscribing: () {});
    expect((o as DictationText).text, 'dos');
    ports.releaseDictation();
    ports.dispose();
    await settle();
    expect(drafts, isEmpty);
    expect(rec.starts, 0, reason: 'the phone\'s recognizer: no recording, no OpenAI');
  });

  test('9: a second dictation never loses the first text: it becomes the draft', () async {
    stt.answers.addAll(<String>['primer texto', 'segundo']);
    await ports.listen('s1', onTranscribing: () {});
    // No send / save / release happened (a bug path): the next dictation
    // keeps the first text as a draft.
    expect(await ports.listen('s1', onTranscribing: () {}), isA<DictationText>());
    expect(drafts, <(String, String)>[('s1', 'primer texto')]);
    ports.releaseDictation();
  });

  test('3: an answer for another prompt, pane or session is refused without a request', () async {
    expect(await ports.answer('s1', '1', pane: '%7', promptSig: 'otra pregunta'),
        'La pregunta cambió; no respondí.');
    expect(await ports.answer('s2', '1', pane: '%7', promptSig: 'x'), 'La sesión cambió; no respondí.');
    expect(requests, isEmpty);
  });

  group('dictation: like the chat (OpenAI) with a key, the phone without one', () {
    Future<LiveCarPorts> portsWith({String? key}) async =>
        LiveCarPorts(voice: voiceWith(key: key), outbox: outbox, chat: () => chat, tts: tts, stt: stt);

    test('without a key everything is on the phone: nothing goes to OpenAI', () async {
      final LiveCarPorts p = await portsWith(key: null);
      stt.answers.addAll(<String>['revisa el deploy', 'siguiente']);
      final DictationOutcome o = await p.listen('s1', onTranscribing: () {});
      expect((o as DictationText).text, 'revisa el deploy');
      expect(rec.starts, 0, reason: 'no recording for OpenAI');
      expect(await p.hear(const Duration(seconds: 3)), 'siguiente');
      expect(await p.speak('Envío: revisa el deploy'), isTrue);
      expect(await p.speakReply('**Hecho.** Todo `verde`.'), isTrue);
      expect(tts.said, <String>['Envío: revisa el deploy', 'Hecho. Todo verde.']);
      expect(openai, isEmpty);
      expect(audio.played, isEmpty, reason: 'no OpenAI voice either');
      // The text is held until sent: leaving keeps it as a draft.
      p.dispose();
      await settle();
      expect(drafts, <(String, String)>[('s1', 'revisa el deploy')]);
    });

    test('with a key, what is said is recorded and transcribed like the chat\'s dictation', () async {
      final LiveCarPorts p = await portsWith(key: 'sk-' 'test-0000000000000000000000abcd');
      heard.add('dile a Relotto que siga con el deploy de mañana');
      bool transcribing = false;
      final Future<DictationOutcome> f = p.listen('s1', onTranscribing: () => transcribing = true);
      await waitFor(() => rec.starts == 1 && rec.path != null);
      File(rec.path!).writeAsBytesSync(<int>[1, 2, 3]);
      p.stopListening(); // «Listo» / the silence
      final DictationOutcome o = await f;
      expect((o as DictationText).text, 'dile a Relotto que siga con el deploy de mañana');
      expect(transcribing, isTrue);
      expect(openai, <String>['/v1/audio/transcriptions']);
      expect(stt.answers, isEmpty);
      // Sent: durable in the outbox, the recording is let go, no draft.
      expect((await p.send('s1', null, o.text)).kind, SendKind.delivered);
      p.dispose();
      await settle();
      expect(drafts, isEmpty);
    });

    test('a short answer (a permission option) and commands stay on the phone', () async {
      final LiveCarPorts p = await portsWith(key: 'sk-' 'test-0000000000000000000000abcd');
      stt.answers.add('dos');
      expect(((await p.listen('s1', onTranscribing: () {}, shortAnswer: true)) as DictationText).text, 'dos');
      p.releaseDictation();
      stt.answers.add('cancela');
      expect(await p.hear(const Duration(seconds: 3)), 'cancela');
      expect(rec.starts, 0);
      expect(openai, isEmpty);
    });

    test('a sent on-device message is durable in the outbox, not left as a draft', () async {
      final LiveCarPorts p = await portsWith(key: null);
      stt.answers.add('despliega');
      await p.listen('s1', onTranscribing: () {});
      expect((await p.send('s1', null, 'despliega')).kind, SendKind.delivered);
      p.dispose();
      await settle();
      expect(drafts, isEmpty);
      expect(outbox.items.single.text, 'despliega');
    });

    test('no key and no recognizer: a clear refusal', () async {
      stt.available = false;
      final LiveCarPorts none = await portsWith(key: null);
      expect(await none.listen('s1', onTranscribing: () {}), isA<DictationDenied>());
      expect(await none.hear(const Duration(seconds: 3)), isNull);
      expect(openai, isEmpty);
    });
  });

  group('Modo auto\'s microphone: the take that heard «OK ComandOS» is the turn', () {
    late FakeCarMic mic;
    const String key = 'sk-' 'test-0000000000000000000000abcd';

    LiveCarPorts portsWith({String? key}) => LiveCarPorts(
          voice: voiceWith(key: key),
          outbox: outbox,
          chat: () => chat,
          tts: tts,
          stt: stt,
          mic: mic,
          holdFocus: () async {},
          releaseFocus: () async {},
        );

    /// A take recorded natively into the dictation's folder.
    Future<String> wav(String name) async {
      final Directory d = Directory('${dir.path}/takes')..createSync(recursive: true);
      final File f = File('${d.path}/$name.wav')..writeAsBytesSync(<int>[82, 73, 70, 70]);
      return f.path;
    }

    setUp(() => mic = FakeCarMic());

    test('words said in the same breath are transcribed with the take, without the phrase', () async {
      final LiveCarPorts p = portsWith(key: key);
      p.takeAfterWake('okay comandos qué');
      expect(mic.takes, <String?>['okay comandos qué'], reason: 'the take starts at the wake, before the earcon');
      expect(mic.silences.single, const Duration(seconds: 3), reason: 'the new default silence');
      heard.add('OK ComandOS, ¿qué hay de nuevo?');
      bool transcribing = false;
      final Future<DictationOutcome> f = p.listen('s1', onTranscribing: () => transcribing = true, requireSpeech: true);
      final String path = await wav('take-1');
      mic.done(MicTake(end: 'silence', path: path, text: 'qué hay de nuevo', words: true));
      final DictationOutcome o = await f;
      expect((o as DictationText).text, 'Qué hay de nuevo?');
      expect(mic.takes, hasLength(1), reason: 'listen took the take the wake started');
      expect(transcribing, isTrue);
      expect(openai, <String>['/v1/audio/transcriptions']);
      expect(rec.starts, 0, reason: 'the chat\'s recorder never opens');
      expect((await p.send('s1', null, o.text)).kind, SendKind.delivered);
      expect(outbox.items.single.text, 'Qué hay de nuevo?');
      await settle();
      expect(File(path).existsSync(), isFalse, reason: 'sent: the recording goes');
      p.dispose();
      await settle();
      expect(drafts, isEmpty);
    });

    test('without a key, what the phone heard offline is the text', () async {
      final LiveCarPorts p = portsWith();
      final String path = await wav('take-2');
      mic.next = MicTake(end: 'silence', path: path, text: 'revisa el deploy', words: true);
      final DictationOutcome o = await p.listen('s1', onTranscribing: () {}, requireSpeech: true);
      expect((o as DictationText).text, 'revisa el deploy');
      expect(openai, isEmpty);
      expect(File(path).existsSync(), isFalse);
      // Held until sent: leaving keeps it as a draft.
      p.dispose();
      await settle();
      expect(drafts, <(String, String)>[('s1', 'revisa el deploy')]);
    });

    test('OpenAI failing after its retries: the offline text goes, nothing is dropped', () async {
      final LiveCarPorts p = portsWith(key: key);
      sttError = const OpenAiException(OpenAiErrorKind.unauthorized);
      final String path = await wav('take-3');
      mic.next = MicTake(end: 'silence', path: path, text: 'dile a relotto que siga', words: true);
      final DictationOutcome o = await p.listen('s1', onTranscribing: () {}, requireSpeech: true);
      expect((o as DictationText).text, 'dile a relotto que siga');
      expect(voice.dictation.pendingFor('s1'), isEmpty, reason: 'not left as a pending take');
      expect(File(path).existsSync(), isFalse);
      p.releaseDictation();
    });

    test('OpenAI hearing nothing where the phone heard words: the offline text', () async {
      final LiveCarPorts p = portsWith(key: key);
      heard.add('');
      mic.next = MicTake(end: 'silence', path: await wav('take-4'), text: 'sí', words: true);
      expect(((await p.listen('s1', onTranscribing: () {})) as DictationText).text, 'sí');
      p.releaseDictation();
    });

    test('a take without words (silence, road noise) is dropped without OpenAI', () async {
      final LiveCarPorts p = portsWith(key: key);
      final String path = await wav('take-5');
      mic.next = MicTake(end: 'nowords', path: path);
      expect(await p.listen('s1', onTranscribing: () {}, requireSpeech: true), isA<DictationNothing>());
      expect(openai, isEmpty);
      expect(File(path).existsSync(), isFalse);
    });

    test('«Hablar» ends the take, hanging up drops it, the phone\'s recognizer gets the mic', () async {
      final LiveCarPorts p = portsWith();
      final Future<DictationOutcome> f = p.listen('s1', onTranscribing: () {});
      await settle();
      p.stopListening();
      expect(mic.log, contains('end'));
      mic.done(MicTake(end: 'stopped', path: await wav('take-6'), text: 'listo', words: true));
      expect(((await f) as DictationText).text, 'listo');
      p.releaseDictation();

      final Future<DictationOutcome> g = p.listen('s1', onTranscribing: () {});
      await settle();
      p.cancelListening();
      expect(await g, isA<DictationCancelled>());
      expect(mic.log, contains('cancel'));

      stt.answers.add('dos');
      expect(await p.hear(const Duration(seconds: 3)), 'dos');
      expect(mic.log.last, 'release');
    });

    test('a hang-up right after the phrase drops the take it started', () async {
      final LiveCarPorts p = portsWith();
      p.takeAfterWake('okay comandos');
      p.cancelListening();
      expect(mic.log, <String>['take', 'cancel']);
    });

    test('when Modo auto\'s microphone cannot record, the chat\'s dictation does', () async {
      final LiveCarPorts p = portsWith(key: key);
      mic.next = const MicTake(end: 'error', error: 'No pude cargar el reconocimiento de «OK ComandOS».');
      heard.add('hola');
      final Future<DictationOutcome> f = p.listen('s1', onTranscribing: () {});
      await waitFor(() => rec.starts == 1 && rec.path != null);
      File(rec.path!).writeAsBytesSync(<int>[1, 2, 3]);
      p.stopListening();
      expect(((await f) as DictationText).text, 'hola');
      p.releaseDictation();
    });
  });

  test('a long reply is read whole, in chunks, whatever the chat\'s read mode', () async {
    final String para = List<String>.filled(30, 'Revisé el deploy y todo sigue verde.').join(' ');
    final String reply = '$para\n\nSegundo párrafo con más detalle. $para\n\n¿Sigo?';
    expect(voice.settings.value.autoReadMode, ReadMode.ending, reason: 'the chat would read only the ending');
    expect(await ports.speakReply(reply), isTrue);
    expect(tts.said.length, greaterThan(2));
    expect(tts.said.join(' '), prepareForSpeech(reply).replaceAll('\n\n', ' '));
    expect(tts.said.every((String c) => c.length <= 600), isTrue);
    expect(carReplyChunks(reply), tts.said);
  });

  test('earcon: other apps\' audio is restored afterwards', () async {
    await ports.cue();
    expect(audio.played, hasLength(1));
    expect(audio.releases, 1);
  });
}
