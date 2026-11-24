import 'dart:async';

import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/core/models/session_feed.dart';
import 'package:comandos_mobile/features/car/car_machine.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/reply_watcher.dart';
import 'package:comandos_mobile/features/notifications/car_text.dart';
import 'package:flutter_test/flutter_test.dart';

import '../voice/voice_fakes.dart';
import 'car_machine_test.dart' show FakePorts, ask;

/// Modo auto as a phone call: no tap needed anywhere.
CarSession sp(
  String id,
  SessionState st, {
  String? label,
  int split = 1,
  int splits = 1,
  double ts = 0,
  bool unread = false,
  String agent = '',
  String model = '',
}) =>
    CarSession(
      session: id,
      label: label ?? id.toUpperCase(),
      state: st,
      ts: ts,
      pane: '%$id$split',
      split: split,
      splits: splits,
      unread: unread,
      agent: agent,
      model: model,
    );

void main() {
  late FakePorts ports;
  late FakeTimers clock;

  CarMachine machine({String? session}) =>
      CarMachine(ports: ports, session: session, now: clock.now, timer: clock.call);

  setUp(() {
    ports = FakePorts();
    clock = FakeTimers();
  });

  /// Starts the call on [m] and lets the current split's chat "load".
  Future<void> start(CarMachine m, {AskPrompt? prompt, ReplyRef? latest, bool working = false}) async {
    unawaited(m.startCall());
    await flush();
    m.updateCurrent(working: working, ask: prompt, latest: latest, latestUnread: latest != null);
    await flush();
  }

  test('opening: says how many splits wait, visits the first, reads its prompt and listens for the answer',
      () async {
    final CarMachine m = machine();
    m.updateSessions(<CarSession>[
      sp('p', SessionState.idle, label: 'Pagos', ts: 5, splits: 2, agent: 'claude', model: 'opus-5'),
      sp('p', SessionState.ask, label: 'Pagos', ts: 4, split: 2, splits: 2, agent: 'claude', model: 'opus-5'),
      sp('d', SessionState.work, label: 'Docs', ts: 3),
      sp('w', SessionState.idle, label: 'Web', ts: 1, unread: true),
    ]);
    unawaited(m.startCall());
    await flush();
    expect(ports.spoken.first, '2 splits te esperan. Pagos, split 2, claude opus 5.');
    expect(m.current!.pane, '%p2');
    expect(ports.listening, isNull, reason: 'waits for the split\'s chat first');
    m.updateCurrent(working: false, ask: ask, pane: '%p2');
    await flush();
    expect(ports.spoken.last, askCarText(ask));
    expect(ports.cues, 1);
    expect(m.phase, CarPhase.listening);
    expect(ports.shortAnswers.last, isTrue, reason: 'an answer: the phone\'s recognizer, never OpenAI');

    // "La dos": read back, «¿Confirmo?», and only an explicit «sí» answers.
    ports.dictated(const DictationText('la dos'));
    await flush();
    expect(ports.spoken.last, 'Opción dos: no. ¿Confirmo?');
    expect(ports.hearing, isNotNull);
    expect(ports.hearWindows.last, const Duration(seconds: 6));
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.answers, isEmpty, reason: 'silence is never a yes');
    ports.command('sí');
    await flush();
    expect(ports.answers, <(String, String)>[('p', '2')]);

    // Then on to the next split that waits: Web, with an unread reply.
    expect(ports.spoken, containsAllInOrder(<String>['Listo.', 'Web, split 1.']));
    m.updateCurrent(working: false, latest: const ReplyRef('r1', 'Listo, desplegado.'), latestUnread: true);
    await flush();
    expect(ports.spoken.last, 'REPLY:Listo, desplegado.');
    expect(ports.reads, <String>['w:r1']);
    expect(m.phase, CarPhase.listening);
    expect(ports.shortAnswers.last, isFalse);
    expect(ports.drafts, isEmpty);
    expect(ports.sent, isEmpty);
  });

  test('a message: whole read-back, «¿Lo envío?», sent only on «sí»; then it waits quietly', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    expect(ports.spoken, <String>['Ningún split te espera. A, split 1.', 'Sin respuesta nueva.']);
    expect(m.phase, CarPhase.listening);
    ports.dictated(const DictationText('revisa el deploy'));
    await flush();
    expect(m.phase, CarPhase.confirming);
    expect(ports.spoken.last, 'Tu mensaje: «revisa el deploy». ¿Lo envío?');
    expect(ports.hearWindows.last, const Duration(seconds: 6), reason: 'listens for the answer');
    expect(m.hint, 'Di «sí» para confirmar o «no» para dejarlo');
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.sent, isEmpty, reason: 'silence is never a yes');
    ports.command('sí');
    await flush();
    expect(ports.sent, <(String, String?, String)>[('a', '%a1', 'revisa el deploy')]);
    expect(ports.spoken, containsAllInOrder(<String>['Enviado.', 'Nada más por ahora. Te aviso cuando haya algo.']));
    expect(m.phase, CarPhase.waiting);
    expect(ports.hearing, isNotNull, reason: 'the line stays open for commands');
  });

  test('"no" during the read-back: nothing sent, kept as a draft, and it listens again', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('borra la rama'));
    await flush();
    await clock.elapse(const Duration(seconds: 1));
    ports.command('No.');
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('a', 'borra la rama')]);
    expect(ports.spoken, contains('Cancelado. Quedó como borrador.'));
    expect(m.phase, CarPhase.listening, reason: 'say it again, or something else');
  });

  test('"envíalo" during the read-back sends at once; "cancela" works too', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('hola'));
    await flush();
    ports.command('envíalo');
    await flush();
    expect(ports.sent.single.$3, 'hola');

    // Back in a turn ("repite" re-opens it), then "cancela".
    ports.command('repite');
    await flush();
    ports.dictated(const DictationText('hola otra vez'));
    await flush();
    ports.command('cancela');
    await flush();
    await clock.elapse(const Duration(seconds: 5));
    expect(ports.sent, hasLength(1));
    expect(ports.drafts.single.$2, 'hola otra vez');
  });

  test('commands in a turn are never sent nor drafted; a longer sentence is a message', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.idle, ts: 1)]);
    await start(m, latest: const ReplyRef('r1', 'Hecho.'));
    expect(ports.spoken.last, 'REPLY:Hecho.');

    ports.dictated(const DictationText('Siguiente.'));
    await flush();
    expect(ports.spoken.last, 'B, split 1.');
    expect(m.currentId, 'b');
    m.updateCurrent(working: true);
    await flush();
    expect(ports.spoken.last, 'Trabajando.');

    ports.dictated(const DictationText('el anterior'));
    await flush();
    expect(m.currentId, 'a');
    m.updateCurrent(working: false, latest: const ReplyRef('r1', 'Hecho.'));
    await flush();
    expect(ports.spoken.last, 'Sin respuesta nueva.', reason: 'already read in this call');

    ports.dictated(const DictationText('repite'));
    await flush();
    expect(ports.spoken.last, 'REPLY:Hecho.');
    expect(m.phase, CarPhase.listening);

    expect(ports.sent, isEmpty);
    expect(ports.drafts, isEmpty);
    expect(ports.log.where((String x) => x == 'release'), hasLength(3));

    ports.dictated(const DictationText('sigue con el deploy'));
    await flush();
    expect(m.phase, CarPhase.confirming);
    expect(ports.spoken.last, 'Tu mensaje: «sigue con el deploy». ¿Lo envío?');
  });

  test('«siguiente» jumps to the next split that waits, skipping idle ones', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[
      sp('a', SessionState.idle, ts: 9),
      sp('b', SessionState.idle, ts: 8),
      sp('c', SessionState.ask, ts: 1),
    ]);
    await start(m);
    ports.dictated(const DictationText('siguiente'));
    await flush();
    expect(m.currentId, 'c', reason: 'B is idle: nothing to hear there');
  });

  test('silence after «¿Lo envío?» never sends: the message stays as a draft', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('borra producción'));
    await flush();
    ports.command(''); // the window closed with nobody speaking
    await flush();
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('a', 'borra producción')]);
    expect(ports.spoken, contains('No te oí confirmar. No lo envié; quedó como borrador.'));
  });

  test('anything but a yes after «¿Lo envío?» never sends either', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('arregla el login'));
    await flush();
    ports.command('mañana lo vemos');
    await flush();
    expect(ports.sent, isEmpty);
    expect(ports.drafts, <(String, String)>[('a', 'arregla el login')]);
    expect(ports.spoken, contains('No era un sí. No lo envié; quedó como borrador.'));
  });

  test('silence moves on to the next split that waits, then waits; a reply that arrives is read', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.ask, ts: 1)]);
    await start(m);
    expect(ports.spoken.first, 'Un split te espera. A, split 1.');
    ports.dictated(const DictationNothing());
    await flush();
    expect(ports.spoken.last, 'B, split 1.');
    m.updateCurrent(working: false, ask: ask);
    await flush();
    expect(ports.spoken.last, askCarText(ask));

    // Silence again: nobody else waits (B is not read again on its own).
    ports.dictated(const DictationNothing());
    await flush();
    expect(m.phase, CarPhase.waiting);
    expect(ports.spoken.last, 'Nada más por ahora. Te aviso cuando haya algo.');
    final int listens = ports.requireSpeech.length;
    for (int i = 0; i < 4; i++) {
      ports.command('');
      await flush();
    }
    expect(ports.hearing, isNull, reason: 'stops listening after a few rounds, still in the call');
    await clock.elapse(const Duration(minutes: 5));
    expect(ports.requireSpeech.length, listens);

    // The agent answers: read, then a turn.
    m.updateCurrent(working: false);
    m.replyCompleted(const ReplyRef('r9', 'Terminé.'));
    await flush();
    expect(ports.spoken.last, 'REPLY:Terminé.');
    expect(m.phase, CarPhase.listening);
  });

  test('waiting: another split that finishes is visited, without a tap', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.work, ts: 1)]);
    await start(m);
    ports.dictated(const DictationNothing());
    await flush();
    expect(m.phase, CarPhase.waiting);
    expect(ports.hearing, isNotNull);
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.idle, ts: 3)]);
    await flush();
    expect(ports.log, contains('stopHearing'));
    expect(ports.spoken.last, 'B, split 1.');
    expect(ports.spoken.where((String x) => x.contains('terminó')), isEmpty, reason: 'visited, not announced');
  });

  test('a command while waiting: «siguiente» and «mensaje»', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.work, ts: 1)]);
    await start(m);
    ports.dictated(const DictationNothing());
    await flush();
    ports.command('oye, mensaje');
    await flush();
    expect(m.phase, CarPhase.listening);
    expect(m.currentId, 'a');
    ports.dictated(const DictationNothing());
    await flush();
    ports.command('próximo');
    await flush();
    expect(ports.spoken.last, 'B, split 1.');
    expect(m.currentId, 'b');
    expect(ports.sent, isEmpty);
  });

  test('«espera» pauses (nothing is read); «sigue» resumes; it resumes on its own after 2 min', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('espera'));
    await flush();
    expect(m.paused, isTrue);
    expect(ports.spoken.last, 'En pausa. Di sigue para seguir.');
    m.replyCompleted(const ReplyRef('r1', 'Respuesta'));
    await flush();
    expect(ports.spoken.where((String x) => x.startsWith('REPLY:')), isEmpty);
    ports.command('sigue');
    await flush();
    expect(m.paused, isFalse);
    expect(ports.spoken, containsAllInOrder(<String>['Seguimos.', 'REPLY:Respuesta']));

    ports.dictated(const DictationText('un momento'));
    await flush();
    expect(m.paused, isTrue);
    await clock.elapse(const Duration(minutes: 2));
    expect(m.paused, isFalse);
    expect(ports.spoken.last, 'Nada más por ahora. Te aviso cuando haya algo.');
  });

  test('«cuelga» ends the call; a message in its countdown stays as a draft', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('a medias'));
    await flush();
    ports.command('cuelga');
    await flush();
    expect(m.ended, isTrue);
    expect(m.inCall, isFalse);
    expect(ports.spoken.last, 'Llamada terminada.');
    expect(ports.drafts, <(String, String)>[('a', 'a medias')]);
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.sent, isEmpty);
  });

  test('«para» in a turn hangs up too', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('Para.'));
    await flush();
    expect(m.ended, isTrue);
    expect(ports.drafts, isEmpty);
  });

  test('nothing can listen (no recognizer, no key): says why once and waits for a tap', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle, ts: 2), sp('b', SessionState.ask, ts: 1)]);
    await start(m);
    ports.dictated(const DictationDenied('Este teléfono no tiene reconocimiento de voz.'));
    await flush();
    expect(ports.spoken.last, 'Este teléfono no tiene reconocimiento de voz.');
    expect(m.phase, CarPhase.waiting);
    final int listens = ports.requireSpeech.length;
    m.updateCurrent(working: false);
    await clock.elapse(const Duration(minutes: 1));
    expect(ports.requireSpeech.length, listens, reason: 'no loop');
    unawaited(m.talk());
    await flush();
    expect(m.phase, CarPhase.listening);
  });

  test('hidden while confirming; back, the read-back and «¿Lo envío?» run again (no tap)', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.idle)]);
    await start(m);
    ports.dictated(const DictationText('despliega'));
    await flush();
    m.setForeground(false);
    await clock.elapse(const Duration(seconds: 30));
    expect(ports.sent, isEmpty);
    m.setForeground(true);
    await flush();
    expect(ports.spoken.where((String x) => x == 'Tu mensaje: «despliega». ¿Lo envío?'), hasLength(2));
    ports.command('sí');
    await flush();
    expect(ports.sent.single.$3, 'despliega');
  });

  test('a prompt answered by voice: "no" during its countdown cancels it', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.ask)]);
    await start(m, prompt: ask);
    ports.dictated(const DictationText('sí'));
    await flush();
    expect(m.phase, CarPhase.choosing);
    ports.command('no');
    await flush();
    await clock.elapse(const Duration(seconds: 10));
    expect(ports.answers, isEmpty);
    expect(ports.spoken, contains('Cancelado.'));
    expect(m.phase, CarPhase.listening, reason: 'asks for the answer again');
  });

  test('«siguiente» during a prompt leaves it unanswered and moves on', () async {
    final CarMachine m = machine(session: 'a');
    m.updateSessions(<CarSession>[sp('a', SessionState.ask, ts: 2), sp('b', SessionState.idle, ts: 1)]);
    await start(m, prompt: ask);
    ports.dictated(const DictationText('siguiente'));
    await flush();
    expect(ports.answers, isEmpty);
    expect(ports.spoken.last, 'B, split 1.');
  });

  test('splitsOf: one split per pane of paneList, else one per session', () {
    final List<CarSession> s = splitsOf(const <SessionSummary>[
      SessionSummary(
        session: 'p',
        label: 'Pagos',
        state: SessionState.ask,
        unread: true,
        ts: 9,
        paneList: <PaneSummary>[
          PaneSummary(pane: '%1', state: SessionState.idle, ts: 8, agent: 'claude', model: 'opus-5'),
          PaneSummary(pane: '%2', state: SessionState.ask, ts: 9, agent: 'codex', model: 'gpt-6'),
        ],
      ),
      SessionSummary(session: 'd', label: 'Docs', state: SessionState.work, unread: false, ts: 1, pane: '%5'),
    ]);
    expect(s.map((CarSession x) => '${x.session}${x.pane}:${x.split}/${x.splits}:${x.state.name}'),
        <String>['p%1:1/2:idle', 'p%2:2/2:ask', 'd%5:1/1:work']);
    expect(s[1].intro, 'Pagos, split 2, codex gpt 6.');
    expect(s[1].name, 'Pagos, split 2');
    expect(s[2].name, 'Docs');
  });
}
