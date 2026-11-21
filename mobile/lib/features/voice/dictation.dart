import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/util/clock.dart';
import 'openai_client.dart';
import 'recorder.dart';

/// Result of one dictation (or retry).
sealed class DictationOutcome {
  const DictationOutcome();
}

/// Transcribed text. It has ALSO been delivered to the owner's sink (or its
/// draft); the audio is deleted once that delivery completes.
class DictationText extends DictationOutcome {
  const DictationText(this.text);
  final String text;
}

/// OpenAI heard nothing (empty transcription). The audio is deleted.
class DictationNothing extends DictationOutcome {
  const DictationNothing();
}

/// Transcription failed; the recording is kept as [take] for a retry.
class DictationFailed extends DictationOutcome {
  const DictationFailed(this.take, this.message, this.kind);
  final PendingTake take;
  final String message;
  final OpenAiErrorKind? kind;
}

class DictationCancelled extends DictationOutcome {
  const DictationCancelled();
}

/// Could not start (no permission, no key, mic busy).
class DictationDenied extends DictationOutcome {
  const DictationDenied(this.message);
  final String message;
}

/// A recording on disk waiting for (re)transcription.
class PendingTake {
  PendingTake({required this.id, required this.owner, required this.path, required this.createdAtMs, this.message});

  final String id;

  /// Session id the text belongs to.
  final String owner;
  final String path;
  final int createdAtMs;
  String? message;

  String get metaPath => '${path.substring(0, path.lastIndexOf('.'))}.json';
}

enum DictationPhase { idle, recording, transcribing }

/// Receives dictated text for one owner. Completes when the text is durable
/// (composer: inserted and saved as draft; car: sent or saved).
typedef TextSink = Future<void> Function(String text);

/// Turns an audio file into text. Throws [OpenAiException].
typedef Transcriber = Future<String> Function(String path, Future<void> abort);

const String kMicDenied = 'Sin permiso de micrófono. Actívalo en Ajustes de Android › Apps › '
    'ComandOS › Permisos › Micrófono.';

/// One microphone, many screens. Records, auto-stops on silence or at the
/// cap, transcribes with retries, and NEVER drops a recording that has not
/// produced text: on failure the file stays on disk (also across restarts)
/// until a retry succeeds or the person discards it.
class DictationController extends ChangeNotifier {
  DictationController({
    required Recorder recorder,
    required Transcriber transcribe,
    required Future<String> Function() dir,
    required Future<void> Function(String owner, String text) fallback,
    Future<String?> Function()? precheck,
    Future<void> Function()? beforeRecord,
    Future<void> Function()? afterRecord,
    double Function()? silenceSeconds,
    NowMs now = systemNowMs,
    TimerFactory timer = systemTimer,
    Future<void> Function(Duration d)? delay,
    this.maxDuration = const Duration(minutes: 3),
    this.retryDelays = const <Duration>[Duration(seconds: 1), Duration(seconds: 3)],
  })  : _recorder = recorder,
        _transcribe = transcribe,
        _dir = dir,
        _fallback = fallback,
        _precheck = precheck,
        _beforeRecord = beforeRecord,
        _afterRecord = afterRecord,
        _silenceSeconds = silenceSeconds ?? (() => 2.5),
        _now = now,
        _timer = timer,
        _delay = delay ?? ((Duration d) => Future<void>.delayed(d));

  final Recorder _recorder;
  final Transcriber _transcribe;
  final Future<String> Function() _dir;
  final Future<void> Function(String owner, String text) _fallback;
  final Future<String?> Function()? _precheck;
  final Future<void> Function()? _beforeRecord;
  final Future<void> Function()? _afterRecord;
  final double Function() _silenceSeconds;
  final NowMs _now;
  final TimerFactory _timer;
  final Future<void> Function(Duration d) _delay;
  final Duration maxDuration;

  /// Waits between automatic attempts (3 attempts in total by default).
  final List<Duration> retryDelays;

  static const Duration keepPending = Duration(days: 7);
  static const Duration orphanAge = Duration(hours: 24);

  DictationPhase _phase = DictationPhase.idle;
  String? _owner;
  int _startedMs = 0;
  int _elapsedMs = 0;
  double _level = 0;
  int _attempt = 0;
  PendingTake? _current;
  Completer<DictationOutcome>? _done;
  Completer<void>? _abort;
  bool _cancelled = false;
  StreamSubscription<double>? _levels;
  Timer? _cap;
  SilenceDetector? _detector;
  String? _path;
  Future<void>? _loading;
  final List<PendingTake> _pending = <PendingTake>[];
  final Map<String, String> _notices = <String, String>{};
  final Map<String, List<TextSink>> _sinks = <String, List<TextSink>>{};

  DictationPhase get phase => _phase;

  /// The session currently recording / transcribing.
  String? get activeOwner => _owner;
  Duration get elapsed => Duration(milliseconds: _elapsedMs);

  /// 0..1 input level while recording.
  double get level => _level;

  /// 1-based attempt while transcribing.
  int get attempt => _attempt;
  bool get busy => _phase != DictationPhase.idle;

  List<PendingTake> pendingFor(String owner) =>
      _pending.where((PendingTake t) => t.owner == owner).toList(growable: false);

  PendingTake? take(String id) => _pending.where((PendingTake t) => t.id == id).firstOrNull;

  String? noticeFor(String owner) => _notices[owner];

  void clearNotice(String owner) {
    if (_notices.remove(owner) != null) notifyListeners();
  }

  // ---- Sinks ----------------------------------------------------------------

  /// The most recently attached sink of an owner receives its text.
  void attach(String owner, TextSink sink) => (_sinks[owner] ??= <TextSink>[]).add(sink);

  void detach(String owner, TextSink sink) {
    final List<TextSink>? l = _sinks[owner];
    l?.remove(sink);
    if (l != null && l.isEmpty) _sinks.remove(owner);
  }

  /// Hands [text] to the owner's top sink (skipping [except]) or, with no
  /// screen open for it, to the fallback (the session's draft).
  Future<void> deliver(String owner, String text, {TextSink? except}) {
    final List<TextSink> l = _sinks[owner] ?? const <TextSink>[];
    for (int i = l.length - 1; i >= 0; i--) {
      if (!identical(l[i], except)) return l[i](text);
    }
    return _fallback(owner, text);
  }

  // ---- Disk -----------------------------------------------------------------

  /// Scans the takes directory once: pending recordings come back (they
  /// survive restarts); orphans older than 24 h and takes older than 7 days
  /// are deleted. Runs on first voice use, never at app start.
  Future<void> load() => _loading ??= _load();

  Future<void> _load() async {
    try {
      final Directory d = Directory(await _dir());
      await d.create(recursive: true);
      final int now = _now();
      final Set<String> withMeta = <String>{};
      final List<File> audio = <File>[];
      await for (final FileSystemEntity e in d.list()) {
        if (e is! File) continue;
        final String p = e.path;
        if (p.endsWith('.json')) {
          final PendingTake? t = await _readMeta(e);
          final String base = p.substring(0, p.length - 5);
          if (t == null || now - t.createdAtMs > keepPending.inMilliseconds) {
            await _rm(e.path);
            await _rm('$base.m4a');
            continue;
          }
          final File a = File(t.path);
          if (!await a.exists() || await a.length() == 0) {
            if (now - t.createdAtMs > orphanAge.inMilliseconds) await _rm(e.path);
            continue;
          }
          withMeta.add(t.path);
          if (!_pending.any((PendingTake x) => x.id == t.id) && _current?.id != t.id) {
            _pending.add(t..message = 'Grabación sin transcribir.');
          }
        } else {
          audio.add(e);
        }
      }
      for (final File f in audio) {
        if (withMeta.contains(f.path) || f.path == _path) continue;
        try {
          if (now - (await f.lastModified()).millisecondsSinceEpoch > orphanAge.inMilliseconds) {
            await _rm(f.path);
          }
        } catch (_) {}
      }
      _pending.sort((PendingTake a, PendingTake b) => a.createdAtMs.compareTo(b.createdAtMs));
      notifyListeners();
    } catch (_) {
      // No storage: dictation still works for the current process.
    }
  }

  Future<PendingTake?> _readMeta(File f) async {
    try {
      final Object? j = jsonDecode(await f.readAsString());
      if (j is! Map || j['owner'] is! String || j['id'] is! String || j['createdAtMs'] is! num) return null;
      final String p = f.path;
      return PendingTake(
        id: j['id'] as String,
        owner: j['owner'] as String,
        path: '${p.substring(0, p.length - 5)}.m4a',
        createdAtMs: (j['createdAtMs'] as num).toInt(),
      );
    } catch (_) {
      return null;
    }
  }

  static Future<void> _rm(String path) async {
    try {
      await File(path).delete();
    } catch (_) {}
  }

  Future<void> _forget(PendingTake t) async {
    _pending.removeWhere((PendingTake x) => x.id == t.id);
    await _rm(t.path);
    await _rm(t.metaPath);
  }

  // ---- Recording --------------------------------------------------------------

  /// Records for [owner] until [stop], silence or the cap, then transcribes.
  Future<DictationOutcome> start(String owner) async {
    if (_phase != DictationPhase.idle) return const DictationDenied('Ya hay una grabación en curso.');
    _phase = DictationPhase.recording; // claim the mic before any await
    _owner = owner;
    _notices.remove(owner);
    _elapsedMs = 0;
    _level = 0;
    _cancelled = false;
    notifyListeners();
    final String? problem = await _precheck?.call();
    if (problem != null) return _deny(owner, problem);
    await load();
    await _beforeRecord?.call();
    bool allowed = false;
    try {
      allowed = await _recorder.hasPermission();
    } catch (_) {}
    if (!allowed) return _deny(owner, kMicDenied);
    if (_cancelled) return _deny(owner, null);

    final int now = _now();
    final String dir = await _dir();
    final String id = 'take-$now';
    final PendingTake take = PendingTake(id: id, owner: owner, path: '$dir/$id.m4a', createdAtMs: now);
    try {
      await Directory(dir).create(recursive: true);
      // Meta first: a recording interrupted by a crash is still found.
      await File(take.metaPath).writeAsString(
          jsonEncode(<String, Object?>{'id': id, 'owner': owner, 'createdAtMs': now}),
          flush: true);
      _path = take.path;
      await _recorder.start(take.path);
    } catch (_) {
      _path = null;
      await _rm(take.metaPath);
      return _deny(owner, 'No pude usar el micrófono.');
    }
    _current = take;
    _startedMs = _now();
    final Completer<DictationOutcome> done = _done = Completer<DictationOutcome>();
    if (_cancelled) {
      unawaited(cancel());
      return done.future;
    }
    _detector = SilenceDetector(silence: Duration(milliseconds: (_silenceSeconds() * 1000).round()));
    _levels = _recorder.levels().listen(_onLevel, onError: (Object _) {});
    _cap = _timer(maxDuration, () => unawaited(stop()));
    notifyListeners();
    return done.future;
  }

  DictationOutcome _deny(String owner, String? message) {
    unawaited(_afterRecord?.call());
    _phase = DictationPhase.idle;
    _owner = null;
    if (message != null) _notices[owner] = message;
    notifyListeners();
    return message == null ? const DictationCancelled() : DictationDenied(message);
  }

  void _onLevel(double db) {
    if (_phase != DictationPhase.recording) return;
    final int now = _now();
    _elapsedMs = now - _startedMs;
    _level = SilenceDetector.meter(db);
    notifyListeners();
    if (_detector?.add(db, now) ?? false) unawaited(stop());
  }

  void _stopSignals() {
    unawaited(_levels?.cancel());
    _levels = null;
    _cap?.cancel();
    _cap = null;
    _detector = null;
  }

  /// Ends the recording now and transcribes it.
  Future<void> stop() async {
    if (_phase != DictationPhase.recording || _current == null) return;
    final PendingTake take = _current!;
    _stopSignals();
    _phase = DictationPhase.transcribing;
    _elapsedMs = _now() - _startedMs;
    _level = 0;
    notifyListeners();
    try {
      await _recorder.stop();
    } catch (_) {}
    _path = null;
    unawaited(_afterRecord?.call());
    await _transcribeTake(take);
  }

  /// Discards the recording or transcription in progress.
  Future<void> cancel() async {
    if (_phase == DictationPhase.idle) return;
    _cancelled = true;
    if (_phase == DictationPhase.recording && _current != null) {
      final PendingTake take = _current!;
      _stopSignals();
      try {
        await _recorder.cancel();
      } catch (_) {}
      _path = null;
      unawaited(_afterRecord?.call());
      await _forget(take);
      _finish(const DictationCancelled());
    } else if (_phase == DictationPhase.transcribing) {
      final Completer<void>? a = _abort;
      if (a != null && !a.isCompleted) a.complete();
    }
  }

  /// Transcribes a pending take again.
  Future<DictationOutcome> retry(String takeId) async {
    if (_phase != DictationPhase.idle) return const DictationDenied('Espera a que termine la grabación en curso.');
    final PendingTake? take = _pending.where((PendingTake t) => t.id == takeId).firstOrNull;
    if (take == null) return const DictationCancelled();
    _pending.remove(take);
    _phase = DictationPhase.transcribing;
    _owner = take.owner;
    _cancelled = false;
    _current = take;
    _done = Completer<DictationOutcome>();
    final Future<DictationOutcome> f = _done!.future;
    notifyListeners();
    await _transcribeTake(take);
    return f;
  }

  /// Deletes a pending take (the person chose Descartar).
  Future<void> discard(String takeId) async {
    final PendingTake? take = _pending.where((PendingTake t) => t.id == takeId).firstOrNull;
    if (take == null) return;
    await _forget(take);
    notifyListeners();
  }

  Future<void> _transcribeTake(PendingTake take) async {
    String? text;
    OpenAiException? error;
    for (int i = 0; i <= retryDelays.length; i++) {
      _attempt = i + 1;
      notifyListeners();
      final Completer<void> abort = _abort = Completer<void>();
      try {
        text = await _transcribe(take.path, abort.future);
        error = null;
        break;
      } on OpenAiException catch (e) {
        error = e;
      } catch (_) {
        error = const OpenAiException(OpenAiErrorKind.badResponse);
      }
      if (_cancelled || error.kind == OpenAiErrorKind.aborted) break;
      if (!error.retryable || i == retryDelays.length) break;
      await _delay(retryDelays[i]);
      if (_cancelled) break;
    }
    _abort = null;
    if (_cancelled) {
      await _forget(take);
      _finish(const DictationCancelled());
      return;
    }
    if (error != null || text == null) {
      final OpenAiException e = error ?? const OpenAiException(OpenAiErrorKind.badResponse);
      take.message = e.userMessage;
      _pending.add(take);
      _finish(DictationFailed(take, e.userMessage, e.kind));
      return;
    }
    final String heard = text.trim();
    if (heard.isEmpty) {
      await _forget(take);
      _notices[take.owner] = 'No se oyó nada.';
      _finish(const DictationNothing());
      return;
    }
    _finish(DictationText(heard));
    // The audio goes only once the text is safe with its owner.
    try {
      await deliver(take.owner, heard);
      await _forget(take);
    } catch (_) {
      take.message = 'No pude guardar el texto. La grabación sigue aquí.';
      _pending.add(take);
      notifyListeners();
    }
  }

  void _finish(DictationOutcome outcome) {
    _phase = DictationPhase.idle;
    _owner = null;
    _current = null;
    _attempt = 0;
    final Completer<DictationOutcome>? d = _done;
    _done = null;
    notifyListeners();
    if (d != null && !d.isCompleted) d.complete(outcome);
  }

  @override
  void dispose() {
    _stopSignals();
    super.dispose();
  }
}

/// Inserts dictated [text] at the cursor of [current] (selection replaced),
/// or appends it when there is no cursor. Adds a space where needed.
/// Returns the new text and the cursor offset after the insertion.
(String, int) insertDictation(String current, int selStart, int selEnd, String text) {
  final String t = text.trim();
  if (t.isEmpty) return (current, selEnd < 0 ? current.length : selEnd);
  int start = selStart, end = selEnd;
  if (start < 0 || end < 0 || start > current.length || end > current.length) {
    start = end = current.length;
  }
  if (start > end) (start, end) = (end, start);
  final String before = current.substring(0, start);
  final String after = current.substring(end);
  final String lead = before.isEmpty || RegExp(r'\s$').hasMatch(before) ? '' : ' ';
  final String trail = after.isEmpty || RegExp(r'^\s').hasMatch(after) ? '' : ' ';
  final String inserted = '$lead$t$trail';
  return ('$before$inserted$after', before.length + lead.length + t.length);
}

/// Appends [text] to a draft with one space between.
String appendToDraft(String draft, String text) {
  final String d = draft.trimRight();
  return d.isEmpty ? text.trim() : '$d ${text.trim()}';
}
