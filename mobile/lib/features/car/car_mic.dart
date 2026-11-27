import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/platform/native_channel.dart';
import '../voice/recorder.dart';
import 'wake_phrase.dart';

/// What one take of Modo auto's microphone gave.
class MicTake {
  const MicTake({required this.end, this.path, this.text = '', this.words = false, this.error});

  /// Why it ended: `silence`, `nowords`, `max`, `stopped` (asked), `error`
  /// or `cancelled`.
  final String end;

  /// The WAV (16 kHz mono), when something was recorded.
  final String? path;

  /// What the offline recognizer heard, without the wake phrase.
  final String text;

  /// The offline recognizer heard words of the turn (the wake phrase does not count).
  final bool words;
  final String? error;

  bool get cancelled => end == 'cancelled';

  /// Nothing could be recorded (no model, microphone busy).
  bool get unusable => path == null && end == 'error';

  static MicTake from(Map<Object?, Object?>? m) {
    if (m == null) return const MicTake(end: 'error', error: 'Sin micrófono de Modo auto.');
    String? str(String k) => m[k] is String && (m[k] as String).isNotEmpty ? m[k] as String : null;
    return MicTake(
      end: str('end') ?? 'error',
      path: str('path'),
      text: str('text') ?? '',
      words: m['words'] == true,
      error: str('error'),
    );
  }
}

/// Modo auto's own microphone (`CarListener.kt`): the offline listener that
/// hears «OK ComandOS» and then, without letting the microphone go, records
/// the person's turn and ends it by the words it hears. Faked in tests.
abstract class CarMic {
  /// Input level 0..1 while a take records.
  ValueListenable<double> get level;

  /// Records the person's turn. [afterWake]: what matched the wake phrase
  /// (the listener is still running; the take keeps ~2 s from before it).
  /// [silence] of no new words ends it.
  Future<MicTake> take({required Duration silence, String? afterWake});

  /// Ends the take now; it completes with what was said so far.
  Future<void> end();

  /// Drops the take; it completes as cancelled and its file is deleted.
  Future<void> cancel();

  /// Frees the microphone (the phone's recognizer needs it).
  Future<void> release();
}

/// [CarMic] on the phone.
class NativeCarMic implements CarMic {
  NativeCarMic({required this.dir, NativeChannel? native}) : _native = native ?? NativeChannel.instance;

  /// Where takes are written (the dictation's takes folder).
  final Future<String> Function() dir;
  final NativeChannel _native;
  final ValueNotifier<double> _level = ValueNotifier<double>(0);

  /// No word at all for this long: the take ends empty.
  static const Duration noWords = Duration(seconds: 8);

  /// The longest turn.
  static const Duration longest = Duration(minutes: 10);

  /// Audio kept from before the take started (the wake phrase and what
  /// followed it in the same breath).
  static const Duration preRoll = Duration(milliseconds: 2500);

  @override
  ValueListenable<double> get level => _level;

  @override
  Future<MicTake> take({required Duration silence, String? afterWake}) async {
    _native.onTakeLevel = (double db) => _level.value = SilenceDetector.meter(db);
    try {
      final String d = await dir();
      await Directory(d).create(recursive: true);
      return MicTake.from(await _native.startTake(
        dir: d,
        silenceMs: silence.inMilliseconds,
        noWordsMs: noWords.inMilliseconds,
        maxMs: longest.inMilliseconds,
        preRollMs: preRoll.inMilliseconds,
        skip: afterWake == null ? 0 : wakePhraseWords(afterWake),
      ));
    } catch (_) {
      return const MicTake(end: 'error', error: 'No pude usar el micrófono.');
    } finally {
      _level.value = 0;
    }
  }

  @override
  Future<void> end() => _native.endTake();

  @override
  Future<void> cancel() => _native.cancelTake();

  @override
  Future<void> release() => _native.stopWake();
}
