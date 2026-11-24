import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import '../../core/api/api_client.dart';
import '../voice/audio_out.dart';
import '../voice/speech.dart';
import '../voice/voice_settings.dart';
import 'on_device.dart';

/// Edge through the paired desktop (`POST /tts`), for [EdgeVoice.render].
Future<Uint8List> Function(String, String, String)? edgeThrough(ComandosApi? api) => api == null
    ? null
    : (String text, String voice, String rate) => api.speech(text: text, voice: voice, rate: rate);

/// Voices the Modo auto can speak with. Empty name: the phone's own voice.
const List<(String, String)> kCarVoices = <(String, String)>[
  ('es-MX-DaliaNeural', 'Dalia · Edge, México (predeterminada)'),
  ('es-MX-JorgeNeural', 'Jorge · Edge, México'),
  ('es-US-PalomaNeural', 'Paloma · Edge, EE. UU.'),
  ('es-US-AlonsoNeural', 'Alonso · Edge, EE. UU.'),
  ('', 'Voz del teléfono (sin internet, privada)'),
];

/// `edge-tts --rate` for a speed setting (1.0 → "+0%", 1.2 → "+20%").
String edgeRate(double speed) {
  final int pct = ((speed - 1) * 100).round().clamp(-50, 99);
  return pct < 0 ? '$pct%' : '+$pct%';
}

/// Edge's neural voice (free, rendered on the desktop by `/tts`), with the
/// phone's voice as the fallback: when Edge fails or is slow the phone speaks
/// at once and keeps speaking for [cooldown], so a bad network never leaves
/// the driver waiting in silence. An empty [voice] is always the phone.
class EdgeVoice implements DeviceSpeech {
  EdgeVoice({
    required this.render,
    required this.voice,
    required this.phone,
    required this.dir,
    AudioOut? out,
    this.cooldown = const Duration(minutes: 2),
    this.wait = const Duration(seconds: 6),
  }) : _out = out ?? JustAudioOut();

  /// Renders [text] with an Edge voice (mp3); null when there is no desktop
  /// to ask (not paired).
  final Future<Uint8List> Function(String text, String voice, String rate)? Function() render;
  final String Function() voice;
  final DeviceSpeech phone;
  final Future<String> Function() dir;
  final Duration cooldown;
  final Duration wait;
  final AudioOut _out;

  DateTime? _failed;
  int _gen = 0;

  /// Short phrases repeat a lot («Enviado.», «¿Lo envío?»): their audio is
  /// kept (voice + speed + text → file) so they sound at once.
  final Map<String, String> _cache = <String, String>{};
  static const int _cacheMax = 40;
  static const int _cacheChars = 120;

  bool get _edge {
    final DateTime? f = _failed;
    return voice().isNotEmpty && render() != null && (f == null || DateTime.now().difference(f) > cooldown);
  }

  @override
  Future<bool> speak(String text, {required SttLanguage language, required double speed}) async {
    final String t = text.trim();
    if (t.isEmpty) return true;
    final int gen = ++_gen;
    if (!_edge) return phone.speak(t, language: language, speed: speed);
    final String key = '${voice()}|${edgeRate(speed)}|$t';
    final String? cached = _cache[key];
    if (cached != null && File(cached).existsSync()) return _out.play(cached, 1.0);
    final Future<Uint8List> Function(String, String, String) synth = render()!;
    final Uint8List mp3;
    try {
      mp3 = await synth(t, voice(), edgeRate(speed)).timeout(wait);
    } catch (_) {
      _failed = DateTime.now();
      if (gen != _gen) return false; // stopped meanwhile
      return phone.speak(t, language: language, speed: speed);
    }
    if (gen != _gen) return false;
    final bool keep = t.length <= _cacheChars;
    final File f = File('${await dir()}/edge-${keep ? 'c${_cache.length % _cacheMax}' : 'voice'}.mp3');
    try {
      await f.writeAsBytes(mp3, flush: true);
    } catch (_) {
      return phone.speak(t, language: language, speed: speed);
    }
    if (gen != _gen) return false;
    if (keep) {
      _cache.removeWhere((String _, String path) => path == f.path);
      if (_cache.length >= _cacheMax) _cache.remove(_cache.keys.first);
      _cache[key] = f.path;
    }
    return _out.play(f.path, 1.0);
  }

  @override
  Future<void> stop() async {
    _gen++;
    await Future.wait(<Future<void>>[_out.stop(), phone.stop()]);
  }
}
