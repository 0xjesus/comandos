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
    voice = testVoiceServices(
      dir: dir.path,
      key: 'sk-test-0000000000000000000000abcd',
      recorder: rec,
      audio: audio,
      transcribe: (String path, Future<void> abort) async => heard.removeAt(0),
      fallback: (String owner, String text) async => drafts.add((owner, text)),
    );
    ports = LiveCarPorts(voice: voice, outbox: outbox, chat: () => chat);
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

  test('earcon: other apps\' audio is restored afterwards', () async {
    await ports.cue();
    expect(audio.played, hasLength(1));
    expect(audio.releases, 1);
  });
}
