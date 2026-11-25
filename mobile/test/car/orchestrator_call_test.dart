import 'dart:async';

import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/features/car/orchestrator_call.dart';
import 'package:comandos_mobile/features/car/wake_phrase.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import 'car_machine_test.dart' show FakePorts;

class FakeLink extends ChangeNotifier implements OrchestratorLink {
  final List<ChatMessage> messages = <ChatMessage>[];
  AskPrompt? ask;
  int opens = 0;
  final List<String> answered = <String>[];

  void add(ChatRole r, String t) {
    messages.add(ChatMessage(r, t));
    notifyListeners();
  }

  @override
  Future<(String, String?)> open() async {
    opens++;
    return ('auto', '%9');
  }

  @override
  OrchSnapshot get now => OrchSnapshot(messages: List<ChatMessage>.of(messages), ask: ask, loaded: true);

  @override
  void poke() {}

  @override
  Future<String?> answer(AskPrompt ask, AskOption option) async {
    answered.add(option.key);
    this.ask = null;
    return null;
  }

  @override
  Future<String?> interrupt() async => null;

  @override
  void close() {}
}

class FakeWake implements WakeListener {
  Completer<WakeResult>? waiting;
  int stops = 0;

  @override
  String? problem;

  void hear() => waiting!.complete(WakeResult.heard);

  @override
  Future<WakeResult> wait() => (waiting = Completer<WakeResult>()).future;

  @override
  Future<void> stop() async {
    stops++;
    final Completer<WakeResult>? w = waiting;
    waiting = null;
    if (w != null && !w.isCompleted) w.complete(WakeResult.stopped);
  }
}

Future<void> until(bool Function() ok) async {
  for (int i = 0; i < 400 && !ok(); i++) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
  expect(ok(), isTrue);
}

void main() {
  OrchestratorCall call(FakePorts p, FakeLink l) =>
      OrchestratorCall(ports: p, link: l, settle: const Duration(milliseconds: 30), patience: const Duration(seconds: 5));

  test('the call opens the orchestrator, asks what waits, reads the answer and listens', () async {
    final FakePorts p = FakePorts();
    final FakeLink l = FakeLink();
    final OrchestratorCall c = call(p, l);
    unawaited(c.start());
    await until(() => p.sent.isNotEmpty);
    expect(p.sent.single, ('auto', '%9', OrchestratorCall.kickoff));
    l.add(ChatRole.user, OrchestratorCall.kickoff);
    l.add(ChatRole.tool, 'Bash · comandos auto sesiones');
    l.add(ChatRole.assistant, 'Te espera Relotto.');
    await until(() => p.listening != null);
    expect(p.spoken, contains('REPLY:Te espera Relotto.'));
    expect(c.phase, OrchPhase.listening);

    // What the person says goes to the orchestrator as it is.
    p.dictated(const DictationText('dile a Relotto que siga'));
    await until(() => p.sent.length == 2);
    expect(p.sent.last.$3, 'dile a Relotto que siga');
    l.add(ChatRole.user, 'dile a Relotto que siga');
    l.add(ChatRole.assistant, 'Le mando a Relotto: que siga. ¿Sí?');
    await until(() => p.listening != null);
    expect(p.spoken.last, 'REPLY:Le mando a Relotto: que siga. ¿Sí?');
    c.dispose();
  });

  test('«cuelga» ends the call without sending anything', () async {
    final FakePorts p = FakePorts();
    final FakeLink l = FakeLink();
    final OrchestratorCall c = call(p, l);
    unawaited(c.start());
    await until(() => p.sent.isNotEmpty);
    l.add(ChatRole.user, OrchestratorCall.kickoff);
    l.add(ChatRole.assistant, 'Nada pendiente.');
    await until(() => p.listening != null);
    p.dictated(const DictationText('cuelga'));
    await until(() => c.ended);
    expect(p.sent, hasLength(1));
    c.dispose();
  });

  test('an older identical message is never taken for the new answer', () {
    final ChatMessage oldYes = ChatMessage(ChatRole.user, 'sí');
    final ChatMessage oldDone = ChatMessage(ChatRole.assistant, 'Listo.');
    final List<ChatMessage> before = <ChatMessage>[oldYes, oldDone];
    // Sent «sí» again: until it shows up, there is no answer.
    expect(OrchestratorCall.answerTo(before, oldDone.hash, 'sí'), isNull);
    final List<ChatMessage> after = <ChatMessage>[...before, ChatMessage(ChatRole.user, 'sí'), ChatMessage(ChatRole.assistant, 'Listo.')];
    expect(OrchestratorCall.answerTo(after, oldDone.hash, 'sí')!.single.text, 'Listo.');
    // The window moved past the last message there was: by content.
    final List<ChatMessage> moved = <ChatMessage>[ChatMessage(ChatRole.user, 'sí'), ChatMessage(ChatRole.assistant, 'Hecho.')];
    expect(OrchestratorCall.answerTo(moved, oldDone.hash, 'sí')!.single.text, 'Hecho.');
  });

  test('ten seconds without a voice it sleeps; «OK ComandOS» wakes it', () async {
    DateTime now = DateTime(2026, 11, 25, 9);
    final FakePorts p = FakePorts();
    final FakeLink l = FakeLink();
    final FakeWake w = FakeWake();
    final OrchestratorCall c = OrchestratorCall(
      ports: p,
      link: l,
      wake: w,
      settle: const Duration(milliseconds: 30),
      patience: const Duration(seconds: 5),
      clock: () => now,
    );
    unawaited(c.start());
    await until(() => p.sent.isNotEmpty);
    l.add(ChatRole.user, OrchestratorCall.kickoff);
    l.add(ChatRole.assistant, 'Nada pendiente.');
    await until(() => p.listening != null);
    final int cues = p.cues;

    // The recognizer gave up after 6 s: still within the window, it listens
    // again without a new cue.
    now = now.add(const Duration(seconds: 6));
    p.dictated(const DictationNothing());
    await until(() => p.listening != null);
    expect(p.cues, cues);
    expect(c.phase, OrchPhase.listening);

    // 10 s with no voice: asleep, waiting for the phrase, nothing sent.
    now = now.add(const Duration(seconds: 5));
    p.dictated(const DictationNothing());
    await until(() => w.waiting != null);
    expect(c.phase, OrchPhase.sleeping);
    expect(c.status, OrchestratorCall.sleepingStatus);
    expect(p.sent, hasLength(1));

    // «OK ComandOS»: a cue and it listens.
    w.hear();
    await until(() => p.listening != null);
    expect(p.cues, cues + 1);
    p.dictated(const DictationText('qué hace Relotto'));
    await until(() => p.sent.length == 2);
    expect(p.sent.last.$3, 'qué hace Relotto');
    c.dispose();
  });

  test('the steering wheel wakes it too, and «espera» puts it to sleep', () async {
    final FakePorts p = FakePorts();
    final FakeLink l = FakeLink();
    final FakeWake w = FakeWake();
    final OrchestratorCall c = OrchestratorCall(
        ports: p, link: l, wake: w, settle: const Duration(milliseconds: 30), patience: const Duration(seconds: 5));
    unawaited(c.start());
    await until(() => p.sent.isNotEmpty);
    l.add(ChatRole.user, OrchestratorCall.kickoff);
    l.add(ChatRole.assistant, 'Nada pendiente.');
    await until(() => p.listening != null);
    p.dictated(const DictationText('espera'));
    await until(() => w.waiting != null);
    expect(c.phase, OrchPhase.sleeping);
    unawaited(c.talk());
    await until(() => p.listening != null);
    expect(w.stops, greaterThan(0));
    expect(c.phase, OrchPhase.listening);
    c.dispose();
  });

  test('a phone that cannot hear the phrase says so once and waits for a tap', () async {
    final FakePorts p = FakePorts();
    final FakeLink l = FakeLink();
    final FakeWake w = FakeWake()..problem = 'El micrófono está ocupado.';
    final OrchestratorCall c = OrchestratorCall(
        ports: p, link: l, wake: w, settle: const Duration(milliseconds: 30), patience: const Duration(seconds: 5));
    unawaited(c.start());
    await until(() => p.sent.isNotEmpty);
    l.add(ChatRole.user, OrchestratorCall.kickoff);
    l.add(ChatRole.assistant, 'Nada pendiente.');
    await until(() => p.listening != null);
    p.dictated(const DictationText('espera'));
    await until(() => w.waiting != null);
    w.waiting!.complete(WakeResult.unavailable);
    await until(() => p.spoken.any((String s) => s.contains('El micrófono está ocupado.')));
    expect(c.phase, OrchPhase.idle);
    c.dispose();
  });

  test('the wake phrase as the offline recognizer writes it', () {
    for (final String yes in <String>[
      'okay comandos',
      'oye comandos',
      'okay y comandos',
      'oye y comandos',
      'aunque hay comandos',
      'okay comandos qué hay de nuevo',
      'OK ComandOS',
      'ok comando',
    ]) {
      expect(isWakePhrase(yes), isTrue, reason: yes);
    }
    for (final String no in <String>[
      'vamos a comprar unos tacos al rato',
      'los comandos de la terminal no sirven',
      'okay cucho pon música',
      'okay google pon música',
      'comando especial de la policía',
      'okay como andas',
      'okay revisa los comandos',
      'comandos',
      '',
    ]) {
      expect(isWakePhrase(no), isFalse, reason: no);
    }
  });
}
