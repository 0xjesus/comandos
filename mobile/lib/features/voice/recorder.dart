import 'dart:async';

import 'package:record/record.dart';

/// The microphone, behind an interface so dictation is testable.
abstract class Recorder {
  /// Asks for the permission if needed. False = denied.
  Future<bool> hasPermission();

  /// Starts recording to [path].
  Future<void> start(String path);

  /// Input level in dBFS (≈ -160 silence … 0 loudest), about every 100 ms.
  Stream<double> levels();

  /// Finishes the file. Returns its path (null if nothing was recorded).
  Future<String?> stop();

  /// Stops and deletes the file.
  Future<void> cancel();
}

/// `record` package implementation. The native recorder exists only while
/// recording: created in [start], disposed in [stop] / [cancel].
class PackageRecorder implements Recorder {
  AudioRecorder? _rec;

  /// Compact speech settings: AAC-LC in .m4a, mono, 16 kHz, 32 kbps
  /// (≈ 240 KB per minute: 20 minutes are ≈ 5 MB, under OpenAI's 25 MB).
  static const RecordConfig config = RecordConfig(
    encoder: AudioEncoder.aacLc,
    bitRate: 32000,
    sampleRate: 16000,
    numChannels: 1,
    noiseSuppress: true,
    autoGain: true,
  );

  @override
  Future<bool> hasPermission() async {
    final AudioRecorder r = AudioRecorder();
    try {
      return await r.hasPermission();
    } finally {
      unawaited(r.dispose());
    }
  }

  @override
  Future<void> start(String path) async {
    final AudioRecorder r = _rec ??= AudioRecorder();
    await r.start(config, path: path);
  }

  @override
  Stream<double> levels() {
    final AudioRecorder? r = _rec;
    if (r == null) return const Stream<double>.empty();
    return r.onAmplitudeChanged(const Duration(milliseconds: 100)).map((Amplitude a) => a.current);
  }

  @override
  Future<String?> stop() async {
    final AudioRecorder? r = _rec;
    _rec = null;
    if (r == null) return null;
    try {
      return await r.stop();
    } finally {
      unawaited(r.dispose());
    }
  }

  @override
  Future<void> cancel() async {
    final AudioRecorder? r = _rec;
    _rec = null;
    if (r == null) return;
    try {
      await r.cancel();
    } finally {
      unawaited(r.dispose());
    }
  }
}

/// Decides when a dictation is over from the input level. Adapts to the
/// background noise (a moving car is not silent): speech is anything
/// [speechMarginDb] above the tracked noise floor; once speech was heard,
/// [silence] of quiet ends the take. If nothing is heard for [noSpeech], it
/// ends too (the audio is still transcribed: a quiet voice is not discarded).
class SilenceDetector {
  SilenceDetector({
    required this.silence,
    this.noSpeech = const Duration(seconds: 10),
    this.speechMarginDb = 14,
    this.minSpeechDb = -55,
  });

  final Duration silence;
  final Duration noSpeech;
  final double speechMarginDb;
  final double minSpeechDb;

  double? _floor;
  int? _startMs;
  int? _lastLoudMs;

  bool get heardSpeech => _lastLoudMs != null;

  /// Feeds one level sample; true when the take should end now.
  bool add(double db, int atMs) {
    _startMs ??= atMs;
    final double level = db.isFinite ? db.clamp(-120.0, 0.0) : -120.0;
    final double floor = _floor ?? level;
    final bool speech = level >= minSpeechDb && level >= floor + speechMarginDb;
    // The floor follows quiet at once and noise slowly, and barely moves
    // while someone speaks: a minute of talking without a pause must not
    // become the new «background» (it used to after ~4 s and cut the take).
    _floor = level < floor ? level : floor + (level - floor) * (speech ? 0.0005 : 0.02);
    if (speech) _lastLoudMs = atMs;
    final int? loud = _lastLoudMs;
    if (loud != null) return atMs - loud >= silence.inMilliseconds;
    return atMs - _startMs! >= noSpeech.inMilliseconds;
  }

  /// 0..1 meter value for the UI.
  static double meter(double db) => db.isFinite ? ((db + 60) / 60).clamp(0.0, 1.0) : 0.0;
}
