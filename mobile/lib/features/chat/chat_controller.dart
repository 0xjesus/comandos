import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/connection.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/transcript.dart';
import '../../core/outbox/message_transport.dart';
import '../../core/outbox/outbox.dart';
import '../../core/polling/poller.dart';
import 'transcript_merge.dart';

/// State of one open chat. Polls `POST /chat/transcript` every 2 s while
/// visible, shows exactly the window the server answers (as the web page
/// does), and feeds the outbox the user messages it sees (to settle
/// delivered/uncertain bubbles).
class ChatController extends ChangeNotifier {
  ChatController({
    required this.session,
    String? pane,
    required Connection connection,
    required Outbox outbox,
  })  : _pane = pane,
        _connection = connection,
        _outbox = outbox {
    _poller = Poller(interval: interval, task: _tick);
  }

  static const Duration interval = Duration(seconds: 2);

  /// How many answers in a row may say «no agent here» before the chat
  /// believes it (the agent restarting or its transcript rotating look the
  /// same for a moment). After that it shows what the server says, as the
  /// web does, instead of an old conversation that no longer moves.
  static const int gapPolls = 2;

  /// "Working" while the transcript changed this recently and the last
  /// message is not the assistant's (same rule as the web client).
  static const int workingAgeSec = 90;

  final String session;
  final Connection _connection;
  final Outbox _outbox;
  late final Poller _poller;

  String? _pane;
  String _token = '';
  List<ChatMessage> _messages = const <ChatMessage>[];
  int _gaps = 0;
  int? _age;
  int _ageAtMs = 0;
  AskPrompt? _ask;
  String? _agent;
  bool _loaded = false;
  bool _gone = false;
  ApiException? _error;
  int _errors = 0;
  bool _visible = false;
  bool _seenSent = false;
  bool _lastWorking = false;

  /// Bumps when the target pane changes: an answer for the old pane is dropped.
  int _gen = 0;

  /// Bumps when the message list changed (for stick-to-bottom logic).
  final ValueNotifier<int> listVersion = ValueNotifier<int>(0);

  String? get pane => _pane;
  /// The server's latest window (≤ 80 messages), oldest first.
  List<ChatMessage> get messages => _messages;
  AskPrompt? get ask => _ask;
  String? get agent => _agent;
  bool get loaded => _loaded;

  /// Loaded, and the pane has no Claude/Codex transcript (a plain terminal).
  bool get noAgent => _loaded && _agent == null && _messages.isEmpty;

  /// The session no longer exists on the desktop.
  bool get gone => _gone;
  ApiException? get error => _error;

  bool get working {
    final int? age = _age;
    if (age == null) return false;
    final int elapsed = ((DateTime.now().millisecondsSinceEpoch - _ageAtMs) / 1000).floor();
    if (age + elapsed >= workingAgeSec) return false;
    return _messages.isNotEmpty && _messages.last.role != ChatRole.assistant;
  }

  /// How many user messages equal [text] in the server's current window.
  /// Passed to the outbox at enqueue time.
  int? baselineFor(String text) => _loaded ? countMatchingUserMessages(_messages, text) : null;

  void setVisible(bool visible) {
    if (_visible == visible) return;
    _visible = visible;
    if (visible && !_gone) {
      _poller.start();
    } else {
      _poller.stop();
    }
  }

  /// Fetch right now (after sending, after answering a prompt).
  void poke() => _poller.poke();

  /// Reads another pane of the session as chat (the dock's pane chips). Null
  /// or the same pane: nothing changes.
  void setPane(String? pane) {
    if (pane == null || pane.isEmpty || pane == _pane) return;
    _gen++;
    _pane = pane;
    _token = '';
    _messages = const <ChatMessage>[];
    _gaps = 0;
    _loaded = false;
    _ask = null;
    _agent = null;
    _age = null;
    _error = null;
    _errors = 0;
    listVersion.value++;
    notifyListeners();
    if (_visible && !_gone) _poller.poke();
  }

  Future<Duration?> _tick() async {
    final ComandosApi? api = _connection.api;
    if (api == null) return const Duration(seconds: 10);
    if (!_seenSent) {
      _seenSent = true;
      unawaited(api.sessionSeen(session).catchError((Object _) {}));
    }
    final int gen = _gen;
    try {
      final TranscriptResponse r = await api.transcript(session: session, pane: _pane, since: _token);
      // The pane changed while this was in flight: its answer is for the old one.
      if (gen != _gen) return Duration.zero;
      _errors = 0;
      final bool hadError = _error != null;
      _error = null;
      _apply(r, force: hadError);
      return null;
    } on BusyException {
      // "Chat busy" (429): at most 2 transcripts run at once on the desktop.
      return const Duration(seconds: 3);
    } on NotFoundException {
      _gone = true;
      _error = const NotFoundException(message: 'Esta sesión ya no existe en tu equipo.');
      _poller.stop();
      notifyListeners();
      return null;
    } on ApiException catch (e) {
      _errors++;
      _error = e;
      if (e is UnauthorizedException) _connection.tokenRejected.value = true;
      notifyListeners();
      return errorBackoff(_errors, base: interval);
    } catch (_) {
      _errors++;
      _error = const BadResponseException();
      notifyListeners();
      return errorBackoff(_errors, base: interval);
    }
  }

  void _apply(TranscriptResponse r, {bool force = false}) {
    bool notify = force || !_loaded;
    _loaded = true;
    if (r.pane != null && r.pane != _pane) {
      _pane = r.pane;
      notify = true;
    }
    if (r.age != null) {
      _age = r.age;
      _ageAtMs = DateTime.now().millisecondsSinceEpoch;
    }
    final String? oldAsk = _ask?.signature;
    _ask = r.ask;
    if (oldAsk != _ask?.signature) notify = true;
    _token = r.token;

    if (!r.unchanged) {
      // «No agent here» while a conversation shows: a short gap keeps the
      // chat (agent restarting, transcript rotating); a lasting one is the
      // truth, and the chat says so instead of staying on old messages.
      final bool gap = r.agent == null && r.messages.isEmpty && _messages.isNotEmpty;
      if (gap && _gaps < gapPolls) {
        _gaps++;
      } else {
        _gaps = 0;
        if (r.agent != _agent) {
          _agent = r.agent;
          notify = true;
        }
        final WindowUpdate w = adoptWindow(_messages, r.messages);
        if (w.changed) {
          _messages = w.messages;
          listVersion.value++;
          notify = true;
        }
        _outbox.observeTranscript(session, <String>[
          for (final ChatMessage msg in _messages)
            if (msg.role == ChatRole.user) msg.text,
        ]);
      }
    }
    if (working != _lastWorking) {
      _lastWorking = working;
      notify = true;
    }
    if (notify) notifyListeners();
  }

  /// Answers the permission menu: digit, then Enter 250 ms later.
  Future<String?> answer(String option) async {
    final ComandosApi? api = _connection.api;
    if (api == null) return 'Sin vincular.';
    try {
      await api.answerAsk(session: session, pane: _pane, option: option);
      _ask = null;
      notifyListeners();
      poke();
      return null;
    } on ApiException catch (e) {
      return e.userMessage;
    }
  }

  /// Answers the prompt the person saw: reads the pane again first and only
  /// types the digit if the same prompt (same question, detail and options)
  /// is still open in the same pane, so a tap never lands on a prompt that
  /// replaced it. Null when answered, else a Spanish reason.
  Future<String?> answerPrompt(AskPrompt shown, AskOption option) async {
    final ComandosApi? api = _connection.api;
    if (api == null) return 'Sin vincular.';
    final String? pane = _pane;
    final int gen = _gen;
    try {
      final TranscriptResponse fresh = await api.transcript(session: session, pane: pane, since: _token);
      if (gen != _gen) return 'Cambiaste de pane; no respondí.';
      final AskPrompt? now = fresh.ask;
      if (now == null) {
        _ask = null;
        notifyListeners();
        poke();
        return 'Ese permiso ya no está abierto.';
      }
      if (now.signature != shown.signature || (pane != null && fresh.pane != null && fresh.pane != pane)) {
        _ask = now;
        notifyListeners();
        return 'El permiso cambió; revisa las opciones otra vez.';
      }
      if (!now.options.any((AskOption o) => o.key == option.key)) return 'Esa opción ya no está.';
    } on ApiException catch (e) {
      return e.userMessage;
    }
    return answer(option.key);
  }

  /// Interrupts the agent (Escape).
  Future<String?> interrupt() async {
    final ComandosApi? api = _connection.api;
    if (api == null) return 'Sin vincular.';
    try {
      await api.key(session: session, pane: _pane, key: 'Escape');
      poke();
      return null;
    } on ApiException catch (e) {
      return e.userMessage;
    }
  }

  @override
  void dispose() {
    _poller.stop();
    listVersion.dispose();
    // Drop the cache of a closed chat right away.
    _messages = const <ChatMessage>[];
    super.dispose();
  }
}
