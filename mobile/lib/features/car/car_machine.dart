import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/models/ask_prompt.dart';
import '../../core/models/session_feed.dart';
import '../../core/util/clock.dart';
import '../voice/dictation.dart';
import '../voice/reply_watcher.dart';
import 'car_commands.dart';
import 'option_parser.dart';

/// One split (a pane of a session) as Modo auto sees it.
@immutable
class CarSession {
  const CarSession({
    required this.session,
    required this.label,
    required this.state,
    this.ts = 0,
    this.pane,
    this.reply = '',
    this.split = 1,
    this.splits = 1,
    this.agent = '',
    this.model = '',
    this.unread = false,
  });

  final String session;
  final String label;
  final SessionState state;
  final double ts;
  final String? pane;
  final String reply;

  /// 1-based position of this pane in its session, and how many it has.
  final int split;
  final int splits;
  final String agent;
  final String model;

  /// The session has a reply the person has not seen.
  final bool unread;

  /// Identity of the split.
  String get key => '$session\u0001${pane ?? ''}';

  /// "Pagos" or, in a session with several splits, "Pagos, split 2".
  String get name => splits > 1 ? '$label, split $split' : label;

  /// What the call says when it arrives at this split:
  /// "Pagos, split 2, claude opus 5."
  String get intro {
    final String am = <String>[agent, model]
        .where((String x) => x.trim().isNotEmpty)
        .join(' ')
        .replaceAll(RegExp(r'[-_.:/]+'), ' ')
        .replaceAll(RegExp(r'\s+'), ' ')
        .trim();
    return '$label, split $split${am.isEmpty ? '' : ', $am'}.';
  }

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

/// The splits of a `/session-feed` snapshot, in feed order: one per pane of
/// `paneList`, or one for the session when the list is empty.
List<CarSession> splitsOf(List<SessionSummary> sessions) => <CarSession>[
      for (final SessionSummary s in sessions)
        if (s.paneList.isEmpty)
          CarSession(
            session: s.session,
            label: s.label,
            state: s.state,
            ts: s.ts,
            pane: s.pane,
            reply: s.reply,
            agent: s.agent,
            model: s.model,
            unread: s.unread,
          )
        else
          for (int i = 0; i < s.paneList.length; i++)
            CarSession(
              session: s.session,
              label: s.label,
              state: s.paneList[i].state,
              ts: s.paneList[i].ts,
              pane: s.paneList[i].pane,
              reply: s.paneList[i].reply,
              split: i + 1,
              splits: s.paneList.length,
              agent: s.paneList[i].agent,
              model: s.paneList[i].model,
              unread: s.unread,
            ),
    ];

/// "Te espera" first, then working, then the rest; newest first in each.
List<CarSession> orderForCar(List<CarSession> sessions) {
  int rank(CarSession s) => s.state.waitsForHuman ? 0 : (s.state == SessionState.work ? 1 : 2);
  return <CarSession>[...sessions]..sort((CarSession a, CarSession b) {
      final int r = rank(a).compareTo(rank(b));
      if (r != 0) return r;
      final int t = b.ts.compareTo(a.ts);
      if (t != 0) return t;
      final int n = a.session.compareTo(b.session);
      return n != 0 ? n : a.split.compareTo(b.split);
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
  /// [shortAnswer]: an answer to a permission prompt ("dos", "sí"): the
  /// phone's own recognizer, never OpenAI.
  Future<DictationOutcome> listen(String session,
      {required VoidCallback onTranscribing, bool requireSpeech = false, bool shortAnswer = false});
  void stopListening();
  void cancelListening();

  /// Listens once, with the phone's own recognizer (free), for a spoken
  /// command: '' when nothing was said, null when nothing can listen.
  /// [until] may end it early on a partial result.
  Future<String?> hear(Duration window, {bool Function(String partial)? until});

  /// Ends [hear] now (it completes with '').
  void stopHearing();
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

enum CarPhase {
  idle,
  listening,
  transcribing,
  failed,
  confirming,
  sending,
  reading,
  choosing,

  /// The call is open with nothing to do: it may be listening for a
  /// command and speaks as soon as something arrives.
  waiting,
}

/// Commands that apply in each moment of the call.
const Set<CarCommand> _openTurnCommands = <CarCommand>{
  CarCommand.next, CarCommand.previous, CarCommand.repeat, CarCommand.hangUp, CarCommand.pause, //
  CarCommand.cancel,
};
const Set<CarCommand> _askTurnCommands = <CarCommand>{
  CarCommand.next, CarCommand.previous, CarCommand.repeat, CarCommand.hangUp, CarCommand.pause, //
};
const Set<CarCommand> _countdownCommands = <CarCommand>{
  CarCommand.no, CarCommand.cancel, CarCommand.send, CarCommand.hangUp,
};
const Set<CarCommand> _waitCommands = <CarCommand>{
  CarCommand.next, CarCommand.previous, CarCommand.repeat, CarCommand.hangUp, CarCommand.pause, //
  CarCommand.talk,
};
const Set<CarCommand> _pausedCommands = <CarCommand>{
  CarCommand.resume, CarCommand.next, CarCommand.previous, CarCommand.repeat, CarCommand.hangUp, //
  CarCommand.talk,
};

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
/// The call ([startCall]): it says how many splits wait, then visits them
/// one by one (waiting first): "Pagos, split 2, claude opus 5.", the
/// permission prompt or the new reply, then it listens on its own. What is
/// said is a command (`car_commands.dart`), an answer to the prompt, or a
/// message: "Envío: …" and it goes after [countdown] seconds unless "no" /
/// "cancela" is heard meanwhile. Silence moves on to the next split that
/// waits, or waits quietly (listening for commands for a while) and speaks
/// when something arrives. Without [startCall] the machine works by taps
/// (the original Modo auto).
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
/// * A command counts only as a whole utterance, and only where it applies
///   (a bare "no" is a message in an open turn, a cancel in a countdown).
class CarMachine extends ChangeNotifier {
  CarMachine({
    required CarPorts ports,
    String? session,
    String? pane,
    NowMs now = systemNowMs,
    TimerFactory timer = systemTimer,
    this.countdown = 3,
    this.announceGap = const Duration(seconds: 45),
    this.announceTtl = const Duration(minutes: 2),
    this.loadTimeout = const Duration(seconds: 6),
    this.quietRounds = 4,
    this.quietWindow = const Duration(seconds: 20),
    this.pauseMax = const Duration(minutes: 2),
  })  : _ports = ports,
        _currentId = session,
        _wantPane = pane,
        _explicit = session != null,
        _now = now,
        _timer = timer;

  final CarPorts _ports;
  final NowMs _now;
  final TimerFactory _timer;
  final int countdown;
  final Duration announceGap;
  final Duration announceTtl;

  /// How long a visit waits for the split's chat to load.
  final Duration loadTimeout;

  /// Waiting quietly, how many times it listens for a command before it
  /// only speaks when something arrives (each listen restarts the phone's
  /// recognizer, which may beep).
  final int quietRounds;
  final Duration quietWindow;

  /// «Espera» resumes on its own after this.
  final Duration pauseMax;

  /// Empty automatic rounds (continuous conversation) before the loop
  /// stops and waits for the Hablar button.
  static const int maxEmptyRounds = 2;

  CarPhase _phase = CarPhase.idle;
  List<CarSession> _ordered = const <CarSession>[];
  String? _currentId;
  String? _currentKey;
  final String? _wantPane;
  final bool _explicit;
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

  // The call.
  bool _inCall = false;
  bool _ended = false;
  bool _paused = false;

  /// Nothing can listen (no microphone / recognizer): no automatic turns
  /// until a tap.
  bool _blocked = false;
  bool _visiting = false;
  bool _quietSaid = false;
  bool _hearing = false;
  int _hearGen = 0;

  /// Splits with news (finished, or unread when the call started).
  final Set<String> _fresh = <String>{};

  /// Splits left in silence: not visited again on their own until their
  /// state changes.
  final Set<String> _quiet = <String>{};
  Completer<void>? _loaded;
  String? _loadedKey;
  ReplyRef? _latest;
  bool _latestUnread = false;
  String _lastSaid = '';

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

  /// Session of the current split.
  String? get currentId => _currentId;
  CarSession? get current {
    for (final CarSession s in _ordered) {
      if (_currentKey != null ? s.key == _currentKey : s.session == _currentId) return s;
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

  /// The call is on ([startCall]); [ended] once hung up.
  bool get inCall => _inCall;
  bool get ended => _ended;
  bool get paused => _paused;

  /// Listening for a spoken command (quiet wait or a countdown).
  bool get hearing => _hearing;

  /// Splits that wait for the person: a prompt, a limit or news.
  int get waitingCount => _ordered.where(_needsYou).length;

  bool get canSwitch =>
      _phase == CarPhase.idle || _phase == CarPhase.reading || _phase == CarPhase.failed || _phase == CarPhase.waiting;

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
      case CarPhase.waiting:
        if (_paused) return 'En pausa';
        return _working ? 'Trabajando…' : 'En espera';
      case CarPhase.idle:
        if (_ask != null) return 'Te espera: elige una opción';
        return _working ? 'Trabajando…' : 'Listo';
    }
  }

  /// What can be said right now (empty outside the call).
  String get hint {
    if (!_inCall) return '';
    switch (_phase) {
      case CarPhase.confirming:
      case CarPhase.choosing:
        return _needsTap ? '' : 'Di «no» para cancelar';
      case CarPhase.listening:
        return _ask != null
            ? 'Di la opción, o «siguiente», «repite», «cuelga»'
            : 'Habla, o di «siguiente», «repite», «espera», «cuelga»';
      case CarPhase.waiting:
        if (_blocked) return '';
        if (_paused) return 'Di «sigue» para seguir';
        return _hearing ? 'Di «siguiente», «repite», «mensaje» o «cuelga»' : 'Te aviso cuando haya algo';
      case CarPhase.idle:
      case CarPhase.transcribing:
      case CarPhase.failed:
      case CarPhase.sending:
      case CarPhase.reading:
        return '';
    }
  }

  // ---- Feed ------------------------------------------------------------------

  /// New `/session-feed` snapshot (one entry per split). Announces other
  /// splits that finished or started waiting (short, rate-limited, never
  /// over a dictation); in a call it visits them instead.
  ///
  /// When the current split is missing from a snapshot, the machine only
  /// moves to another one while nothing is in progress (idle / reading /
  /// failed / waiting): a dictation, countdown or answer stays with its
  /// split.
  void updateSessions(List<CarSession> sessions) {
    if (_disposed) return;
    _ordered = orderForCar(sessions);
    final bool present = _ordered.any((CarSession s) => s.key == _currentKey);
    if (_currentKey == null) {
      final CarSession? first = _initialSplit();
      if (first != null) {
        _currentKey = first.key;
        _currentId = first.session;
      } else if (_ordered.isNotEmpty && canSwitch) {
        _supersede();
        _adopt(_ordered.first);
      }
    } else if (!present && _ordered.isNotEmpty && canSwitch) {
      if (_inCall) {
        unawaited(_visit(_ordered.first));
      } else {
        _supersede();
        _adopt(_ordered.first);
      }
    }
    final Set<String> keys = <String>{for (final CarSession s in _ordered) s.key};
    _fresh.removeWhere((String k) => !keys.contains(k));
    _quiet.removeWhere((String k) => !keys.contains(k));
    final int now = _now();
    for (final CarSession s in sessions) {
      final SessionState? before = _states[s.key];
      _states[s.key] = s.state;
      if (!_sessionsSeen || before == null || before == s.state) continue;
      _quiet.remove(s.key);
      if (s.state == SessionState.work) _fresh.remove(s.key);
      if (s.key == _currentKey) continue;
      final bool finished = before == SessionState.work && s.state == SessionState.idle;
      if (s.state.waitsForHuman || finished) _fresh.add(s.key);
      if (_inCall) continue; // the call visits it instead
      String? text;
      if (s.state == SessionState.ask) {
        text = '${s.name} te espera: pide permiso.';
      } else if (s.state == SessionState.limit) {
        text = '${s.name} llegó al límite de uso.';
      } else if (finished) {
        text = '${s.name} terminó.';
      }
      if (text == null) continue;
      final int? last = _lastAnnounced[s.key];
      if (last != null && now - last < announceGap.inMilliseconds) continue;
      _lastAnnounced[s.key] = now;
      _announcements.add((text, now));
      if (_announcements.length > 3) _announcements.removeAt(0);
    }
    _sessionsSeen = true;
    notifyListeners();
    _drain();
  }

  /// The split asked for when the screen opened (its session, and its pane
  /// when given), or the first one when none was asked for.
  CarSession? _initialSplit() {
    if (_ordered.isEmpty) return null;
    if (_currentId == null) return _ordered.first;
    final List<CarSession> mine = _ordered.where((CarSession s) => s.session == _currentId).toList()
      ..sort((CarSession a, CarSession b) => a.split.compareTo(b.split));
    if (mine.isEmpty) return null;
    return mine.where((CarSession s) => s.pane == _wantPane).firstOrNull ?? mine.first;
  }

  /// The current split's chat: working flag, permission prompt, excerpt,
  /// the pane the chat reads (where messages and answers go) and its last
  /// completed reply ([latestUnread]: not read aloud before).
  void updateCurrent({
    required bool working,
    AskPrompt? ask,
    String? excerpt,
    String? pane,
    ReplyRef? latest,
    bool latestUnread = false,
  }) {
    if (_disposed) return;
    _working = working;
    if (excerpt != null) _excerpt = excerpt;
    if (pane != null) _pane = pane;
    if (latest?.id != _latest?.id) {
      _latest = latest;
      _latestUnread = latestUnread;
    } else if (latestUnread) {
      _latestUnread = true;
    }
    _loadedKey = _currentKey;
    final Completer<void>? loaded = _loaded;
    if (loaded != null && !loaded.isCompleted) loaded.complete();
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

  /// A reply of the current split completed. Read once, when free.
  void replyCompleted(ReplyRef reply) {
    if (_disposed || _readIds.contains(reply.id)) return;
    _pendingReply = reply;
    _drain();
  }

  /// The app went to the background (false) or came back (true). Hidden,
  /// Modo auto stops speaking and recording (the recording is still
  /// transcribed) and cancels any countdown: what is pending waits for a
  /// tap, or, in a call, is read back again once visible. Back in the
  /// foreground it picks up what is queued.
  void setForeground(bool foreground) {
    if (_disposed || _foreground == foreground) return;
    _foreground = foreground;
    if (!foreground) {
      _supersede();
      if (_phase == CarPhase.listening) _ports.stopListening();
      if (_phase == CarPhase.reading) _phase = CarPhase.idle;
      if (_phase == CarPhase.confirming || _phase == CarPhase.choosing) _needsTap = true;
      notifyListeners();
      return;
    }
    notifyListeners();
    if (_inCall && _phase == CarPhase.confirming && _heard != null) {
      unawaited(_confirm(_heard!));
    } else if (_inCall && _phase == CarPhase.choosing && _choice != null) {
      unawaited(_choose(_choice!));
    } else if (!_drain() && _inCall && _phase == CarPhase.waiting && !_blocked) {
      unawaited(_quietWait(announce: false));
    }
  }

  // ---- The call ------------------------------------------------------------------

  /// Starts the call: "N splits te esperan", then the first split that
  /// waits (or the one Modo auto was opened for), with no tap.
  Future<void> startCall() async {
    if (_inCall || _ended || _disposed) return;
    _inCall = true;
    for (final CarSession s in _ordered) {
      if (s.unread && s.state == SessionState.idle) _fresh.add(s.key);
    }
    final int n = _ordered.where(_needsYou).length;
    final String intro = _ordered.isEmpty
        ? 'No hay sesiones abiertas. '
        : n == 0
            ? 'Ningún split te espera. '
            : n == 1
                ? 'Un split te espera. '
                : '$n splits te esperan. ';
    final CarSession? first = (_explicit ? current : null) ?? _ordered.where(_needsYou).firstOrNull;
    if (first == null) {
      final int op = ++_op;
      _quietSaid = true;
      notifyListeners();
      await _speakIn(op, () => _ports.speak('${intro}Te aviso cuando haya algo.'));
      if (op == _op && !_disposed) _drain();
      return;
    }
    await _visit(first, intro: intro);
  }

  /// Ends the call ("cuelga", "para", or the hang-up area). A message in
  /// its countdown is kept as a draft; a recording in progress is stopped
  /// and its text lands in the draft. The screen closes on [ended].
  Future<void> hangUp() async {
    if (_ended || _disposed) return;
    final String? text = _phase == CarPhase.confirming ? _heard : null;
    final String? target = _target;
    final bool listening = _phase == CarPhase.listening;
    _supersede();
    _ended = true;
    _inCall = false;
    _paused = false;
    if (listening) _ports.stopListening();
    if (text != null && target != null) {
      _heard = null;
      unawaited(_ports.saveDraft(target, text));
    }
    _phase = CarPhase.idle;
    final int op = ++_op;
    notifyListeners();
    await _speakIn(op, () => _ports.speak('Llamada terminada.'));
    if (!_disposed) notifyListeners();
  }

  bool _needsYou(CarSession s) => s.state.waitsForHuman || _fresh.contains(s.key);

  /// The next split (after the current one, in order) that waits for the
  /// person and was not just left in silence.
  CarSession? _nextPending() {
    if (_ordered.isEmpty) return null;
    final int i = _ordered.indexWhere((CarSession s) => s.key == _currentKey);
    for (int k = 1; k <= _ordered.length; k++) {
      final CarSession s = _ordered[(i + k) % _ordered.length];
      if (s.key == _currentKey || _quiet.contains(s.key)) continue;
      if (_needsYou(s)) return s;
    }
    return null;
  }

  /// After something ended: the next split that waits, else wait quietly.
  Future<void> _callContinue() async {
    if (!_inCall || _ended || _disposed || !_foreground) return;
    if (!_paused && !_blocked) {
      final CarSession? n = _nextPending();
      if (n != null) return _visit(n);
    }
    await _quietWait();
  }

  /// Goes to [to]: says who it is, waits for its chat, then presents it.
  Future<void> _visit(CarSession to, {String intro = ''}) async {
    _supersede();
    if (to.key != _currentKey) _adopt(to);
    if (_loadedKey != to.key) _loaded ??= Completer<void>();
    _fresh.remove(to.key);
    _phase = CarPhase.idle;
    _visiting = true;
    _quietSaid = false;
    _paused = false;
    _reasks = 0;
    _emptyRounds = 0;
    _notice = null;
    notifyListeners();
    final int op = ++_op;
    await _speakIn(op, () => _ports.speak('$intro${to.intro}'));
    if (op != _op || _disposed) return;
    final Completer<void>? loaded = _loaded;
    if (loaded != null && !loaded.isCompleted) {
      final Timer t = _timer(loadTimeout, () {
        if (!loaded.isCompleted) loaded.complete();
      });
      _timers.add(t);
      await loaded.future;
      t.cancel();
      _timers.remove(t);
      if (op != _op || _disposed) return;
    }
    _loaded = null;
    _visiting = false;
    await _present(op);
  }

  /// The split's prompt, else its new reply, else its state; then listens.
  Future<void> _present(int op) async {
    final AskPrompt? a = _ask;
    if (a != null) {
      _askUnread = false;
      await _readAsk(a);
      return;
    }
    final ReplyRef? r = _pendingReply ?? (_latestUnread ? _latest : null);
    _pendingReply = null;
    if (r != null && !_readIds.contains(r.id)) {
      await _read(r);
      return;
    }
    final String say = _working
        ? 'Trabajando.'
        : current?.state == SessionState.limit
            ? 'Llegó al límite de uso.'
            : 'Sin respuesta nueva.';
    await _speakIn(op, () => _ports.speak(say));
    if (op != _op || _disposed) return;
    await _openTurn();
  }

  /// Earcon, then listen (a message, an answer or a command).
  Future<void> _openTurn() async {
    if (!_inCall || _disposed || !_foreground) return;
    final int op = ++_op;
    _phase = CarPhase.idle;
    if (!await _cue(op)) return;
    await _listen(auto: true);
  }

  /// The call is open with nothing to do: listens for a command for a
  /// while (free, on the phone), then only speaks when something arrives.
  Future<void> _quietWait({bool announce = true}) async {
    if (_disposed || _ended || !_inCall) return;
    final int op = ++_op;
    _phase = CarPhase.waiting;
    notifyListeners();
    if (announce && !_quietSaid) {
      _quietSaid = true;
      await _speakIn(
          op, () => _ports.speak(_paused ? 'En pausa. Di sigue para seguir.' : 'Nada más por ahora. Te aviso cuando haya algo.'));
      if (op != _op || _disposed) return;
      if (_drain()) return; // something arrived meanwhile
    }
    if (_blocked) return;
    for (int i = 0; i < quietRounds; i++) {
      final String? h = await _hear(quietWindow);
      if (op != _op || _disposed) return;
      if (h == null) break;
      final CarCommand? c = parseCarCommand(h);
      if (c != null && (_paused ? _pausedCommands : _waitCommands).contains(c)) {
        _phase = CarPhase.idle;
        await _command(c);
        return;
      }
    }
  }

  Future<void> _command(CarCommand c) async {
    switch (c) {
      case CarCommand.next:
        await _step(1);
      case CarCommand.previous:
        await _step(-1);
      case CarCommand.repeat:
        await _repeat();
      case CarCommand.hangUp:
        await hangUp();
      case CarCommand.pause:
        await _pause();
      case CarCommand.resume:
        await _resume();
      case CarCommand.talk:
        _paused = false;
        await _openTurn();
      case CarCommand.cancel:
        // Nothing to cancel in an open turn: move on.
        final String? k = _currentKey;
        if (k != null) _quiet.add(k);
        await _callContinue();
      case CarCommand.no:
      case CarCommand.send:
        return;
    }
  }

  Future<void> _step(int delta) async {
    if (_ordered.isEmpty) return;
    _paused = false;
    if (_ordered.length < 2) {
      final int op = ++_op;
      _phase = CarPhase.idle;
      await _speakIn(op, () => _ports.speak('Solo hay un split.'));
      if (op != _op || _disposed) return;
      await _openTurn();
      return;
    }
    final int i = _ordered.indexWhere((CarSession s) => s.key == _currentKey);
    final CarSession to = _ordered[(i + delta + _ordered.length) % _ordered.length];
    _quiet.remove(to.key);
    await _visit(to);
  }

  /// «Repite»: the prompt, else the last reply; then listens again.
  Future<void> _repeat() async {
    _paused = false;
    _phase = CarPhase.idle;
    final AskPrompt? a = _ask;
    if (a != null) {
      await _readAsk(a);
      return;
    }
    final String text = _lastSaid.isNotEmpty ? _lastSaid : (_latest?.text ?? '');
    final int op = ++_op;
    if (text.isEmpty) {
      await _speakIn(op, () => _ports.speak('No hay nada que repetir.'));
    } else {
      _phase = CarPhase.reading;
      notifyListeners();
      await _speakIn(op, () => _ports.speakReply(text));
      if (op != _op || _disposed) return;
      _phase = CarPhase.idle;
      notifyListeners();
    }
    if (op != _op || _disposed) return;
    await _openTurn();
  }

  /// «Espera»: nothing is read or visited; it listens a while for
  /// «sigue» and resumes on its own after [pauseMax].
  Future<void> _pause() async {
    _supersede();
    _paused = true;
    _quietSaid = false;
    late final Timer t;
    t = _timer(pauseMax, () {
      _timers.remove(t);
      if (_paused && !_disposed) unawaited(_resume());
    });
    _timers.add(t);
    await _quietWait();
  }

  Future<void> _resume() async {
    _supersede();
    _paused = false;
    _quietSaid = false;
    _phase = CarPhase.idle;
    final int op = ++_op;
    notifyListeners();
    await _speakIn(op, () => _ports.speak('Seguimos.'));
    if (op == _op && !_disposed) _drain();
  }

  // ---- Buttons ---------------------------------------------------------------

  /// The giant button: Hablar / Terminar / Reintentar.
  Future<void> talk() async {
    switch (_phase) {
      case CarPhase.idle:
      case CarPhase.reading:
      case CarPhase.waiting:
        _loopOff = false;
        _blocked = false;
        _paused = false;
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
        // In a call: say it again (or something else) right away.
        unawaited(_sayThen('Cancelado. Quedó como borrador.', _inCall ? _openTurn : null));
      case CarPhase.choosing:
        _supersede();
        _option = null;
        _choice = null;
        _needsTap = false;
        _phase = CarPhase.idle;
        _notice = 'Cancelado.';
        notifyListeners();
        unawaited(_sayThen('Cancelado.', _inCall ? _openTurn : null));
      case CarPhase.idle:
      case CarPhase.failed:
      case CarPhase.sending:
      case CarPhase.reading:
      case CarPhase.waiting:
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

  /// Callar. A countdown whose read-back is cut waits for a tap. In a call,
  /// cutting a reading opens the turn (to answer or say «siguiente»).
  void hush() {
    final bool wasReading = _hush();
    notifyListeners();
    if (_inCall && wasReading) unawaited(_openTurn());
  }

  bool _hush() {
    final bool wasReading = _phase == CarPhase.reading;
    _supersede();
    _announcements.clear();
    if (_phase == CarPhase.reading) _phase = CarPhase.idle;
    if (_phase == CarPhase.confirming || _phase == CarPhase.choosing) _needsTap = true;
    return wasReading;
  }

  /// Detener: Escape to the agent.
  Future<void> stopAgent() async {
    final String? s = _currentId;
    if (s == null) return;
    _hush();
    notifyListeners();
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

  void next() {
    if (_inCall) {
      unawaited(_step(1));
    } else {
      _switch(1);
    }
  }

  void prev() {
    if (_inCall) {
      unawaited(_step(-1));
    } else {
      _switch(-1);
    }
  }

  // ---- Internals ---------------------------------------------------------------

  void _supersede() {
    _op++;
    _cancelTimers();
    _ports.stopSpeaking();
    _speaking = false;
    _visiting = false;
    _stopHearing();
  }

  /// Makes [to] the current split, forgetting everything of the old one.
  void _adopt(CarSession to) {
    _currentKey = to.key;
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
    _latest = null;
    _latestUnread = false;
    _lastSaid = '';
    _loadedKey = null;
    _loaded = null;
  }

  void _switch(int delta) {
    if (!canSwitch || _ordered.length < 2) return;
    final int i = _ordered.indexWhere((CarSession s) => s.key == _currentKey);
    final CarSession to = _ordered[(i + delta + _ordered.length) % _ordered.length];
    _supersede();
    _adopt(to);
    notifyListeners();
    unawaited(_announceThenDrain('${to.name}, ${to.stateText}.'));
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

  /// The earcon ("your turn"); nothing else starts while it plays. False
  /// when something took over meanwhile.
  Future<bool> _cue(int op) async {
    _speaking = true;
    try {
      await _ports.cue();
    } catch (_) {}
    if (op != _op || _disposed) return false;
    _speaking = false;
    return true;
  }

  Future<void> _announceThenDrain(String text) => _sayThen(text, null);

  /// Says [text], then runs [then] (or picks up what is queued).
  Future<void> _sayThen(String text, Future<void> Function()? then) async {
    final int op = ++_op;
    await _speakIn(op, () => _ports.speak(text));
    if (op != _op || _disposed) return;
    if (then != null) {
      await then();
    } else {
      _drain();
    }
  }

  /// Listens once for a spoken command (free, on the phone).
  Future<String?> _hear(Duration window, {bool Function(String partial)? until}) async {
    if (!_foreground || _disposed) return null;
    final int g = ++_hearGen;
    _hearing = true;
    notifyListeners();
    try {
      return await _ports.hear(window, until: until);
    } catch (_) {
      return null;
    } finally {
      if (g == _hearGen && _hearing) {
        _hearing = false;
        if (!_disposed) notifyListeners();
      }
    }
  }

  void _stopHearing() {
    if (!_hearing) return;
    _hearing = false;
    _ports.stopHearing();
  }

  /// During a countdown (after its read-back was heard): "no" / "cancela"
  /// cancels, "sí" / "envíalo" skips the wait, "cuelga" hangs up.
  Future<void> _hearWindow(int op, {required bool choice}) async {
    final String? h = await _hear(Duration(seconds: countdown), until: (String partial) {
      final CarCommand? c = parseCarCommand(partial);
      return c == CarCommand.no || c == CarCommand.cancel || c == CarCommand.hangUp;
    });
    if (op != _op || _disposed || h == null) return;
    final CarCommand? c = parseCarCommand(h);
    if (c == null || !_countdownCommands.contains(c)) return;
    switch (c) {
      case CarCommand.no:
      case CarCommand.cancel:
        cancel();
      case CarCommand.send:
        await (choice ? confirmChoice() : sendNow());
      case CarCommand.hangUp:
        await hangUp();
      default:
        return;
    }
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

  /// Runs the next queued thing when free, silent and in the foreground:
  /// a reply, a prompt, an announcement; in a call, the next split that
  /// waits (or the quiet wait). True when it started something.
  bool _drain() {
    if (_disposed || !_foreground || _speaking || _visiting || _ended) return false;
    if (_phase == CarPhase.waiting) {
      if (_paused || _blocked) return false;
      final bool work = _pendingReply != null || (_askUnread && _ask != null) || _nextPending() != null;
      if (!work) return false;
      _op++;
      _cancelTimers();
      _stopHearing();
      _phase = CarPhase.idle;
    }
    if (_phase != CarPhase.idle) return false;
    final ReplyRef? reply = _pendingReply;
    if (reply != null) {
      _pendingReply = null;
      unawaited(_read(reply));
      return true;
    }
    if (_askUnread && _ask != null) {
      _askUnread = false;
      unawaited(_readAsk(_ask!));
      return true;
    }
    final int now = _now();
    _announcements.removeWhere(((String, int) a) => now - a.$2 > announceTtl.inMilliseconds);
    if (_announcements.isNotEmpty) {
      final (String text, int _) = _announcements.removeAt(0);
      unawaited(_announceThenDrain(text));
      return true;
    }
    if (_inCall) {
      unawaited(_callContinue());
      return true;
    }
    return false;
  }

  /// Starts a dictation. [auto]: started by the call, the continuous loop
  /// or a re-ask, not by a tap (a silent recording is then dropped).
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
    final DictationOutcome o = await _ports.listen(s,
        requireSpeech: auto,
        shortAnswer: _listenAskSig != null,
        onTranscribing: () {
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
    if (_ended) {
      // Hung up while listening: a message is kept, an answer dropped.
      if (o is DictationText) {
        final String? t = _target;
        if (askFor == null && t != null) {
          unawaited(_ports.saveDraft(t, o.text));
        } else {
          _ports.releaseDictation();
        }
      }
      return;
    }
    switch (o) {
      case DictationText(:final String text):
        _emptyRounds = 0;
        if (_inCall) {
          final CarCommand? c = parseCarCommand(text);
          if (c != null && (askFor != null ? _askTurnCommands : _openTurnCommands).contains(c)) {
            // A command: never a message, never a draft.
            _ports.releaseDictation();
            _phase = CarPhase.idle;
            notifyListeners();
            await _command(c);
            return;
          }
          _quiet.remove(_currentKey);
        }
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
        if (_inCall) {
          // Silence: move on to the next split that waits (or wait).
          final String? k = _currentKey;
          if (k != null) _quiet.add(k);
          _notice = null;
          notifyListeners();
          await _callContinue();
          return;
        }
        _notice = 'No se oyó nada.';
        if (_autoRound && ++_emptyRounds >= maxEmptyRounds) {
          _loopOff = true;
          _notice = 'No se oyó nada. Toca Hablar para seguir.';
        }
        notifyListeners();
        await _announceThenDrain('No se oyó nada.');
      case DictationFailed(:final PendingTake take, :final String message):
        if (_inCall) {
          // The recording waits in the session's chat (Reintentar there);
          // the call goes on.
          _phase = CarPhase.idle;
          _notice = message;
          notifyListeners();
          await _announceThenDrain('No se pudo transcribir. La grabación quedó guardada en el chat.');
          return;
        }
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
        _notice = message;
        if (_inCall) {
          // Nothing can listen: no automatic turns until a tap.
          _blocked = true;
          _phase = CarPhase.waiting;
          notifyListeners();
          final int op = ++_op;
          await _speakIn(op, () => _ports.speak(message));
          return;
        }
        _phase = CarPhase.idle;
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

  /// Shows the text, says "Envío: …", counts down (in a call, listening
  /// for "no"), sends. If the read-back was not heard, there is no
  /// countdown: it waits for a tap.
  Future<void> _confirm(String text) async {
    _supersede();
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
    if (_inCall) unawaited(_hearWindow(op, choice: false));
    for (int n = countdown; n > 0; n--) {
      _count = n;
      notifyListeners();
      await _wait(const Duration(seconds: 1));
      if (op != _op || _disposed) return;
    }
    _stopHearing();
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
    _lastSaid = reply.text;
    _phase = CarPhase.reading;
    notifyListeners();
    final bool done = await _speakIn(op, () => _ports.speakReply(reply.text));
    if (op != _op || _disposed) return;
    _phase = CarPhase.idle;
    notifyListeners();
    if (_inCall) {
      await _openTurn();
    } else if (done && _ports.continuous && !_loopOff) {
      if (!await _cue(op)) return;
      await _listen(auto: true);
    } else {
      _drain();
    }
  }

  Future<void> _readAsk(AskPrompt ask) async {
    final int op = ++_op;
    _phase = CarPhase.idle;
    final String q = ask.question.trim().isEmpty ? 'El agente te pregunta.' : ask.question.trim();
    final bool done = await _speakIn(op, () => _ports.speak('$q ${describeOptions(ask.options)}'));
    if (op != _op || _disposed) return;
    final bool same = _askSig == ask.signature;
    if (done && same) _askReadSig = ask.signature;
    if (done && same && (_inCall || (_ports.continuous && !_loopOff))) {
      if (!await _cue(op)) return;
      await _listen(auto: true);
    } else {
      final String? k = _currentKey;
      if (_inCall && k != null) _quiet.add(k);
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
        if (!await _cue(op)) return;
        await _listen(auto: true);
      } else {
        final String? k = _currentKey;
        if (_inCall && k != null) _quiet.add(k);
        _drain();
      }
      return;
    }
    await _choose(_Choice(session: session, pane: _pane, sig: sig, option: o));
  }

  /// "Elegiste opción dos, no." then a countdown (in a call, listening for
  /// "no"), then the answer. Not heard → waits for Confirmar.
  Future<void> _choose(_Choice c) async {
    _supersede();
    final int op = ++_op;
    final AskOption o = c.option;
    _choice = c;
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
    if (_inCall) unawaited(_hearWindow(op, choice: true));
    for (int n = countdown; n > 0; n--) {
      _count = n;
      notifyListeners();
      await _wait(const Duration(seconds: 1));
      if (op != _op || _disposed) return;
    }
    _stopHearing();
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
    if (_hearing) _ports.stopHearing();
    if (_phase == CarPhase.listening) _ports.stopListening();
    if (text != null && target != null) unawaited(_ports.saveDraft(target, text));
    super.dispose();
  }
}

/// First words of a long message, for the spoken read-back.
String briefForSpeech(String text, {int words = 14}) {
  final List<String> w = text.trim().split(RegExp(r'\s+'));
  return w.length <= words ? text.trim() : '${w.take(words).join(' ')}…';
}
