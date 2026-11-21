import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

// Audio focus: speech that ducks other audio.
import 'package:audio_session/audio_session.dart';
import 'package:just_audio/just_audio.dart';

import 'speech.dart';

const AudioSessionConfiguration _session = AudioSessionConfiguration(
  androidAudioAttributes: AndroidAudioAttributes(
    contentType: AndroidAudioContentType.speech,
    usage: AndroidAudioUsage.assistanceNavigationGuidance,
  ),
  androidAudioFocusGainType: AndroidAudioFocusGainType.gainTransientMayDuck,
  androidWillPauseWhenDucked: true,
);

bool _sessionConfigured = false;

Future<void> _ensureSession() async {
  if (_sessionConfigured) return;
  _sessionConfigured = true;
  try {
    await (await AudioSession.instance).configure(_session);
  } catch (_) {}
}

/// While the mic records, other audio (music in the car) pauses so it is
/// not recorded on top of the voice: transient exclusive focus.
Future<void> holdFocusForRecording() async {
  await _ensureSession();
  try {
    await (await AudioSession.instance)
        .setActive(true, androidAudioFocusGainType: AndroidAudioFocusGainType.gainTransientExclusive);
  } catch (_) {}
}

/// Gives audio focus back (other apps un-duck / resume).
Future<void> releaseAudioFocus() async {
  try {
    await (await AudioSession.instance).setActive(false);
  } catch (_) {}
}

/// just_audio player, created on first play and freed by [dispose] (the
/// speech service disposes it after 30 s idle). Audio focus: transient,
/// "may duck" (music lowers its volume instead of stopping), navigation
/// guidance usage, so it behaves like a GPS voice over the car's Bluetooth.
/// Routing follows Android's default (Bluetooth when connected).
class JustAudioOut implements AudioOut {
  AudioPlayer? _player;

  @override
  Future<bool> play(String path, double speed) async {
    await _ensureSession();
    final AudioPlayer p = _player ??= AudioPlayer();
    try {
      await p.setFilePath(path);
      await p.setSpeed(speed);
      await p.play(); // completes at the end, or when stopped/paused
      final bool ended = p.processingState == ProcessingState.completed;
      await p.pause(); // play() leaves `playing` true after completion
      return ended;
    } catch (_) {
      return false;
    }
  }

  @override
  Future<void> stop() async {
    try {
      await _player?.stop();
    } catch (_) {}
  }

  @override
  Future<void> release() => releaseAudioFocus();

  @override
  Future<void> dispose() async {
    final AudioPlayer? p = _player;
    _player = null;
    try {
      await p?.dispose();
    } catch (_) {}
  }
}

/// A short two-tone "your turn" cue as a WAV file (generated, no asset).
Future<String> earconFile(String dir) async {
  final File f = File('$dir/earcon-v1.wav');
  if (await f.exists()) return f.path;
  await Directory(dir).create(recursive: true);
  await f.writeAsBytes(_tone(), flush: true);
  return f.path;
}

Uint8List _tone() {
  const int rate = 22050;
  const List<(double, int)> notes = <(double, int)>[(660, 90), (990, 120)];
  final List<int> samples = <int>[];
  for (final (double hz, int ms) in notes) {
    final int n = rate * ms ~/ 1000;
    for (int i = 0; i < n; i++) {
      final double env = min(1.0, min(i, n - i) / (rate * 0.01));
      samples.add((sin(2 * pi * hz * i / rate) * env * 0.35 * 32767).round());
    }
  }
  final ByteData b = ByteData(44 + samples.length * 2);
  void str(int o, String s) {
    for (int i = 0; i < s.length; i++) {
      b.setUint8(o + i, s.codeUnitAt(i));
    }
  }

  str(0, 'RIFF');
  b.setUint32(4, 36 + samples.length * 2, Endian.little);
  str(8, 'WAVE');
  str(12, 'fmt ');
  b.setUint32(16, 16, Endian.little);
  b.setUint16(20, 1, Endian.little);
  b.setUint16(22, 1, Endian.little);
  b.setUint32(24, rate, Endian.little);
  b.setUint32(28, rate * 2, Endian.little);
  b.setUint16(32, 2, Endian.little);
  b.setUint16(34, 16, Endian.little);
  str(36, 'data');
  b.setUint32(40, samples.length * 2, Endian.little);
  for (int i = 0; i < samples.length; i++) {
    b.setInt16(44 + i * 2, samples[i], Endian.little);
  }
  return b.buffer.asUint8List();
}
