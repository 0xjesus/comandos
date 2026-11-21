import 'dart:async';

import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/openai_client.dart';
import 'package:comandos_mobile/features/voice/openai_key_store.dart';
import 'package:comandos_mobile/features/voice/recorder.dart';
import 'package:comandos_mobile/features/voice/speech.dart';
import 'package:comandos_mobile/features/voice/tts_cache.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:comandos_mobile/features/voice/voice_settings.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

/// Manual clock + timers: nothing fires until [elapse].
class FakeTimers {
  int nowMs = 1700000000000;
  final List<_FakeTimer> _timers = <_FakeTimer>[];

  int now() => nowMs;

  Timer call(Duration d, void Function() cb) {
    final _FakeTimer t = _FakeTimer(nowMs + d.inMilliseconds, cb, this);
    _timers.add(t);
    return t;
  }

  int get pending => _timers.length;

  /// Advances time, firing due timers in order (and microtasks between).
  Future<void> elapse(Duration d) async {
    final int end = nowMs + d.inMilliseconds;
    while (true) {
      await flush();
      _timers.sort((_FakeTimer a, _FakeTimer b) => a.due.compareTo(b.due));
      final _FakeTimer? next = _timers.where((_FakeTimer t) => t.due <= end).firstOrNull;
      if (next == null) break;
      nowMs = next.due;
      _timers.remove(next);
      next.cb();
    }
    nowMs = end;
    await flush();
  }
}

class _FakeTimer implements Timer {
  _FakeTimer(this.due, this.cb, this.owner);
  final int due;
  final void Function() cb;
  final FakeTimers owner;

  @override
  void cancel() => owner._timers.remove(this);
  @override
  bool get isActive => owner._timers.contains(this);
  @override
  int get tick => 0;
}

/// Lets pending microtasks and short async chains run.
Future<void> flush([int rounds = 20]) async {
  for (int i = 0; i < rounds; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

/// Like [flush] but also lets real file I/O finish.
Future<void> settle([int rounds = 10]) async {
  for (int i = 0; i < rounds; i++) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
}

/// Polls until [cond] holds (real time; for steps that do file I/O).
Future<void> waitFor(bool Function() cond, {Duration timeout = const Duration(seconds: 5)}) async {
  final DateTime end = DateTime.now().add(timeout);
  while (!cond()) {
    if (DateTime.now().isAfter(end)) throw StateError('condition not met in time');
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
  await flush();
}

class FakeRecorder implements Recorder {
  bool allow = true;
  final StreamController<double> levelsCtl = StreamController<double>.broadcast();
  String? path;
  int starts = 0;
  int cancels = 0;

  @override
  Future<bool> hasPermission() async => allow;

  @override
  Future<void> start(String p) async {
    path = p;
    starts++;
  }

  @override
  Stream<double> levels() => levelsCtl.stream;

  @override
  Future<String?> stop() async => path;

  @override
  Future<void> cancel() async => cancels++;
}

class FakeAudioOut implements AudioOut {
  final List<String> played = <String>[];
  Completer<bool>? current;
  bool autoComplete = true;
  int stops = 0;
  int releases = 0;
  int disposes = 0;

  @override
  Future<bool> play(String path, double speed) {
    played.add(path);
    if (autoComplete) return Future<bool>.value(true);
    return (current = Completer<bool>()).future;
  }

  /// Ends the current file naturally.
  void finish() {
    final Completer<bool>? c = current;
    current = null;
    if (c != null && !c.isCompleted) c.complete(true);
  }

  @override
  Future<void> stop() async {
    stops++;
    final Completer<bool>? c = current;
    current = null;
    if (c != null && !c.isCompleted) c.complete(false);
  }

  @override
  Future<void> release() async => releases++;

  @override
  Future<void> dispose() async => disposes++;
}

/// VoiceServices with fakes everywhere; never touches a platform plugin or
/// the network. [key] null = no key saved.
VoiceServices testVoiceServices({
  required String dir,
  String? key,
  http.Client? client,
  Recorder? recorder,
  AudioOut? audio,
}) {
  final MemorySecretStore secrets = MemorySecretStore();
  if (key != null) secrets.values['comandos.openai.v1'] = key;
  final OpenAiKeyStore keys = OpenAiKeyStore(secrets);
  final VoiceSettingsStore settings = VoiceSettingsStore(() async => null);
  final OpenAiClient openai = OpenAiClient(
    key: keys.read,
    clientFactory: () => client ?? MockClient((http.Request r) async => http.Response('{}', 500)),
  );
  final SpeechService speech = SpeechService(
    synth: (String t, Future<void> a) => openai.speech(t, const TtsOptions(model: kDefaultTtsModel, voice: kDefaultVoice), abort: a),
    cache: TtsCache(() async => '$dir/tts'),
    voiceSignature: () => 'test',
    audio: audio ?? FakeAudioOut(),
    settings: () => settings.value,
  );
  final DictationController dictation = DictationController(
    recorder: recorder ?? FakeRecorder(),
    transcribe: (String p, Future<void> a) =>
        openai.transcribe(p, const SttOptions(model: kDefaultSttModel, language: SttLanguage.es), abort: a),
    dir: () async => '$dir/takes',
    fallback: (String o, String t) async {},
  );
  return VoiceServices(
    keys: keys,
    settings: settings,
    openai: openai,
    dictation: dictation,
    speech: speech,
    cacheDir: () async => dir,
  );
}
