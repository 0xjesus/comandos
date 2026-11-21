import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/models/ask_prompt.dart';
import '../../core/models/session_feed.dart';
import '../../core/util/clock.dart';
import '../voice/dictation.dart';
import '../voice/reply_watcher.dart';
import 'option_parser.dart';

/// One session as Modo auto sees it.
@immutable
class CarSession {
  const CarSession({
    required this.session,
    required this.label,
    required this.state,
    this.ts = 0,
    this.pane,
    this.reply = '',
  });

  final String session;
  final String label;
  final SessionState state;
  final double ts;
  final String? pane;
  final String reply;

  String get stateText {
    switch (state) {
      case SessionState.ask:
        return 'te espera';
      case SessionState.limit:
        return 'llegó al límite';
      case SessionState.work:
        return 'trabajando';
      case SessionState.idle:
        return 'lista';
    }
  }
}

/// "Te espera" first, then working, then the rest; newest first in each.
List<CarSession> orderForCar(List<CarSession> sessions) {
  int rank(CarSession s) => s.state.waitsForHuman ? 0 : (s.state == SessionState.work ? 1 : 2);
  return <CarSession>[...sessions]..sort((CarSession a, CarSession b) {
      final int r = rank(a).compareTo(rank(b));
      return r != 0 ? r : b.ts.compareTo(a.ts);
    });
}

enum SendKind {
  delivered,
  queuedOffline,
  failed,
  rejected,

  /// An earlier message of that session is failed / uncertain: this one
  /// waits behind it until the person acts in the chat.
  blocked,

  /// The desktop refused because a permission prompt is open; the outbox
  /// delivers it once the prompt is answered.
  waitingPrompt,
}

class SendOutcome {
  const SendOutcome(this.kind, [this.message = '']);
  final SendKind kind;
  final String message;
}

/// Everything Modo auto does to the outside world. The live version wires
/// dictation, speech, the outbox and the chat; tests use a fake.
abstract class CarPorts {
  /// Conversación continua.
  bool get continuous;

  /// Records (auto-stops on silence) and transcribes for [session].
  /// [onTranscribing] fires when recording ends and transcription starts.
  /// [requireSpeech]: a recording in which no speech was detected is
  /// dropped without transcribing ([DictationNothing]).
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false});
  void stopListening();
  void cancelListening();
  Future<DictationOutcome> retry(String takeId);
  Future<void> discard(String takeId);

  /// The last dictated text was consumed as a spoken answer (or a failed
  /// attempt at one): let its recording go, without keeping it as a draft.
  void releaseDictation();

  /// Speaks plain text. True only when it played to the end.
  Future<bool> speak(String text);

  /// Reads a reply (markdown) with the configured read mode.
  Future<bool> speakReply(String markdown);
  void stopSpeaking();

  /// Through the outbox (durable); reports how delivery went.
  Future<SendOutcome> send(String session, String? pane, String text);

  /// Keeps dictated text that will not be sent (as the session's draft).
  Future<void> saveDraft(String session, String text);

  /// Answers the prompt whose [promptSig] the choice was made for, in
  /// [pane]. Returns an error (and sends nothing) if the chat now shows
  /// another session, pane or prompt.
  Future<String?> answer(String session, String optionKey, {String? pane, required String promptSig});
  Future<String?> interrupt(String session);
  void markRead(String session, String replyId);

  /// Earcon + haptic: "your turn". Completes when the earcon ended.
  Future<void> cue();
}

enum CarPhase { idle, listening, transcribing, failed, confirming, sending, reading, choosing }

/// What a spoken or tapped choice was made for. An answer is only ever
/// sent while all of it still holds.
@immutable
class _Choice {
  const _Choice({required this.session, required this.pane, required this.sig, required this.option});
  final String session;
  final String? pane;
  final String sig;
  final AskOption option;
}

/// Modo auto as an explicit state machine. Pure Dart: the clock, timers and
/// [CarPorts] are injected. The screen only forwards taps and feed updates
/// and renders the public getters.
///
/// Safety rules (the person is driving):
/// * A countdown only starts after its read-back was actually heard; if
///   speaking fails or is cut, the machine waits for a tap.
/// * A spoken answer goes only to the prompt that was on screen AND read
///   aloud before listening started, in the same session and pane; if any
///   of that changed, nothing is sent and the new prompt is read.
/// * While a prompt is open, a normal message is never sent: it becomes a
///   draft and the prompt is read.
/// * In the background nothing speaks, listens or counts down.
class CarMachine extends ChangeNotifier {
  CarMachine({
    required CarPorts ports,
    String? session,
    NowMs now = systemNowMs,
    TimerFactory timer = systemTimer,
    this.countdown = 3,
    this.announceGap = const Duration(seconds: 45),
    this.announceTtl = const Duration(minutes: 2),
  })  : _ports = ports,
        _currentId = session,
        _now = now,
        _timer = timer;

  final CarPorts _ports;
  final NowMs _now;
  final TimerFactory _timer;
  final int countdown;
  final Duration announceGap;
  final Duration announceTtl;

  /// Empty automatic rounds (continuous conversation) before the loop
  /// stops and waits for the Hablar button.
  static const int maxEmptyRounds = 2;

  CarPhase _phase = CarPhase.idle;
  List<CarSession> _ordered = const <CarSession>[];
  String? _currentId;
  final Map<String, SessionState> _states = <String, SessionState>{};
  bool _sessionsSeen = false;

  bool _working = false;
  String? _pane;
  AskPrompt? _ask;
  String? _askSig;
  bool _askUnread = false;

  /// Signature of the prompt that was last read aloud to the end.
  String? _askReadSig;
  String _excerpt = '';

  String? _heard;
  String? _target;
  String? _targetPane;

  /// The prompt the current dictation may answer (on screen and read
  /// before listening started), else null.
  String? _listenAskSig;
  bool _autoRound = false;
  int _emptyRounds = 0;
  bool _loopOff = false;
  int _count = 0;
  bool _needsTap = false;
  AskOption? _option;
  _Choice? _choice;
  String? _failedTake;
  String? _notice;
  int _reasks = 0;
  bool _foreground = true;

  /// Bumped by every action that supersedes what was in progress; async
  /// continuations compare it and give up when it changed.
  int _op = 0;
  bool _speaking = false;
  ReplyRef? _pendingReply;
  final Set<String> _readIds = <String>{};
  final List<(String, int)> _announcements = <(String, int)>[];
  final Map<String, int> _lastAnnounced = <String, int>{};
  final Set<Timer> _timers = <Timer>{};
  bool _disposed = false;

  // ---- What the screen shows -------------------------------------------------

  CarPhase get phase => _phase;
  List<CarSession> get sessions => _ordered;
  String? get currentId => _currentId;
  CarSession? get current {
    for (final CarSession s in _ordered) {
      if (s.session == _currentId) return s;
    }
    return null;
  }

  bool get working => _working;
  AskPrompt? get ask => _ask;
  String get excerpt => _excerpt;

  /// Recognised text waiting for the countdown (or a tap).
  String? get heard => _heard;

  /// Seconds left in a countdown.
  int get count => _count;
  AskOption? get pendingOption => _option;
  bool get speaking => _speaking;

  /// Confirming / choosing with no countdown: only a tap sends.
  bool get needsTap => _needsTap;

  /// The app is in the foreground (Modo auto works only then).
  bool get foreground => _foreground;

  /// Last outcome to show ("Enviado", errors…).
  String? get notice => _notice;

  bool get canSwitch => _phase == CarPhase.idle || _phase == CarPhase.reading || _phase == CarPhase.failed;

  String get status {
    switch (_phase) {
      case CarPhase.listening:
        return 'Escuchando…';
      case CarPhase.transcribing:
        return 'Transcribiendo…';
      case CarPhase.failed:
        return 'No se pudo transcribir';
      case CarPhase.confirming:
        return _needsTap ? 'Toca Enviar o Cancelar' : 'Envío en $_count…';
      case CarPhase.sending:
        return 'Enviando…';
      case CarPhase.reading:
        return 'Leyendo respuesta…';
      case CarPhase.choosing:
        final String w = optionWord(_option?.key ?? '');
        return _needsTap ? 'Opción $w: toca Confirmar' : 'Opción $w en $_count…';
      case CarPhase.idle:
        if (_ask != null) return 'Te espera: elige una opción';
        return _working ? 'Trabajando…' : 'Listo';
    }
  }

  // ---- Feed ------------------------------------------------------------------

  /// New `/session-feed` snapshot. Announces other sessions that finished
  /// or started waiting (short, rate-limited, never over a dictation).
  ///
  /// When the current session is missing from a snapshot, the machine only
  /// moves to another one while nothing is in progress (idle / reading /
  /// failed): a dictation, countdown or answer stays with its session.
  void updateSessions(List<CarSession> sessions) {
    if (_disposed) return;
    _ordered = orderForCar(sessions);
    final bool present = _ordered.any((CarSession s) => s.session == _currentId);
    if (_currentId == null && _ordered.isNotEmpty) {
      _currentId = _ordered.first.session;
    } else if (!present && _ordered.isNotEmpty && canSwitch) {
      _supersede();
      _adopt(_ordered.first);
    }
    final int now = _now();
    for (final CarSession s in sessions) {
      final SessionState? before = _states[s.session];
      _states[s.session] = s.state;
      if (!_sessionsSeen || before == null || before == s.state || s.session == _currentId) continue;
      String? text;
      if (s.state == SessionState.ask) {
        text = '${s.label} te espera: pide permiso.';
      } else if (s.state == SessionState.limit) {
        text = '${s.label} llegó al límite de uso.';
      } else if (before == SessionState.work && s.state == SessionState.idle) {
        text = '${s.label} terminó.';
      }
      if (text == null) continue;
      final int? last = _lastAnnounced[s.session];
      if (last != null && now - last < announceGap.inMilliseconds) continue;
      _lastAnnounced[s.session] = now;
      _announcements.add((text, now));
      if (_announcements.length > 3) _announcements.removeAt(0);
    }
    _sessionsSeen = true;
    notifyListeners();
    _drain();
  }

  /// The current session's chat: working flag, permission prompt, excerpt
  /// and the pane the chat reads (where messages and answers go).
  void updateCurrent({required bool working, AskPrompt? ask, String? excerpt, String? pane}) {
    if (_disposed) return;
    _working = working;
    if (excerpt != null) _excerpt = excerpt;
    if (pane != null) _pane = pane;
    final String? sig = ask?.signature;
    if (sig != _askSig) {
      _askSig = sig;
      _ask = ask;
      _askReadSig = null;
      _reasks = 0;
      if (ask != null) _askUnread = true;
    }
    final _Choice? c = _choice;
    if (_phase == CarPhase.choosing && c != null && (c.sig != _askSig || c.pane != _pane)) {
      // The prompt (or pane) the choice was made for is gone: never carry
      // the choice over. The new prompt, if any, is read next.
      _supersede();
      _option = null;
      _choice = null;
      _needsTap = false;
      _phase = CarPhase.idle;
      _notice = 'La pregunta cambió; no respondí.';
    } else if (_phase == CarPhase.confirming && _ask != null) {
      // A prompt appeared before the message went: keep the text as a draft.
      _supersede();
      _parkAsDraft();
      notifyListeners();
      unawaited(_announceThenDrain('Hay un permiso abierto. Tu mensaje quedó como borrador.'));
      return;
    }
    notifyListeners();
    _drain();
  }

  /// A reply of the current session completed. Read once, when free.
  void replyCompleted(ReplyRef reply) {
    if (_disposed || _readIds.contains(reply.id)) return;
    _pendingReply = reply;
    _drain();
  }

  /// The app went to the background (false) or came back (true). Hidden,
  /// Modo auto stops speaking and recording (the recording is still
  /// transcribed) and cancels any countdown: what is pending waits for a
  /// tap. Back in the foreground it picks up what is queued.
  void setForeground(bool foreground) {
    if (_disposed || _foreground == foreground) return;
    _foreground = foreground;
    if (!foreground) {
      _supersede();
      if (_phase == CarPhase.listening) _ports.stopListening();
      if (_phase == CarPhase.reading) _phase = CarPhase.idle;
      if (_phase == CarPhase.confirming || _phase == CarPhase.choosing) _needsTap = true;
      notifyListeners();
    } else {
      notifyListeners();
      _drain();
    }
  }

  // ---- Buttons ---------------------------------------------------------------

  /// The giant button: Hablar / Terminar / Reintentar.
  Future<void> talk() async {
    switch (_phase) {
      case CarPhase.idle:
      case CarPhase.reading:
        _loopOff = false;
        _emptyRounds = 0;
        await _listen(auto: false);
      case CarPhase.listening:
        _ports.stopListening();
      case CarPhase.failed:
        await _retry();
      case CarPhase.transcribing:
      case CarPhase.confirming:
      case CarPhase.sending:
      case CarPhase.choosing:
        return;
    }
  }

  /// Cancelar: dictation, the send countdown or the option countdown.
  void cancel() {
    switch (_phase) {
      case CarPhase.listening:
      case CarPhase.transcribing:
        _ports.cancelListening();
      case CarPhase.confirming:
        final String? text = _heard;
        final String? target = _target;
        _supersede();
        _heard = null;
        _needsTap = false;
        _phase = CarPhase.idle;
        _notice = 'Cancelado. Quedó como borrador.';
        notifyListeners();
        if (text != null && target != null) unawaited(_ports.saveDraft(target, text));
        unawaited(_announceThenDrain('Cancelado. Quedó como borrador.'));
      case CarPhase.choosing:
        _supersede();
        _option = null;
        _choice = null;
        _needsTap = false;
        _phase = CarPhase.idle;
        _notice = 'Cancelado.';
        notifyListeners();
        unawaited(_announceThenDrain('Cancelado.'));
      case CarPhase.idle:
      case CarPhase.failed:
      case CarPhase.sending:
      case CarPhase.reading:
        return;
    }
  }

  /// Enviar ya / Enviar (skip the countdown, or the tap it waits for).
  Future<void> sendNow() async {
    if (_phase != CarPhase.confirming) return;
    _supersede();
    await _send(_op);
  }

  /// Confirmar: the voice-chosen option, skipping the countdown (or the tap
  /// it waits for). Still refused if the prompt changed meanwhile.
  Future<void> confirmChoice() async {
    if (_phase != CarPhase.choosing || _choice == null) return;
    _supersede();
    await _answerChoice(_op);
  }

  /// Descartar a recording that could not be transcribed.
  Future<void> discard() async {
    if (_phase != CarPhase.failed) return;
    final String? take = _failedTake;
    _failedTake = null;
    _phase = CarPhase.idle;
    _notice = 'Grabación descartada.';
    notifyListeners();
    if (take != null) await _ports.discard(take);
    _drain();
  }

  /// Callar. A countdown whose read-back is cut waits for a tap.
  void hush() {
    _supersede();
    _announcements.clear();
    if (_phase == CarPhase.reading) _phase = CarPhase.idle;
    if (_phase == CarPhase.confirming || _phase == CarPhase.choosing) _needsTap = true;
    notifyListeners();
  }

  /// Detener: Escape to the agent.
  Future<void> stopAgent() async {
    final String? s = _currentId;
    if (s == null) return;
    hush();
    final String? err = await _ports.interrupt(s);
    if (_disposed) return;
    _notice = err ?? 'Enviado Escape: el agente debería detenerse.';
    notifyListeners();
    await _announceThenDrain(err == null ? 'Detenido.' : 'No se pudo detener.');
  }

  /// A permission option chosen by button: unambiguous, acts at once, on
  /// the prompt shown.
  Future<void> chooseOption(AskOption o) async {
    final String? s = _currentId;
    final String? sig = _askSig;
    if (_ask == null || s == null || sig == null || _phase == CarPhase.sending) return;
    _supersede();
    await _answer(_Choice(session: s, pane: _pane, sig: sig, option: o), _op);
  }

  void next() => _switch(1);
  void prev() => _switch(-1);

  // ---- Internals ---------------------------------------------------------------

  void _supersede() {
    _op++;
    _cancelTimers();
    _ports.stopSpeaking();
    _speaking = false;
  }

  /// Makes [to] the current session, forgetting everything of the old one.
  void _adopt(CarSession to) {
    _currentId = to.session;
    _phase = CarPhase.idle;
    _failedTake = null; // it stays pending in that session's chat
    _pendingReply = null;
    _ask = null;
    _askSig = null;
    _askReadSig = null;
    _askUnread = false;
    _option = null;
    _choice = null;
    _needsTap = false;
    _pane = to.pane;
    _working = to.state == SessionState.work;
    _excerpt = to.reply;
    _notice = null;
  }

  void _switch(int delta) {
    if (!canSwitch || _ordered.length < 2) return;
    final int i = _ordered.indexWhere((CarSession s) => s.session == _currentId);
    final CarSession to = _ordered[(i + delta + _ordered.length) % _ordered.length];
    _supersede();
    _adopt(to);
    notifyListeners();
    unawaited(_announceThenDrain('${to.label}, ${to.stateText}.'));
  }

  /// Speaks; true only when it was heard to the end (and nothing else took
  /// over meanwhile). Never speaks in the background.
  Future<bool> _speakIn(int op, Future<bool> Function() speech) async {
    if (!_foreground || _disposed) return false;
    _speaking = true;
    notifyListeners();
    bool done = false;
    try {
      done = await speech();
    } catch (_) {
      done = false;
    }
    if (op == _op && !_disposed) {
      _speaking = false;
      notifyListeners();
    }
    return done && op == _op && !_disposed && _foreground;
  }

  Future<void> _announceThenDrain(String text) async {
    final int op = ++_op;
    await _speakIn(op, () => _ports.speak(text));
    if (op == _op) _drain();
  }

  Future<void> _wait(Duration d) {
    final Completer<void> c = Completer<void>();
    late final Timer t;
    t = _timer(d, () {
      _timers.remove(t);
      c.complete();
    });
    _timers.add(t);
    return c.future;
  }

  void _cancelTimers() {
    for (final Timer t in _timers) {
      t.cancel();
    }
    _timers.clear();
  }

  /// Runs the next queued thing when idle, silent and in the foreground:
  /// a reply, a prompt, an announcement.
  void _drain() {
    if (_disposed || !_foreground || _phase != CarPhase.idle || _speaking) return;
    final ReplyRef? reply = _pendingReply;
    if (reply != null) {
      _pendingReply = null;
      unawaited(_read(reply));
      return;
    }
    if (_askUnread && _ask != null) {
      _askUnread = false;
      unawaited(_readAsk(_ask!));
      return;
    }
    final int now = _now();
    _announcements.removeWhere(((String, int) a) => now - a.$2 > announceTtl.inMilliseconds);
    if (_announcements.isNotEmpty) {
      final (String text, int _) = _announcements.removeAt(0);
      unawaited(_announceThenDrain(text));
    }
  }

  /// Starts a dictation. [auto]: started by the continuous loop or a
  /// re-ask, not by a tap (a silent recording is then dropped).
  Future<void> _listen({required bool auto}) async {
    final String? s = _currentId;
    if (s == null || !_foreground || _disposed) return;
    _supersede();
    _phase = CarPhase.listening;
    _target = s;
    _targetPane = _pane ?? current?.pane;
    // Speech may answer a prompt only if it was on screen and read aloud
    // before listening started.
    _listenAskSig = _ask != null && _askSig != null && _askReadSig == _askSig ? _askSig : null;
    _autoRound = auto;
    _heard = null;
    _notice = null;
    notifyListeners();
    final DictationOutcome o = await _ports.listen(s, requireSpeech: auto, onTranscribing: () {
      if (_phase == CarPhase.listening && !_disposed) {
        _phase = CarPhase.transcribing;
        notifyListeners();
      }
    });
    await _onDictation(o);
  }

  Future<void> _retry() async {
    final String? take = _failedTake;
    if (take == null) {
      _phase = CarPhase.idle;
      notifyListeners();
      return;
    }
    _supersede();
    _failedTake = null;
    _phase = CarPhase.transcribing;
    _notice = null;
    _autoRound = false;
    notifyListeners();
    await _onDictation(await _ports.retry(take));
  }

  Future<void> _onDictation(DictationOutcome o) async {
    if (_disposed) return;
    final String? askFor = _listenAskSig;
    _listenAskSig = null;
    switch (o) {
      case DictationText(:final String text):
        _emptyRounds = 0;
        final bool sameSession = _target == _currentId;
        if (askFor != null && sameSession && _ask != null && askFor == _askSig) {
          await _answerBySpeech(text);
        } else if (askFor != null) {
          // Meant as an answer, but the prompt changed or went away: drop
          // it (never a message, never applied to another prompt).
          _ports.releaseDictation();
          _phase = CarPhase.idle;
          _notice = 'La pregunta cambió; no respondí.';
          if (_ask != null) _askUnread = true;
          notifyListeners();
          await _announceThenDrain('La pregunta cambió. No respondí.');
        } else if (sameSession && _ask != null) {
          // A prompt is open: a message could act on it. Keep it as a
          // draft and read the prompt instead.
          _heard = text;
          _parkAsDraft();
          notifyListeners();
          await _announceThenDrain('Hay un permiso abierto. Tu mensaje quedó como borrador.');
        } else {
          await _confirm(text);
        }
      case DictationNothing():
        _phase = CarPhase.idle;
        _notice = 'No se oyó nada.';
        if (_autoRound && ++_emptyRounds >= maxEmptyRounds) {
          _loopOff = true;
          _notice = 'No se oyó nada. Toca Hablar para seguir.';
        }
        notifyListeners();
        await _announceThenDrain('No se oyó nada.');
      case DictationFailed(:final PendingTake take, :final String message):
        _phase = CarPhase.failed;
        _failedTake = take.id;
        _notice = message;
        notifyListeners();
        final int op = ++_op;
        await _speakIn(op, () => _ports.speak('No se pudo transcribir. La grabación está guardada: toca reintentar.'));
      case DictationCancelled():
        _phase = CarPhase.idle;
        notifyListeners();
        _drain();
      case DictationDenied(:final String message):
        _phase = CarPhase.idle;
        _notice = message;
        notifyListeners();
        await _announceThenDrain(message);
    }
  }

  /// [_heard] will not be sent because a prompt is open: it becomes the
  /// target session's draft and the prompt is read next.
  void _parkAsDraft() {
    final String? text = _heard;
    final String? target = _target;
    _heard = null;
    _needsTap = false;
    _phase = CarPhase.idle;
    _notice = 'Hay un permiso abierto: tu mensaje quedó como borrador.';
    if (_ask != null) _askUnread = true;
    if (text != null && target != null) unawaited(_ports.saveDraft(target, text));
  }

  /// Shows the text, says "Envío: …", counts down, sends. If the read-back
  /// was not heard, there is no countdown: it waits for a tap.
  Future<void> _confirm(String text) async {
    final int op = ++_op;
    _heard = text;
    _phase = CarPhase.confirming;
    _count = countdown;
    _needsTap = false;
    notifyListeners();
    final bool spoken = await _speakIn(op, () => _ports.speak('Envío: ${briefForSpeech(text)}'));
    if (op != _op || _disposed) return;
    if (!spoken) {
      _needsTap = true;
      _notice = 'No pude leerte el mensaje en voz alta: toca Enviar o Cancelar.';
      notifyListeners();
      return;
    }
    for (int n = countdown; n > 0; n--) {
      _count = n;
      notifyListeners();
      await _wait(const Duration(seconds: 1));
      if (op != _op || _disposed) return;
    }
    await _send(op);
  }

  Future<void> _send(int op) async {
    final String? text = _heard;
    final String? target = _target;
    if (text == null || target == null) return;
    if (target == _currentId && _ask != null) {
      _parkAsDraft();
      notifyListeners();
      await _announceThenDrain('Hay un permiso abierto. Tu mensaje quedó como borrador.');
      return;
    }
    _needsTap = false;
    _phase = CarPhase.sending;
    notifyListeners();
    final SendOutcome r = await _ports.send(target, _targetPane, text);
    if (_disposed) return;
    _heard = null;
    _phase = CarPhase.idle;
    final String say;
    switch (r.kind) {
      case SendKind.delivered:
        _notice = 'Enviado.';
        say = 'Enviado.';
      case SendKind.queuedOffline:
        _notice = 'Sin conexión: se enviará al volver la red.';
        say = 'Sin conexión, se enviará al volver la red.';
      case SendKind.blocked:
        _notice = 'Hay un mensaje anterior sin resolver; ábrelo en el chat.';
        say = 'Hay un mensaje anterior sin resolver. Ábrelo en el chat.';
      case SendKind.waitingPrompt:
        _notice = 'Hay un permiso abierto: se enviará cuando lo respondas.';
        say = 'Hay un permiso abierto. Se enviará cuando lo respondas.';
      case SendKind.failed:
        _notice = 'No se pudo enviar: ${r.message}';
        say = 'No se pudo enviar. Queda en el chat para reintentar.';
      case SendKind.rejected:
        _notice = '${r.message} Quedó como borrador.';
        say = 'No se pudo guardar el mensaje. Quedó como borrador.';
    }
    notifyListeners();
    await _announceThenDrain(say);
  }

  Future<void> _read(ReplyRef reply) async {
    final String? s = _currentId;
    if (s == null) return;
    final int op = ++_op;
    _readIds.add(reply.id);
    _ports.markRead(s, reply.id);
    _phase = CarPhase.reading;
    notifyListeners();
    final bool done = await _speakIn(op, () => _ports.speakReply(reply.text));
    if (op != _op || _disposed) return;
    _phase = CarPhase.idle;
    notifyListeners();
    if (done && _ports.continuous && !_loopOff) {
      await _ports.cue();
      if (op != _op || _disposed) return;
      await _listen(auto: true);
    } else {
      _drain();
    }
  }

  Future<void> _readAsk(AskPrompt ask) async {
    final int op = ++_op;
    final String q = ask.question.trim().isEmpty ? 'El agente te pregunta.' : ask.question.trim();
    final bool done = await _speakIn(op, () => _ports.speak('$q ${describeOptions(ask.options)}'));
    if (op != _op || _disposed) return;
    final bool same = _askSig == ask.signature;
    if (done && same) _askReadSig = ask.signature;
    if (done && same && _ports.continuous && !_loopOff) {
      await _ports.cue();
      if (op != _op || _disposed) return;
      await _listen(auto: true);
    } else {
      _drain();
    }
  }

  Future<void> _answerBySpeech(String heard) async {
    final AskPrompt ask = _ask!;
    final String sig = _askSig!;
    final String session = _currentId!;
    final AskOption? o = parseOptionChoice(heard, ask.options);
    // What was said was an answer (or an attempt): never a draft.
    _ports.releaseDictation();
    if (o == null) {
      _phase = CarPhase.idle;
      _notice = 'No entendí «$heard».';
      notifyListeners();
      final int op = ++_op;
      final bool done = await _speakIn(
          op, () => _ports.speak('No entendí. Di el número de la opción. ${describeOptions(ask.options)}'));
      if (op != _op || _disposed) return;
      final bool same = _askSig == sig;
      if (done && same) _askReadSig = sig;
      if (done && same && !_loopOff && _reasks++ < 2) {
        await _ports.cue();
        if (op != _op || _disposed) return;
        await _listen(auto: true);
      } else {
        _drain();
      }
      return;
    }
    final int op = ++_op;
    _choice = _Choice(session: session, pane: _pane, sig: sig, option: o);
    _option = o;
    _phase = CarPhase.choosing;
    _count = countdown;
    _needsTap = false;
    notifyListeners();
    final bool spoken =
        await _speakIn(op, () => _ports.speak('Elegiste opción ${optionWord(o.key)}, ${speakableLabel(o.label)}.'));
    if (op != _op || _disposed) return;
    if (!spoken) {
      _needsTap = true;
      _notice = 'No pude leerte la opción en voz alta: toca Confirmar o Cancelar.';
      notifyListeners();
      return;
    }
    for (int n = countdown; n > 0; n--) {
      _count = n;
      notifyListeners();
      await _wait(const Duration(seconds: 1));
      if (op != _op || _disposed) return;
    }
    await _answerChoice(op);
  }

  /// Sends the chosen option only if the session, pane and prompt are the
  /// ones it was chosen for; otherwise sends nothing and reads the prompt.
  Future<void> _answerChoice(int op) async {
    final _Choice? c = _choice;
    if (c == null) return;
    if (c.session != _currentId || c.sig != _askSig || c.pane != _pane || _ask == null) {
      _choice = null;
      _option = null;
      _needsTap = false;
      _phase = CarPhase.idle;
      _notice = 'La pregunta cambió; no respondí.';
      if (_ask != null) _askUnread = true;
      notifyListeners();
      await _announceThenDrain('La pregunta cambió. No respondí.');
      return;
    }
    await _answer(c, op);
  }

  Future<void> _answer(_Choice c, int op) async {
    _choice = null;
    _needsTap = false;
    _phase = CarPhase.sending;
    notifyListeners();
    final String? err = await _ports.answer(c.session, c.option.key, pane: c.pane, promptSig: c.sig);
    if (_disposed) return;
    _option = null;
    _phase = CarPhase.idle;
    if (err == null) {
      if (_askSig == c.sig) _ask = null; // _askSig kept: the same prompt lingering is not re-read
      _notice = 'Respondido: ${speakableLabel(c.option.label)}.';
    } else {
      _notice = err;
      if (_ask != null) _askUnread = true;
    }
    notifyListeners();
    await _announceThenDrain(err == null ? 'Listo.' : 'No se pudo responder.');
  }

  /// Leaving the screen: stop talking, keep anything said.
  @override
  void dispose() {
    if (_disposed) return;
    final String? text = _phase == CarPhase.confirming ? _heard : null;
    final String? target = _target;
    _disposed = true;
    _op++;
    _cancelTimers();
    _ports.stopSpeaking();
    if (_phase == CarPhase.listening) _ports.stopListening();
    if (text != null && target != null) unawaited(_ports.saveDraft(target, text));
    super.dispose();
  }
}

/// Fixed phrases Modo auto may need to say offline (pre-cached on open).
const List<String> kCarPhrases = <String>[
  'Enviado.',
  'Sin conexión, se enviará al volver la red.',
  'No se pudo transcribir. La grabación está guardada: toca reintentar.',
  'No se pudo enviar. Queda en el chat para reintentar.',
  'Hay un mensaje anterior sin resolver. Ábrelo en el chat.',
  'Hay un permiso abierto. Tu mensaje quedó como borrador.',
  'Hay un permiso abierto. Se enviará cuando lo respondas.',
  'La pregunta cambió. No respondí.',
  'Cancelado. Quedó como borrador.',
  'Cancelado.',
  'No se oyó nada.',
  'Listo.',
  'Detenido.',
];

/// First words of a long message, for the spoken read-back.
String briefForSpeech(String text, {int words = 14}) {
  final List<String> w = text.trim().split(RegExp(r'\s+'));
  return w.length <= words ? text.trim() : '${w.take(words).join(' ')}…';
}
