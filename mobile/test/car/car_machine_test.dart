import 'dart:async';

import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/features/car/car_machine.dart';
import 'package:comandos_mobile/features/car/option_parser.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/openai_client.dart';
import 'package:comandos_mobile/features/voice/reply_watcher.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import '../voice/voice_fakes.dart';

class FakePorts implements CarPorts {
  @override
  bool continuous = false;
  bool autoSpeak = true;

  /// What [speak] / [speakReply] report when [autoSpeak] (false = TTS failed).
  bool speakOk = true;
  final List<bool> requireSpeech = <bool>[];
  final List<(String?, String)> answerBindings = <(String?, String)>[];
  SendOutcome sendResult = const SendOutcome(SendKind.delivered);
  String? answerError;

  final List<String> log = <String>[];
  final List<String> spoken = <String>[];
  final List<(String, String?, String)> sent = <(String, String?, String)>[];
  final List<(String, String)> drafts = <(String, String)>[];
  final List<(String, String)> answers = <(String, String)>[];
  final List<String> reads = <String>[];
  final List<String> retried = <String>[];
  final List<String> discarded = <String>[];
  Completer<DictationOutcome>? listening;
  VoidCallback? onTranscribing;
  Completer<bool>? speaking;
  int cues = 0;

  /// What the person said in the current dictation.
  void dictated(DictationOutcome o) {
    final Completer<DictationOutcome> c = listening!;
    listening = null;
    c.complete(o);
  }

  final List<bool> shortAnswers = <bool>[];

  @override
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false, bool shortAnswer = false}) {
    log.add('listen:$session');
    this.requireSpeech.add(requireSpeech);
    shortAnswers.add(shortAnswer);
    this.onTranscribing = onTranscribing;
    return (listening = Completer<DictationOutcome>()).future;
  }

  /// A command listen in progress ([hear]); null when not listening.
  Completer<String?>? hearing;
  final List<Duration> hearWindows = <Duration>[];

  /// Nothing can listen on the phone ([hear] returns null).
  bool canHear = true;

  @override
  Future<String?> hear(Duration window, {bool Function(String partial)? until}) {
    hearWindows.add(window);
    log.add('hear');
    if (!canHear) return Future<String?>.value(null);
    return (hearing = Completer<String?>()).future;
  }

  /// The person says [text] while a command listen is open.
  void command(String text) {
    final Completer<String?> c = hearing!;
    hearing = null;
    c.complete(text);
  }

  @override
  void stopHearing() {
    log.add('stopHearing');
    final Completer<String?>? c = hearing;
    hearing = null;
    if (c != null && !c.isCompleted) c.complete('');
  }

  @override
  void stopListening() {
    log.add('stopListening');
    onTranscribing?.call();
  }

  @override
  void cancelListening() {
    log.add('cancelListening');
    if (listening != null) dictated(const DictationCancelled());
  }

  @override
  Future<DictationOutcome> retry(String takeId) {
    retried.add(takeId);
    return (listening = Completer<DictationOutcome>()).future;
  }

  @override
  Future<void> discard(String takeId) async => discarded.add(takeId);

  @override
  void releaseDictation() => log.add('release');

  Future<bool> _talk(String what) {
    spoken.add(what);
    if (autoSpeak) return Future<bool>.value(speakOk);
    return (speaking = Completer<bool>()).future;
  }

  @override
  Future<bool> speak(String text) => _talk(text);

  @override
  Future<bool> speakReply(String markdown) => _talk('REPLY:$markdown');

  /// Ends the current utterance naturally.
  void spoke() {
    final Completer<bool>? c = speaking;
    speaking = null;
    c?.complete(true);
  }

  @override
  void stopSpeaking() {
    log.add('stopSpeaking');
    final Completer<bool>? c = speaking;
    speaking = null;
    if (c != null && !c.isCompleted) c.complete(false);
  }

  @override
  Future<SendOutcome> send(String session, String? pane, String text) async {
    sent.add((session, pane, text));
    return sendResult;
  }

  @override
  Future<void> saveDraft(String session, String text) async => drafts.add((session, text));

  @override
  Future<String?> answer(String session, String optionKey, {String? pane, required String promptSig}) async {
    answers.add((session, optionKey));
    answerBindings.add((pane, promptSig));
    return answerError;
  }

  @override
  Future<String?> interrupt(String session) async {
    log.add('interrupt:$session');
    return null;
  }

  @override
  void markRead(String session, String replyId) => reads.add('$session:$replyId');

  @override
  Future<void> cue() async => cues++;
}

const AskPrompt ask = AskPrompt(question: '¿Ejecutar git push?', options: <AskOption>[
  AskOption(key: '1', label: 'Yes'),
  AskOption(key: '2', label: 'No'),
]);

CarSession s(String id, SessionState st, {double ts = 0, String? label}) =>
    CarSession(session: id, label: label ?? id.toUpperCase(), state: st, ts: ts, pane: '%$id');

void main() {
  late FakePorts ports;
  late FakeTimers clock;
  late CarMachine m;

  setUp(() {
    ports = FakePorts();
    clock = FakeTimers();
    m = CarMachine(ports: ports, session: 's1', now: clock.now, timer: clock.call);
    m.updateSessions(<CarSession>[s('s1', SessionState.idle, ts: 3), s('s2', SessionState.work, ts: 2)]);
  });

  Future<void> speakAndSend(String text) async {
    unawaited(m.talk());
    await flush();
    expect(m.phase, CarPhase.listening);
    unawaited(m.talk()); // tap again: stop
    expect(m.phase, CarPhase.transcribing);
    ports.dictated(DictationText(text));
    await flush();
  }

  test('orders sessions: waiting, working, then the rest by recency', () {
    final List<CarSession> o = orderForCar(<CarSession>[
      s('a', SessionState.idle, ts: 9),
      s('b', SessionState.work, ts: 1),
      s('c', SessionState.ask, ts: 0),
      s('d', SessionState.idle, ts: 10),
    ]);
    expect(o.map((CarSession x) => x.session), <String>['c', 'b', 'd', 'a']);
  });

  test('happy path: Hablar → text → "Envío: …" → 3 s countdown → outbox → "Enviado"', () async {
    await speakAndSend('revisa el deploy');
    expect(m.phase, CarPhase.confirming);
    expect(m.heard, 'revisa el deploy');
    expect(ports.spoken.last, 'Envío: revisa el deploy');
    expect(m.count, 3);
    await clock.elapse(const Duration(seconds: 2));
    expect(ports.sent, isEmpty);
    expect(m.status, 'Envío en 1…');
    await clock.elapse(const Duration(seconds: 1));
    expect(ports.sent, <(String, String?, String)>[('s1', '%s1', 'revisa el deploy')]);
    expect(ports.spoken.last, 'Enviado.');
    expect(m.phase, CarPhase.idle);
    expect(ports.log.first, 'stopSpeaking'); // listening stops any speech first
  });

  test('Enviar ya skips the countdown', () async {
    await speakAndSend('ya');
    await m.sendNow();
    expect(ports.sent.single.$3, 'ya');
    await clock.elapse(const Duration(seconds: 5));
    expect(ports.sent, hasLength(1));
  });

  test('Cancelar during the countdown: nothing sent, the text is kept as a draft', () async {
    await speakAndSend('no lo mandes');
    await clock.elapse(const Duration(seconds: 1));
    m.cancel();
    await flush();
    await clock.elapse(const Duration(seconds: 5));
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('s1', 'no lo mandes')]);
    expect(m.phase, CarPhase.idle);
    expect(ports.spoken.last, 'Cancelado. Quedó como borrador.');
  });

  test('transcription failure keeps the recording; the big button retries it', () async {
    unawaited(m.talk());
    await flush();
    final PendingTake take = PendingTake(id: 'take-1', owner: 's1', path: '/x.m4a', createdAtMs: 0);
    ports.dictated(DictationFailed(take, 'Sin conexión con OpenAI.', OpenAiErrorKind.network));
    await flush();
    expect(m.phase, CarPhase.failed);
    expect(m.notice, 'Sin conexión con OpenAI.');
    expect(ports.spoken.last, contains('toca reintentar'));
    unawaited(m.talk()); // Reintentar
    await flush();
    expect(ports.retried, <String>['take-1']);
    expect(m.phase, CarPhase.transcribing);
    ports.dictated(const DictationText('ahora sí'));
    await flush();
    expect(m.phase, CarPhase.confirming);

    // Or Descartar.
    m.cancel();
    await flush();
    unawaited(m.talk());
    await flush();
    ports.dictated(DictationFailed(take, 'x', OpenAiErrorKind.server));
    await flush();
    await m.discard();
    expect(ports.discarded, <String>['take-1']);
    expect(m.phase, CarPhase.idle);
  });

  test('offline send is spoken as such (the outbox will deliver later)', () async {
    ports.sendResult = const SendOutcome(SendKind.queuedOffline);
    await speakAndSend('hola');
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.sent, hasLength(1));
    expect(ports.spoken.last, 'Sin conexión, se enviará al volver la red.');
    expect(m.notice, contains('Sin conexión'));
  });

  test('a completed reply is read once', () async {
    const ReplyRef r = ReplyRef('id1', 'Hecho, todo verde.');
    m.replyCompleted(r);
    await flush();
    expect(ports.spoken.where((String x) => x.startsWith('REPLY:')), <String>['REPLY:Hecho, todo verde.']);
    expect(ports.reads, <String>['s1:id1']);
    m.replyCompleted(r);
    await flush();
    expect(ports.spoken.where((String x) => x.startsWith('REPLY:')), hasLength(1));
    expect(ports.listening, isNull, reason: 'not continuous: waits for the button');
  });

  test('a reply arriving mid-dictation waits until the machine is free', () async {
    unawaited(m.talk());
    await flush();
    m.replyCompleted(const ReplyRef('id2', 'Respuesta'));
    await flush();
    expect(ports.spoken.where((String x) => x.startsWith('REPLY:')), isEmpty);
    ports.dictated(const DictationCancelled());
    await flush();
    expect(ports.spoken.last, 'REPLY:Respuesta');
  });

  test('continuous conversation: after the reply, a cue and listening again', () async {
    ports.continuous = true;
    m.replyCompleted(const ReplyRef('id3', 'Listo.'));
    await flush();
    expect(ports.cues, 1);
    expect(ports.log.last, 'listen:s1');
    expect(m.phase, CarPhase.listening);
    // Callar during the reading would have stopped the loop:
    ports.dictated(const DictationCancelled());
    await flush();
    ports.autoSpeak = false;
    m.replyCompleted(const ReplyRef('id4', 'Otra.'));
    await flush();
    expect(m.phase, CarPhase.reading);
    m.hush();
    await flush();
    expect(m.phase, CarPhase.idle);
    expect(ports.cues, 1);
  });

  test('permission prompt read aloud, answered by a giant button', () async {
    m.updateCurrent(working: false, ask: ask);
    await flush();
    expect(ports.spoken.last, '¿Ejecutar git push? Opción uno: sí. Opción dos: no.');
    expect(m.status, 'Te espera: elige una opción');
    await m.chooseOption(ask.options[1]);
    expect(ports.answers, <(String, String)>[('s1', '2')]);
    expect(ports.spoken.last, 'Listo.');
    expect(m.ask, isNull);
    // The same prompt lingering in the next poll is not read again.
    final int n = ports.spoken.length;
    m.updateCurrent(working: false, ask: ask);
    await flush();
    expect(ports.spoken, hasLength(n));
  });

  test('permission answered by voice only after "Elegiste…" and a cancellable countdown', () async {
    m.updateCurrent(working: false, ask: ask);
    await flush();
    unawaited(m.talk());
    await flush();
    ports.dictated(const DictationText('la opción dos'));
    await flush();
    expect(m.phase, CarPhase.choosing);
    expect(m.pendingOption!.key, '2');
    expect(ports.spoken.last, 'Elegiste opción dos, no.');
    await clock.elapse(const Duration(seconds: 2));
    expect(ports.answers, isEmpty);
    await clock.elapse(const Duration(seconds: 1));
    expect(ports.answers, <(String, String)>[('s1', '2')]);
  });

  test('voice answer: cancel during the countdown does nothing; ambiguity asks again', () async {
    m.updateCurrent(working: false, ask: ask);
    await flush();
    unawaited(m.talk());
    await flush();
    ports.dictated(const DictationText('sí'));
    await flush();
    expect(m.pendingOption!.key, '1');
    m.cancel();
    await clock.elapse(const Duration(seconds: 5));
    expect(ports.answers, isEmpty);
    expect(m.phase, CarPhase.idle);

    unawaited(m.talk());
    await flush();
    ports.dictated(const DictationText('pues no sé, tal vez'));
    await flush();
    expect(ports.spoken.last, startsWith('No entendí.'));
    expect(m.phase, CarPhase.listening, reason: 'asks again, never guesses');
    expect(ports.answers, isEmpty);
  });

  test('switching session while speaking stops the speech and announces the new one', () async {
    ports.autoSpeak = false;
    m.replyCompleted(const ReplyRef('id5', 'Larga respuesta'));
    await flush();
    expect(m.phase, CarPhase.reading);
    m.next();
    await flush();
    expect(ports.log, contains('stopSpeaking'));
    expect(m.currentId, 's2');
    expect(m.phase, CarPhase.idle);
    expect(m.working, isTrue);
    expect(ports.spoken.last, 'S2, trabajando.');
  });

  test('cannot switch mid-dictation or countdown (the text belongs to its session)', () async {
    await speakAndSend('para s1');
    m.next();
    expect(m.currentId, 's1');
  });

  test('other sessions: "terminó" / "te espera" announcements, rate-limited', () async {
    m.updateSessions(<CarSession>[s('s1', SessionState.idle, ts: 3), s('s2', SessionState.idle, ts: 2), s('s3', SessionState.ask)]);
    await flush();
    expect(ports.spoken, contains('S2 terminó.'));
    // s3 was not seen before: no announcement for a first sighting.
    expect(ports.spoken.where((String x) => x.startsWith('S3')), isEmpty);
    m.updateSessions(<CarSession>[s('s1', SessionState.idle), s('s2', SessionState.ask), s('s3', SessionState.ask)]);
    await flush();
    expect(ports.spoken.where((String x) => x.startsWith('S2')), hasLength(1), reason: 'rate limit');
    clock.nowMs += 60000;
    m.updateSessions(<CarSession>[s('s1', SessionState.idle), s('s2', SessionState.work), s('s3', SessionState.ask)]);
    m.updateSessions(<CarSession>[s('s1', SessionState.idle), s('s2', SessionState.ask), s('s3', SessionState.ask)]);
    await flush();
    expect(ports.spoken.last, 'S2 te espera: pide permiso.');
  });

  test('Detener sends Escape through the chat action', () async {
    await m.stopAgent();
    expect(ports.log, contains('interrupt:s1'));
    expect(ports.spoken.last, 'Detenido.');
  });

  test('leaving during the countdown keeps the text; while listening it stops (kept)', () async {
    await speakAndSend('a medias');
    m.dispose();
    expect(ports.drafts, <(String, String)>[('s1', 'a medias')]);

    final FakePorts p2 = FakePorts();
    final CarMachine m2 = CarMachine(ports: p2, session: 's1', now: clock.now, timer: clock.call);
    m2.updateSessions(<CarSession>[s('s1', SessionState.idle)]);
    unawaited(m2.talk());
    await flush();
    m2.dispose();
    expect(p2.log, contains('stopListening'));
    p2.dictated(const DictationText('tarde'));
    await flush();
    expect(p2.sent, isEmpty);
  });

  group('parseOptionChoice', () {
    const List<AskOption> three = <AskOption>[
      AskOption(key: '1', label: 'Yes'),
      AskOption(key: '2', label: "Yes, and don't ask again for git"),
      AskOption(key: '3', label: 'No, and tell Claude what to do differently'),
    ];

    test('numbers, words, ordinals and "opción N"', () {
      expect(parseOptionChoice('2', ask.options)!.key, '2');
      expect(parseOptionChoice('Dos.', ask.options)!.key, '2');
      expect(parseOptionChoice('opción uno', ask.options)!.key, '1');
      expect(parseOptionChoice('la tercera', three)!.key, '3');
    });

    test('sí / no map to the matching option', () {
      expect(parseOptionChoice('Sí', ask.options)!.key, '1');
      expect(parseOptionChoice('no', ask.options)!.key, '2');
      expect(parseOptionChoice('sí', three)!.key, '1', reason: 'the bare "Yes"');
      expect(parseOptionChoice('no', three)!.key, '3');
    });

    test('ambiguous or unrelated → null', () {
      expect(parseOptionChoice('uno o dos', ask.options), isNull);
      expect(parseOptionChoice('sí, la dos', ask.options), isNull);
      expect(parseOptionChoice('sí no', ask.options), isNull);
      expect(parseOptionChoice('cinco', ask.options), isNull);
      expect(parseOptionChoice('mándalo a producción', ask.options), isNull);
      expect(parseOptionChoice('', ask.options), isNull);
    });

    test('describeOptions', () {
      expect(describeOptions(three), startsWith('Opción uno: sí. Opción dos: Yes, and'));
    });
  });

  // ---- Safety (review findings) ------------------------------------------------

  const AskPrompt three = AskPrompt(question: '¿Ejecutar rm?', options: <AskOption>[
    AskOption(key: '1', label: 'Yes'),
    AskOption(key: '2', label: "Yes, and don't ask again"),
    AskOption(key: '3', label: 'No, and tell Claude what to do differently'),
  ]);
  const AskPrompt other = AskPrompt(question: '¿Borrar la base?', options: <AskOption>[
    AskOption(key: '1', label: 'Yes'),
    AskOption(key: '2', label: 'No'),
  ]);

  /// Prompt shown and read aloud, then Hablar.
  Future<void> promptThenTalk(AskPrompt p, {String? pane}) async {
    m.updateCurrent(working: false, ask: p, pane: pane);
    await flush();
    unawaited(m.talk());
    await flush();
    expect(m.phase, CarPhase.listening);
  }

  test('1: "No, espera una cosa" and "dame un segundo" answer nothing', () async {
    await promptThenTalk(three);
    ports.dictated(const DictationText('No, espera una cosa'));
    await flush();
    expect(m.phase, isNot(CarPhase.choosing));
    ports.dictated(const DictationText('dame un segundo')); // the re-ask listens again
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
    expect(ports.spoken.where((String x) => x.startsWith('No entendí')), hasLength(2));
  });

  test('2: no read-back heard → no countdown, nothing sent until a tap', () async {
    ports.speakOk = false; // TTS failed (429, no network, focus lost)
    await speakAndSend('borra la rama');
    expect(m.phase, CarPhase.confirming);
    expect(m.needsTap, isTrue);
    expect(clock.pending, 0, reason: 'no timer armed');
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.sent, isEmpty);
    expect(m.status, 'Toca Enviar o Cancelar');
    await m.sendNow(); // the tap
    expect(ports.sent.single.$3, 'borra la rama');
  });

  test('2: Callar during the read-back also waits for a tap', () async {
    ports.autoSpeak = false;
    unawaited(m.talk());
    await flush();
    ports.dictated(const DictationText('hola'));
    await flush();
    expect(ports.spoken.last, 'Envío: hola');
    m.hush();
    await flush();
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.sent, isEmpty);
    expect(m.phase, CarPhase.confirming);
    expect(m.needsTap, isTrue);
  });

  test('2: an option whose read-back failed waits for Confirmar', () async {
    await promptThenTalk(three);
    ports.speakOk = false;
    ports.dictated(const DictationText('opción tres'));
    await flush();
    expect(m.phase, CarPhase.choosing);
    expect(m.needsTap, isTrue);
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.answers, isEmpty);
    await m.confirmChoice();
    expect(ports.answers, <(String, String)>[('s1', '3')]);
  });

  test('3: a prompt replaced during the countdown is not answered; the new one is read', () async {
    await promptThenTalk(three, pane: '%1');
    ports.dictated(const DictationText('uno'));
    await flush();
    expect(m.phase, CarPhase.choosing);
    m.updateCurrent(working: false, ask: other, pane: '%1');
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
    expect(m.phase, isNot(CarPhase.choosing));
    expect(ports.spoken, contains('¿Borrar la base? Opción uno: sí. Opción dos: no.'));
  });

  test('3: a pane change during the countdown cancels the choice', () async {
    await promptThenTalk(three, pane: '%1');
    ports.dictated(const DictationText('uno'));
    await flush();
    m.updateCurrent(working: false, ask: three, pane: '%2');
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
  });

  test('3: the answer carries the prompt and pane it was chosen for', () async {
    await promptThenTalk(three, pane: '%4');
    ports.dictated(const DictationText('la tercera'));
    await flush();
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.answers, <(String, String)>[('s1', '3')]);
    expect(ports.answerBindings.single, ('%4', three.signature));
  });

  test('3: no automatic session switch while listening, confirming or answering', () async {
    unawaited(m.talk());
    await flush();
    m.updateSessions(<CarSession>[s('s2', SessionState.work)]); // s1 missing from one snapshot
    expect(m.currentId, 's1');
    ports.dictated(const DictationText('para s1'));
    await flush();
    m.updateSessions(<CarSession>[s('s2', SessionState.work)]);
    expect(m.currentId, 's1');
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.sent.single.$1, 's1');
    // Idle again: now it may move on.
    m.updateSessions(<CarSession>[s('s2', SessionState.work)]);
    expect(m.currentId, 's2');
  });

  test('4: speech before the prompt appeared is never an answer (nor a message)', () async {
    unawaited(m.talk());
    await flush();
    m.updateCurrent(working: false, ask: three); // the prompt appears while listening
    await flush();
    ports.dictated(const DictationText('No, primero revisa los logs'));
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('s1', 'No, primero revisa los logs')]);
    expect(ports.spoken, contains('¿Ejecutar rm? ${describeOptions(three.options)}'));
  });

  test('4: a prompt on screen but not read to the end is not answered by speech', () async {
    ports.autoSpeak = false;
    m.updateCurrent(working: false, ask: three);
    await flush();
    m.hush(); // cut the reading
    ports.autoSpeak = true;
    unawaited(m.talk());
    await flush();
    ports.dictated(const DictationText('uno'));
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
    expect(ports.sent, isEmpty, reason: 'a prompt is open: never a normal message either');
    expect(ports.drafts.single.$2, 'uno');
  });

  test('15c: a prompt appearing during the send countdown keeps the text as a draft', () async {
    await speakAndSend('despliega');
    expect(m.phase, CarPhase.confirming);
    m.updateCurrent(working: false, ask: three);
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('s1', 'despliega')]);
    expect(ports.spoken, contains('Hay un permiso abierto. Tu mensaje quedó como borrador.'));
  });

  test('5: messages go to the pane the chat reads', () async {
    m.updateCurrent(working: false, pane: '%7');
    await speakAndSend('hola');
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.sent.single.$2, '%7');
  });

  test('7: a queue blocked by an earlier message is reported as such', () async {
    ports.sendResult = const SendOutcome(SendKind.blocked);
    await speakAndSend('hola');
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.spoken.last, 'Hay un mensaje anterior sin resolver. Ábrelo en el chat.');
    expect(m.notice, isNot(contains('Sin conexión')));
  });

  test('8: in the background nothing speaks, listens or counts down; back, it resumes', () async {
    ports.autoSpeak = false;
    m.replyCompleted(const ReplyRef('r1', 'Larga'));
    await flush();
    expect(m.phase, CarPhase.reading);
    m.setForeground(false);
    expect(ports.log, contains('stopSpeaking'));
    expect(m.phase, CarPhase.idle);
    final int spokenBefore = ports.spoken.length;
    m.replyCompleted(const ReplyRef('r2', 'Otra'));
    m.updateCurrent(working: false, ask: three);
    await flush();
    expect(ports.spoken.length, spokenBefore, reason: 'silent while hidden');
    await m.talk();
    expect(ports.listening, isNull, reason: 'no listening while hidden');
    m.setForeground(true);
    await flush();
    expect(ports.spoken.last, 'REPLY:Otra');
  });

  test('8: hidden mid-dictation stops recording; the text waits for a tap', () async {
    unawaited(m.talk());
    await flush();
    m.setForeground(false);
    expect(ports.log, contains('stopListening'));
    ports.dictated(const DictationText('algo'));
    await flush();
    await clock.elapse(const Duration(seconds: 30));
    expect(m.phase, CarPhase.confirming);
    expect(m.needsTap, isTrue);
    expect(ports.sent, isEmpty);
  });

  test('8: hidden during the countdown cancels it', () async {
    await speakAndSend('algo');
    m.setForeground(false);
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.sent, isEmpty);
    expect(m.needsTap, isTrue);
  });

  test('9: spoken answers and re-asks release the dictation, never as a draft', () async {
    await promptThenTalk(three);
    ports.dictated(const DictationText('pues no sé'));
    await flush();
    expect(ports.log.where((String x) => x == 'release'), hasLength(1));
    ports.dictated(const DictationText('opción tres'));
    await flush();
    await clock.elapse(const Duration(seconds: 3));
    expect(ports.log.where((String x) => x == 'release'), hasLength(2));
    expect(ports.answers.single.$2, '3');
    m.dispose();
    expect(ports.drafts, isEmpty);
  });

  test('continuous: automatic rounds drop silence; two empty rounds stop the loop', () async {
    ports.continuous = true;
    m.replyCompleted(const ReplyRef('a', 'Uno.'));
    await flush();
    expect(ports.requireSpeech.last, isTrue);
    ports.dictated(const DictationNothing());
    await flush();
    m.replyCompleted(const ReplyRef('b', 'Dos.'));
    await flush();
    ports.dictated(const DictationNothing());
    await flush();
    final int listens = ports.requireSpeech.length;
    m.replyCompleted(const ReplyRef('c', 'Tres.'));
    await flush();
    expect(ports.spoken.last, 'REPLY:Tres.');
    expect(ports.requireSpeech.length, listens, reason: 'loop stopped');
    unawaited(m.talk()); // the button restarts it; a tap never drops silence
    await flush();
    expect(ports.requireSpeech.last, isFalse);
  });
}
