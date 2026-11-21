import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/util/clock.dart';
import 'openai_client.dart';
import 'text_prep.dart';
import 'tts_cache.dart';
import 'voice.dart';
import 'voice_settings.dart';

/// Plays one audio file at a time. Implemented with just_audio in
/// `audio_out.dart`; faked in tests.
abstract class AudioOut {
  /// Plays [path]; completes when it ends (true) or is stopped /
  /// interrupted (false).
  Future<bool> play(String path, double speed);
  Future<void> stop();

  /// Gives audio focus back (other apps un-duck / resume).
  Future<void> release();

  /// Frees the native player; [play] recreates it.
  Future<void> dispose();
}

enum SpeakResult { completed, stopped, failed }

/// Synthesises one chunk. Throws [OpenAiException].
typedef Synthesizer = Future<Uint8List> Function(String text, Future<void> abort);

/// Read-aloud engine: splits a reply, synthesises the first chunk at once
/// and prefetches the next while the current one plays, so speech starts
/// fast. Chunks are cached on disk. [stop] cancels pending requests and the
/// playback immediately.
class SpeechService extends ChangeNotifier implements Speaker {
  SpeechService({
    required Synthesizer synth,
    required TtsCache cache,
    required String Function() voiceSignature,
    required AudioOut audio,
    required VoiceSettings Function() settings,
    TimerFactory timer = systemTimer,
    this.idleDispose = const Duration(seconds: 30),
    this.onNeedsKey,
  })  : _synth = synth,
        _cache = cache,
        _voiceSignature = voiceSignature,
        _audio = audio,
        _settings = settings,
        _timer = timer;

  final Synthesizer _synth;
  final TtsCache _cache;
  final String Function() _voiceSignature;
  final AudioOut _audio;
  final VoiceSettings Function() _settings;
  final TimerFactory _timer;
  final Duration idleDispose;

  /// Called instead of speaking when there is no key (opens Ajustes › Voz).
  final VoidCallback? onNeedsKey;

  int _gen = 0;
  Completer<void>? _abort;
  bool _speaking = false;
  OpenAiException? _lastError;
  Timer? _idle;

  /// True from the first request until the last chunk ends.
  bool get speaking => _speaking;
  OpenAiException? get lastError => _lastError;

  /// "Leer en voz alta" from a message: the whole reply.
  @override
  Future<void> speak(String text) async {
    await speakReply(text, mode: ReadMode.full);
  }

  /// Reads a reply (markdown) using [mode].
  Future<SpeakResult> speakReply(String markdown, {required ReadMode mode}) =>
      speakChunks(speechChunksFor(markdown, mode));

  /// Speaks plain, short text (announcements, confirmations).
  Future<SpeakResult> say(String text) => speakChunks(chunkForSpeech(text));

  Future<SpeakResult> speakChunks(List<String> chunks) async {
    await _halt();
    if (chunks.isEmpty) return SpeakResult.completed;
    final int gen = ++_gen;
    final Completer<void> abort = _abort = Completer<void>();
    _idle?.cancel();
    _speaking = true;
    _lastError = null;
    notifyListeners();
    final String sig = _voiceSignature();

    Future<String> fetch(String chunk) async {
      final String key = fnv64('$sig|$chunk');
      final File? hit = await _cache.get(key);
      if (hit != null) return hit.path;
      Uint8List bytes;
      try {
        bytes = await _synth(chunk, abort.future);
      } on OpenAiException catch (e) {
        if (!e.retryable || abort.isCompleted) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 600));
        if (abort.isCompleted) throw const OpenAiException(OpenAiErrorKind.aborted);
        bytes = await _synth(chunk, abort.future);
      }
      return (await _cache.put(key, bytes)).path;
    }

    SpeakResult result = SpeakResult.completed;
    Future<String>? next = fetch(chunks.first);
    try {
      for (int i = 0; i < chunks.length; i++) {
        final String path = await next!;
        if (gen != _gen) return result = SpeakResult.stopped;
        next = i + 1 < chunks.length ? fetch(chunks[i + 1]) : null;
        // Mark handled now; the error (if any) is rethrown by the await above.
        next?.ignore();
        final bool ended = await _audio.play(path, _settings().speed);
        if (gen != _gen || !ended) return result = SpeakResult.stopped;
      }
      return result;
    } on OpenAiException catch (e) {
      if (gen != _gen || e.kind == OpenAiErrorKind.aborted) return result = SpeakResult.stopped;
      _lastError = e;
      if (e.kind == OpenAiErrorKind.missingKey) onNeedsKey?.call();
      return result = SpeakResult.failed;
    } catch (_) {
      if (gen != _gen) return result = SpeakResult.stopped;
      _lastError = const OpenAiException(OpenAiErrorKind.badResponse);
      return result = SpeakResult.failed;
    } finally {
      if (gen == _gen) {
        _speaking = false;
        if (result != SpeakResult.stopped) unawaited(_audio.release());
        _scheduleIdle();
        notifyListeners();
      }
    }
  }

  /// Synthesises fixed phrases into the cache ahead of time (one by one,
  /// skipping cached ones), so short feedback such as "Sin conexión…" can
  /// still be spoken when the network is gone. Errors are ignored.
  Future<void> warm(List<String> phrases) async {
    final String sig = _voiceSignature();
    for (final String p in phrases) {
      for (final String chunk in chunkForSpeech(p)) {
        final String key = fnv64('$sig|$chunk');
        try {
          if (await _cache.get(key) != null) continue;
          await _cache.put(key, await _synth(chunk, Completer<void>().future));
        } catch (_) {
          return; // no key / offline / quota: try another time
        }
      }
    }
  }

  /// Plays a local file (earcon) without touching the queue state.
  Future<void> playFile(String path) async {
    await _halt();
    _idle?.cancel();
    await _audio.play(path, 1.0);
    _scheduleIdle();
  }

  Future<void> _halt() async {
    _gen++;
    final Completer<void>? a = _abort;
    _abort = null;
    if (a != null && !a.isCompleted) a.complete();
    if (_speaking) {
      _speaking = false;
      notifyListeners();
    }
    await _audio.stop();
  }

  /// Stops speech at once and gives audio focus back.
  @override
  Future<void> stop() async {
    final bool was = _speaking;
    await _halt();
    if (was) await _audio.release();
    _scheduleIdle();
  }

  void _scheduleIdle() {
    _idle?.cancel();
    _idle = _timer(idleDispose, () => unawaited(_audio.dispose()));
  }

  @override
  void dispose() {
    _idle?.cancel();
    unawaited(_audio.dispose());
    super.dispose();
  }
}
