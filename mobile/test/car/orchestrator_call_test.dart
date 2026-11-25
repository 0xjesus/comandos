import 'dart:async';

import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/features/car/orchestrator_call.dart';
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
}
