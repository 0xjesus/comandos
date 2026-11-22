import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/outbox/drafts.dart';
import 'audio_out.dart';
import 'dictation.dart';
import 'openai_client.dart';
import 'openai_key_store.dart';
import 'recorder.dart';
import 'speech.dart';
import 'tts_cache.dart';
import 'voice_settings.dart';

/// Everything voice, built on FIRST USE (opening a chat with a key, the
/// settings, or Modo auto), never at app start. Constructing it does no
/// I/O; the key and settings load on demand, the recorder and player exist
/// only while recording / speaking.
class VoiceServices {
  VoiceServices({
    required this.keys,
    required this.settings,
    required this.openai,
    required this.dictation,
    required this.speech,
    required this.cacheDir,
  });

  factory VoiceServices.live() {
    final OpenAiKeyStore keys = OpenAiKeyStore(null, _desktopKey);
    final Future<String> Function() support = _memo(() async => (await getApplicationSupportDirectory()).path);
    final Future<String> Function() cache = _memo(() async => (await getTemporaryDirectory()).path);
    final VoiceSettingsStore settings = VoiceSettingsStore(() async => '${await support()}/voice_settings.json');
    final OpenAiClient openai = OpenAiClient(key: keys.read);
    late final SpeechService speech;
    final DictationController dictation = DictationController(
      recorder: PackageRecorder(),
      transcribe: (String path, Future<void> abort) => openai.transcribe(
        path,
        SttOptions(model: settings.value.sttModel, language: settings.value.language),
        abort: abort,
      ),
      // Recordings wait in app storage (not cache): Android never clears it
      // behind our back, and an untranscribed recording must not vanish.
      dir: () async => '${await support()}/voice/takes',
      fallback: _appendToDraft,
      precheck: () async {
        await Future.wait(<Future<void>>[keys.load(), settings.load()]);
        return (await keys.read()) == null ? const OpenAiException(OpenAiErrorKind.missingKey).userMessage : null;
      },
      beforeRecord: () async {
        await speech.stop(); // recording stops read-aloud first
        await holdFocusForRecording();
      },
      afterRecord: releaseAudioFocus,
      silenceSeconds: () => settings.value.silenceSeconds,
    );
    TtsOptions tts() => TtsOptions(model: settings.value.ttsModel, voice: settings.value.effectiveVoice);
    speech = SpeechService(
      synth: (String text, Future<void> abort) => openai.speech(text, tts(), abort: abort),
      cache: TtsCache(() async => '${await cache()}/tts'),
      voiceSignature: () => tts().signature,
      audio: JustAudioOut(),
      settings: () => settings.value,
      onNeedsKey: () => onNeedsKey?.call(),
    );
    return VoiceServices(
      keys: keys,
      settings: settings,
      openai: openai,
      dictation: dictation,
      speech: speech,
      cacheDir: cache,
    );
  }

  final OpenAiKeyStore keys;
  final VoiceSettingsStore settings;
  final OpenAiClient openai;
  final DictationController dictation;
  final SpeechService speech;
  final Future<String> Function() cacheDir;

  /// Set by voice.dart: opens Ajustes › Voz when something needs the key.
  static VoidCallback? onNeedsKey;

  static VoiceServices? _instance;

  /// The services, created on first access.
  static VoiceServices get instance => _instance ??= VoiceServices.live();

  /// The services if anything already created them (no side effect).
  static VoiceServices? get existing => _instance;

  @visibleForTesting
  static set instance(VoiceServices? s) => _instance = s;

  Future<void>? _ready;

  /// Key + settings loaded.
  Future<void> ready() => _ready ??= Future.wait(<Future<void>>[keys.load(), settings.load()]);

  static Future<String> Function() _memo(Future<String> Function() f) {
    Future<String>? v;
    return () => v ??= f();
  }

  /// `GET /voice/openai-key` of the paired desktop: its key, or null.
  static Future<String?> _desktopKey() async {
    final ComandosApi? api = sl.maybe<Connection>()?.api;
    if (api == null) return null;
    final ApiResponse r = await api.request('GET', '/voice/openai-key');
    final Object? json = r.ok ? r.json : null;
    return json is Map && json['key'] is String ? json['key'] as String : null;
  }

  static Future<void> _appendToDraft(String owner, String text) async {
    final DraftStore? drafts = sl.maybe<DraftStore>();
    if (drafts == null || owner.startsWith('_')) return;
    await drafts.setNow(owner, appendToDraft(drafts.get(owner), text));
  }
}
