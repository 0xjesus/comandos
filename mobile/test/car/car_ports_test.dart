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
import 'package:comandos_mobile/features/car/car_machine.dart';
import 'package:comandos_mobile/features/car/car_ports.dart';
import 'package:comandos_mobile/features/chat/chat_controller.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../core/outbox_test.dart' show FakeTransport;
import '../voice/voice_fakes.dart';

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
    voice = voiceWith(key: 'sk-test-0000000000000000000000abcd');
    // These tests dictate through OpenAI (the setting is on).
    await voice.settings.update((VoiceSettings s) => s.copyWith(carCloudDictation: true));
    ports = LiveCarPorts(voice: voice, outbox: outbox, chat: () => chat, tts: tts, stt: stt);
  });

  tearDown(() async {
    chat.dispose();
    outbox.dispose();
    await dir.delete(recursive: true);
  });

  List<String> takes() {
    final Directory d = Directory('${dir.path}/takes');
    return d.existsSync() ? d.listSync().map((FileSystemEntity e) => e.path).toList() : <String>[];
  }

  /// One dictation through the ports: returns its outcome (the text is
  /// then held by the ports until sent, saved or released).
  Future<DictationOutcome> dictate(String text) async {
    heard.add(text);
    final int before = rec.starts;
    final Future<DictationOutcome> f = ports.listen('s1', onTranscribing: () {});
    await waitFor(() => rec.starts > before && voice.dictation.phase == DictationPhase.recording);
    File(rec.path!).writeAsBytesSync(<int>[1, 2, 3]);
    unawaited(voice.dictation.stop());
    return f;
  }

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

  test('9: a voice answer releases the dictation: no draft, recording deleted', () async {
    final DictationOutcome o = await dictate('dos');
    expect(o, isA<DictationText>());
    expect(takes(), isNotEmpty, reason: 'held until the text is safe or released');
    ports.releaseDictation();
    await waitFor(() => takes().isEmpty);
    ports.dispose();
    await settle();
    expect(drafts, isEmpty);
  });

  test('9: a second dictation never orphans the first recording', () async {
    await dictate('primer texto');
    final List<String> first = takes();
    expect(first, isNotEmpty);
    // No send / save / release happened (a bug path): the next dictation
    // keeps the text as a draft and lets the recording go.
    final Future<DictationOutcome> second = dictate('segundo');
    await waitFor(() => first.every((String p) => !File(p).existsSync()));
    expect(drafts, <(String, String)>[('s1', 'primer texto')]);
    expect(await second, isA<DictationText>());
    ports.releaseDictation();
    await waitFor(() => takes().isEmpty);
    expect(voice.dictation.pendingFor('s1'), isEmpty);
  });

  test('3: an answer for another prompt, pane or session is refused without a request', () async {
    expect(await ports.answer('s1', '1', pane: '%7', promptSig: 'otra pregunta'),
        'La pregunta cambió; no respondí.');
    expect(await ports.answer('s2', '1', pane: '%7', promptSig: 'x'), 'La sesión cambió; no respondí.');
    expect(requests, isEmpty);
  });

  group('tokens: the phone\'s own voice and recognizer unless OpenAI is allowed', () {
    Future<LiveCarPorts> portsWith({String? key, bool cloud = false}) async {
      final VoiceServices v = voiceWith(key: key);
      await v.settings.update((VoiceSettings s) => s.copyWith(carCloudDictation: cloud));
      return LiveCarPorts(voice: v, outbox: outbox, chat: () => chat, tts: tts, stt: stt);
    }

    test('without a key everything is on the phone: nothing goes to OpenAI', () async {
      final LiveCarPorts p = await portsWith(key: null, cloud: true);
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

    test('with a key but the setting off (the default), dictation stays on the phone', () async {
      final LiveCarPorts p = await portsWith(key: 'sk-test-0000000000000000000000abcd');
      stt.answers.add('hola');
      expect(await p.listen('s1', onTranscribing: () {}), isA<DictationText>());
      expect(rec.starts, 0);
      expect(openai, isEmpty);
      p.releaseDictation();
    });

    test('with the setting on, only the dictated message goes to OpenAI', () async {
      // A short answer (a permission option) stays on the phone.
      stt.answers.add('dos');
      expect(((await ports.listen('s1', onTranscribing: () {}, shortAnswer: true)) as DictationText).text, 'dos');
      expect(rec.starts, 0);
      ports.releaseDictation();
      // Commands too.
      stt.answers.add('cancela');
      expect(await ports.hear(const Duration(seconds: 3)), 'cancela');
      expect(openai, isEmpty);
      // The message: OpenAI.
      expect(await dictate('revisa los logs'), isA<DictationText>());
      expect(openai, <String>['/v1/audio/transcriptions']);
      ports.releaseDictation();
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

    test('a phone with no recognizer: OpenAI if there is a key, else a clear refusal', () async {
      stt.available = false;
      final LiveCarPorts none = await portsWith(key: null);
      final DictationOutcome o = await none.listen('s1', onTranscribing: () {});
      expect(o, isA<DictationDenied>());
      expect(await none.hear(const Duration(seconds: 3)), isNull);
      // With a key (even with the setting off) the message still gets
      // transcribed.
      await voice.settings.update((VoiceSettings s) => s.copyWith(carCloudDictation: false));
      expect(await dictate('hola'), isA<DictationText>());
      expect(openai, <String>['/v1/audio/transcriptions']);
      ports.releaseDictation();
    });
  });

  test('earcon: other apps\' audio is restored afterwards', () async {
    await ports.cue();
    expect(audio.played, hasLength(1));
    expect(audio.releases, 1);
  });
}
