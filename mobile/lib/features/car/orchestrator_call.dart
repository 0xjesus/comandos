import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/models/ask_prompt.dart';
import '../../core/models/transcript.dart';
import '../notifications/car_text.dart';
import '../voice/dictation.dart';
import 'car_commands.dart';
import 'car_ports.dart';
import 'option_parser.dart';
import 'wake_phrase.dart';

/// The orchestrator session as the call sees it.
class OrchSnapshot {
  const OrchSnapshot({required this.messages, this.ask, required this.loaded, this.busy = false});
  final List<ChatMessage> messages;
  final AskPrompt? ask;
  final bool loaded;

  /// The desktop sees the orchestrator working (`/session-feed` state
  /// `work`): an answer is not finished while this is true, even if its
  /// text stopped changing (a tool running, thinking between steps).
  final bool busy;
}

/// What the call needs from the server: open (or find) the orchestrator
/// session, watch its transcript and answer its own questions.
abstract class OrchestratorLink implements Listenable {
  /// `POST /auto/session`: (session, pane). Throws with a sayable message.
  Future<(String, String?)> open();

  OrchSnapshot get now;

  /// Asks for a fresh read soon.
  void poke();

  Future<String?> answer(AskPrompt ask, AskOption option);

  /// Esc in the orchestrator (stops what it is doing).
  Future<String?> interrupt();

  void close();
}

enum OrchPhase { connecting, sending, thinking, speaking, listening, asking, sleeping, idle, failed, ended }

/// Modo auto as a phone call with ONE Claude Code session (the orchestrator,
/// `dash/native/auto.rs`) that sees and drives the other sessions. The phone
/// only talks: what the person says goes to the orchestrator as a message,
/// what the orchestrator answers is read aloud, and it listens again. The
/// orchestrator asks for a «sí» before it sends or answers anything in
/// another session, so a misheard sentence never reaches a project.
///
/// Like «OK Google»: it opens asleep and only listens after «OK ComandOS»
/// (offline, never sent anywhere: [WakeListener]), a tap or the steering
/// wheel. After it speaks it listens for a follow-up; when [quiet] (10 s)
/// passes without the person's voice it sleeps again.
///
/// Commands that stay on the phone (whole utterance): «cuelga», «repite»,
/// «espera» (sleeps now). Steering wheel: play / next = talk (or interrupt
/// the reading), hang-up = hang up.
class OrchestratorCall extends ChangeNotifier {
  OrchestratorCall({
    required CarPorts ports,
    required OrchestratorLink link,
    WakeListener? wake,
    Duration? settle,
    Duration? patience,
    Duration? quiet,
    DateTime Function()? clock,
  })  : _ports = ports,
        _link = link,
        _wake = wake,
        _settle = settle ?? const Duration(milliseconds: 3200),
        _patience = patience ?? const Duration(minutes: 20),
        _quiet = quiet ?? const Duration(seconds: 10),
        _clock = clock ?? DateTime.now;

  final CarPorts _ports;
  final OrchestratorLink _link;
  final WakeListener? _wake;

  /// Silence that puts it to sleep.
  final Duration _quiet;
  final DateTime Function() _clock;

  static const String sleepingStatus = 'Di «OK ComandOS»';

  /// A reply is complete when the last message is the orchestrator's and
  /// nothing changed for this long.
  final Duration _settle;

  /// Longest wait for a reply.
  final Duration _patience;

  OrchPhase _phase = OrchPhase.connecting;
  OrchPhase get phase => _phase;
  String _status = 'Conectando…';
  String get status => _status;

  /// What the person said last and what the orchestrator answered last
  /// (shown small; the call is meant to be heard).
  String heard = '';
  String said = '';

  String? _session;
  String? _pane;
  String? get session => _session;

  bool _ended = false;
  bool get ended => _ended;
  bool _disposed = false;
  bool _foreground = true;

  /// Bumped by anything that changes the plan (talk, hang up): a running
  /// step that sees another generation stops.
  int _gen = 0;

  /// The wake phrase cannot be heard on this phone: sleeping waits for a tap.
  bool _noWake = false;

  /// «OK ComandOS» does not work here: only a tap or the steering wheel wake it.
  bool get noWake => _wake == null || _noWake;

  void _set(OrchPhase p, String status) {
    _phase = p;
    _status = status;
    if (!_disposed) notifyListeners();
  }

  void setForeground(bool on) => _foreground = on;

  Future<void> start() async {
    _set(OrchPhase.connecting, 'Conectando con el orquestador…');
    try {
      final (String s, String? p) = await _link.open();
      _session = s;
      _pane = p;
    } catch (e) {
      final String why = e.toString().replaceFirst('Exception: ', '');
      _set(OrchPhase.failed, why);
      await _ports.speak('No pude abrir el orquestador. $why');
      return;
    }
    if (_ended) return;
    await _loaded();
    if (_ended) return;
    if (_wake != null) await _ports.speak('Listo. Di OK ComandOS cuando me necesites.');
    if (!_ended) await _sleep();
  }

  Future<void> _loaded() async {
    final DateTime until = DateTime.now().add(const Duration(seconds: 12));
    while (!_link.now.loaded && DateTime.now().isBefore(until) && !_ended) {
      _link.poke();
      await _changed(const Duration(milliseconds: 600));
    }
  }

  /// Waits for the link to change (or [max]).
  Future<void> _changed(Duration max) {
    final Completer<void> c = Completer<void>();
    void on() {
      if (!c.isCompleted) c.complete();
    }

    _link.addListener(on);
    return c.future.timeout(max, onTimeout: () {}).whenComplete(() => _link.removeListener(on));
  }

  /// Sends [text] to the orchestrator, reads its answer, listens again.
  Future<void> _turn(String text) async {
    final int gen = ++_gen;
    final String? s = _session;
    if (s == null) return;
    await _wake?.stop();
    heard = text;
    _set(OrchPhase.sending, 'Enviando…');
    final List<ChatMessage> before = _link.now.messages;
    final int? base = before.isEmpty ? null : before.last.hash;
    final SendOutcome out = await _ports.send(s, _pane, text);
    if (gen != _gen || _ended) return;
    if (out.kind != SendKind.delivered && out.kind != SendKind.queuedOffline) {
      _set(OrchPhase.failed, out.message.isEmpty ? 'No se envió' : out.message);
      await _ports.speak('No le llegó al orquestador. ${out.message}');
      if (gen == _gen && !_ended) unawaited(_listen());
      return;
    }
    _set(OrchPhase.thinking, 'Pensando…');
    final String? reply = await _reply(base, text, gen);
    if (gen != _gen || _ended) return;
    final AskPrompt? ask = _link.now.ask;
    if (ask != null) {
      await _answerOwnQuestion(ask, gen);
      return;
    }
    if (reply == null || reply.trim().isEmpty) {
      await _ports.speak('El orquestador no contestó. ¿Lo intento de nuevo?');
    } else {
      said = reply;
      _set(OrchPhase.speaking, 'Hablando…');
      await _ports.speakReply(reply);
    }
    if (gen == _gen && !_ended) await _listen();
  }

  /// What the orchestrator wrote after [sent] (the person's message, found
  /// after the last message there was before sending, [base]): null until
  /// it is there. The chat keeps a window of the last messages, so this
  /// goes by content, never by position.
  static List<ChatMessage>? answerTo(List<ChatMessage> ms, int? base, String sent) {
    final String want = sent.trim();
    int u = -1;
    for (int i = ms.length - 1; i >= 0; i--) {
      if (ms[i].role == ChatRole.user && ms[i].text.trim() == want) {
        u = i;
        break;
      }
    }
    if (u < 0) return null;
    // Ours only if what was last before sending comes before it (or has
    // left the window): otherwise it is an older identical message.
    if (base != null && ms.any((ChatMessage m) => m.hash == base)) {
      bool before = false;
      for (int j = 0; j < u; j++) {
        if (ms[j].hash == base) before = true;
      }
      if (!before) return null;
    }
    return ms.sublist(u + 1);
  }

  /// The orchestrator's answer to [sent], once it stopped writing.
  Future<String?> _reply(int? base, String sent, int gen) async {
    final DateTime until = DateTime.now().add(_patience);
    DateTime stableSince = DateTime.now();
    int seen = -1;
    bool told = false;
    final DateTime started = DateTime.now();
    while (gen == _gen && !_ended && DateTime.now().isBefore(until)) {
      _link.poke();
      await _changed(const Duration(milliseconds: 800));
      final OrchSnapshot now = _link.now;
      if (now.ask != null) return null;
      final List<ChatMessage>? fresh = answerTo(now.messages, base, sent);
      final int count = fresh?.length ?? -1;
      if (count != seen) {
        seen = count;
        stableSince = DateTime.now();
      }
      if (fresh != null &&
          fresh.isNotEmpty &&
          fresh.last.role == ChatRole.assistant &&
          !now.busy &&
          DateTime.now().difference(stableSince) >= _settle) {
        return fresh.where((ChatMessage m) => m.role == ChatRole.assistant).map((ChatMessage m) => m.text).join('\n\n');
      }
      if (!told && DateTime.now().difference(started) > const Duration(seconds: 15) && _foreground) {
        told = true;
        await _ports.speak('Sigo en eso.');
      }
    }
    return null;
  }

  /// The orchestrator itself asks (a permission it does not have): read
  /// it, hear a number, confirm, answer.
  Future<void> _answerOwnQuestion(AskPrompt ask, int gen) async {
    _set(OrchPhase.asking, 'Te pregunta');
    final List<AskOption> options = ask.choices;
    for (int attempt = 0; attempt < 2 && gen == _gen && !_ended; attempt++) {
      await _ports.speak(attempt == 0 ? '${askCarText(ask)} ¿Cuál opción?' : '¿Qué opción? Di el número.');
      final String? h = await _ports.hear(const Duration(seconds: 8));
      if (gen != _gen || _ended) return;
      final AskOption? o = h == null ? null : parseOptionChoice(h, options);
      if (o == null) continue;
      await _ports.speak('Opción ${optionWord(o.key)}: ${speakableLabel(o.label)}. ¿Confirmo?');
      final String? yes = await _ports.hear(const Duration(seconds: 6));
      if (gen != _gen || _ended) return;
      final CarCommand? c = yes == null ? null : parseCarCommand(yes);
      if (c == CarCommand.send) {
        final String? why = await _link.answer(ask, o);
        await _ports.speak(why ?? 'Listo.');
        if (why == null) {
          _set(OrchPhase.thinking, 'Pensando…');
          final String? reply = await _afterAnswer(gen);
          if (reply != null && reply.trim().isNotEmpty && gen == _gen && !_ended) {
            said = reply;
            _set(OrchPhase.speaking, 'Hablando…');
            await _ports.speakReply(reply);
          }
        }
        break;
      }
      await _ports.speak('No respondí.');
      break;
    }
    if (gen == _gen && !_ended) await _listen();
  }

  /// After answering its question: whatever the orchestrator writes next.
  Future<String?> _afterAnswer(int gen) async {
    final List<ChatMessage> before = _link.now.messages;
    final int? base = before.isEmpty ? null : before.last.hash;
    final DateTime until = DateTime.now().add(_patience);
    DateTime stableSince = DateTime.now();
    int seen = -1;
    while (gen == _gen && !_ended && DateTime.now().isBefore(until)) {
      _link.poke();
      await _changed(const Duration(milliseconds: 800));
      final OrchSnapshot now = _link.now;
      final List<ChatMessage> ms = now.messages;
      int from = ms.length;
      for (int i = ms.length - 1; i >= 0; i--) {
        if (ms[i].hash == base) {
          from = i + 1;
          break;
        }
      }
      final List<ChatMessage> fresh = ms.sublist(from);
      if (fresh.length != seen) {
        seen = fresh.length;
        stableSince = DateTime.now();
      }
      if (fresh.isNotEmpty &&
          fresh.last.role == ChatRole.assistant &&
          !now.busy &&
          DateTime.now().difference(stableSince) >= _settle) {
        return fresh.where((ChatMessage m) => m.role == ChatRole.assistant).map((ChatMessage m) => m.text).join('\n\n');
      }
    }
    return null;
  }

  /// The person's turn. [since]: when the quiet window started (a repeated
  /// listen within it plays no cue).
  Future<void> _listen({DateTime? since}) async {
    final int gen = _gen;
    final String? s = _session;
    if (s == null || _ended) return;
    if (!_foreground) {
      _set(OrchPhase.idle, 'En pausa');
      return;
    }
    _set(OrchPhase.listening, 'Te escucho');
    if (since == null) {
      await _ports.cue();
      if (gen != _gen || _ended) return;
    }
    since ??= _clock();
    final DictationOutcome o = await _ports.listen(s, onTranscribing: () => _set(OrchPhase.thinking, 'Entendiendo…'), requireSpeech: true);
    if (gen != _gen || _ended) return;
    switch (o) {
      case DictationText(:final String text):
        final CarCommand? c = parseCarCommand(text);
        if (c == CarCommand.hangUp) {
          _ports.releaseDictation();
          await hangUp();
          return;
        }
        if (c == CarCommand.repeat) {
          _ports.releaseDictation();
          if (said.isNotEmpty) {
            _set(OrchPhase.speaking, 'Hablando…');
            await _ports.speakReply(said);
          }
          if (gen == _gen && !_ended) await _listen();
          return;
        }
        if (c == CarCommand.pause) {
          _ports.releaseDictation();
          await _ports.speak(_wake == null || _noWake
              ? 'Va, aquí espero. Toca Hablar o el botón del volante.'
              : 'Va. Dime OK ComandOS cuando me necesites.');
          if (gen == _gen && !_ended) await _sleep();
          return;
        }
        await _turn(text);
      case DictationNothing():
        // Still within the quiet window: keep listening, without a new cue.
        if (_clock().difference(since) < _quiet) {
          await _listen(since: since);
        } else {
          await _sleep();
        }
      case DictationFailed(:final PendingTake take):
        await _ports.discard(take.id);
        await _ports.speak('No te entendí. Otra vez.');
        if (gen == _gen && !_ended) await _listen();
      case DictationCancelled():
        await _sleep();
      case DictationDenied(:final String message):
        _set(OrchPhase.failed, message);
        await _ports.speak(message);
    }
  }

  /// Asleep until «OK ComandOS» (or a tap / the steering wheel: [talk]).
  Future<void> _sleep() async {
    final int gen = _gen;
    if (_ended) return;
    final WakeListener? w = _wake;
    if (w == null || _noWake || !_foreground) {
      _set(OrchPhase.idle, 'En espera');
      return;
    }
    _set(OrchPhase.sleeping, sleepingStatus);
    final WakeResult r = await w.wait();
    if (gen != _gen || _ended) return;
    switch (r) {
      case WakeResult.heard:
        // The listener still has the microphone: the turn starts now.
        _ports.takeAfterWake(w.heard);
        await _listen();
      case WakeResult.stopped:
        _set(OrchPhase.idle, 'En espera');
      case WakeResult.unavailable:
        _noWake = true;
        _set(OrchPhase.idle, 'En espera');
        await _ports.speak('${w.problem ?? 'No puedo escuchar OK ComandOS.'} Toca Hablar o el botón del volante.');
    }
  }

  /// Button, steering wheel or screen: talk now (stops any reading).
  Future<void> talk() async {
    if (_ended) return;
    if (_phase == OrchPhase.listening) {
      _ports.stopListening();
      return;
    }
    _gen++;
    _ports.stopSpeaking();
    _ports.stopHearing();
    await _wake?.stop();
    await _listen();
  }

  /// Stops the reading; sleeps until «OK ComandOS».
  void hush() {
    _gen++;
    _ports.stopSpeaking();
    unawaited(_sleep());
  }

  /// Esc in the orchestrator.
  Future<void> stopIt() async {
    final String? why = await _link.interrupt();
    if (why != null) await _ports.speak(why);
  }

  Future<void> hangUp() async {
    if (_ended) return;
    _gen++;
    _ended = true;
    _ports.stopSpeaking();
    _ports.cancelListening();
    _ports.stopHearing();
    unawaited(_wake?.stop());
    _set(OrchPhase.ended, 'Llamada terminada');
  }

  /// Steering wheel / headset.
  Future<void> wheel(CarButton b) => talk();

  @override
  void dispose() {
    _disposed = true;
    _gen++;
    _ended = true;
    _ports.stopSpeaking();
    _ports.cancelListening();
    unawaited(_wake?.stop());
    _link.close();
    super.dispose();
  }
}
