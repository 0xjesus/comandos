import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/util/atomic_json_file.dart';

/// Speech-to-text model. `gpt-transcribe` is OpenAI's recommended file
/// transcription model (checked against the docs on 2026-11-20); the
/// `gpt-4o-*-transcribe` family and `whisper-1` are deprecated (shutdown
/// 2027-02-26) and only kept as fallbacks.
const String kDefaultSttModel = 'gpt-transcribe';
const List<String> kSttModels = <String>['gpt-transcribe', 'gpt-4o-mini-transcribe', 'whisper-1'];

/// Text-to-speech model for `POST /v1/audio/speech`. NOTE: OpenAI deprecated
/// every model of this endpoint (shutdown 2027-01-06); see voice/README.md.
const String kDefaultTtsModel = 'gpt-4o-mini-tts';
const List<String> kTtsModels = <String>['gpt-4o-mini-tts', 'tts-1'];

/// Built-in voices of `/v1/audio/speech` (docs: marin and cedar sound best).
const List<String> kVoices = <String>[
  'marin', 'cedar', 'alloy', 'ash', 'ballad', 'coral', 'echo', //
  'fable', 'nova', 'onyx', 'sage', 'shimmer', 'verse',
];

/// `tts-1` supports a smaller set.
const List<String> kTts1Voices = <String>[
  'alloy', 'ash', 'coral', 'echo', 'fable', 'onyx', 'nova', 'sage', 'shimmer', //
];
const String kDefaultVoice = 'marin';

/// Dictation language: Spanish (default), English or let the model detect.
enum SttLanguage {
  es('Español'),
  en('Inglés'),
  auto('Automático');

  const SttLanguage(this.label);
  final String label;
}

/// How much of a reply is read when it is read automatically.
enum ReadMode {
  full('Completo'),
  ending('Sólo el final'),
  summary('Resumen corto');

  const ReadMode(this.label);
  final String label;
}

/// The voice preferences. Immutable; [VoiceSettingsStore] swaps instances.
@immutable
class VoiceSettings {
  const VoiceSettings({
    this.sttModel = kDefaultSttModel,
    this.language = SttLanguage.es,
    this.ttsModel = kDefaultTtsModel,
    this.voice = kDefaultVoice,
    this.speed = 1.0,
    this.autoReadMode = ReadMode.ending,
    this.continuous = false,
    this.carCloudDictation = false,
    this.silenceSeconds = 2.5,
    this.autoReadSessions = const <String>{},
    this.lastRead = const <String, String>{},
  });

  final String sttModel;
  final SttLanguage language;
  final String ttsModel;
  final String voice;

  /// Playback speed, 0.8–1.5×. Applied by the player (pitch preserved), so
  /// changing it never re-synthesises or misses the cache.
  final double speed;
  final ReadMode autoReadMode;

  /// Modo auto: listen again after a reply has been read. (Modo auto now
  /// always works as a call; kept for the settings file.)
  final bool continuous;

  /// Modo auto: transcribe the messages dictated for an agent with OpenAI
  /// (more accurate, costs money). Off: everything in Modo auto uses the
  /// phone's own recognizer and voice, which cost nothing.
  final bool carCloudDictation;

  /// Silence that ends a dictation.
  final double silenceSeconds;

  /// Chats with "Leer respuestas nuevas" on.
  final Set<String> autoReadSessions;

  /// Session → id of the last reply read aloud (never read twice).
  final Map<String, String> lastRead;

  static const double minSpeed = 0.8;
  static const double maxSpeed = 1.5;
  static const int lastReadCap = 300;

  /// The voice actually sent: falls back when the model lacks it.
  String get effectiveVoice =>
      ttsModel.startsWith('tts-1') && !kTts1Voices.contains(voice) ? 'nova' : voice;

  VoiceSettings copyWith({
    String? sttModel,
    SttLanguage? language,
    String? ttsModel,
    String? voice,
    double? speed,
    ReadMode? autoReadMode,
    bool? continuous,
    bool? carCloudDictation,
    double? silenceSeconds,
    Set<String>? autoReadSessions,
    Map<String, String>? lastRead,
  }) =>
      VoiceSettings(
        sttModel: sttModel ?? this.sttModel,
        language: language ?? this.language,
        ttsModel: ttsModel ?? this.ttsModel,
        voice: voice ?? this.voice,
        speed: (speed ?? this.speed).clamp(minSpeed, maxSpeed),
        autoReadMode: autoReadMode ?? this.autoReadMode,
        continuous: continuous ?? this.continuous,
        carCloudDictation: carCloudDictation ?? this.carCloudDictation,
        silenceSeconds: silenceSeconds ?? this.silenceSeconds,
        autoReadSessions: autoReadSessions ?? this.autoReadSessions,
        lastRead: lastRead ?? this.lastRead,
      );

  Map<String, Object?> toJson() => <String, Object?>{
        'v': 1,
        'sttModel': sttModel,
        'language': language.name,
        'ttsModel': ttsModel,
        'voice': voice,
        'speed': speed,
        'autoReadMode': autoReadMode.name,
        'continuous': continuous,
        'carCloudDictation': carCloudDictation,
        'silenceSeconds': silenceSeconds,
        'autoReadSessions': autoReadSessions.toList(),
        'lastRead': lastRead,
      };

  static VoiceSettings fromJson(Object? json) {
    if (json is! Map) return const VoiceSettings();
    T pick<T extends Enum>(List<T> values, Object? name, T fallback) =>
        values.firstWhere((T e) => e.name == name, orElse: () => fallback);
    String str(String k, String fallback, List<String> allowed) {
      final Object? v = json[k];
      return v is String && allowed.contains(v) ? v : fallback;
    }

    double dbl(String k, double fallback) => json[k] is num ? (json[k] as num).toDouble() : fallback;
    final Object? sessions = json['autoReadSessions'];
    final Object? last = json['lastRead'];
    return VoiceSettings(
      sttModel: str('sttModel', kDefaultSttModel, kSttModels),
      language: pick(SttLanguage.values, json['language'], SttLanguage.es),
      ttsModel: str('ttsModel', kDefaultTtsModel, kTtsModels),
      voice: str('voice', kDefaultVoice, kVoices),
      speed: dbl('speed', 1.0).clamp(minSpeed, maxSpeed),
      autoReadMode: pick(ReadMode.values, json['autoReadMode'], ReadMode.ending),
      continuous: json['continuous'] == true,
      carCloudDictation: json['carCloudDictation'] == true,
      silenceSeconds: dbl('silenceSeconds', 2.5).clamp(1.0, 6.0),
      autoReadSessions: sessions is List ? <String>{for (final Object? s in sessions) if (s is String) s} : const <String>{},
      lastRead: last is Map
          ? <String, String>{
              for (final MapEntry<Object?, Object?> e in last.entries)
                if (e.key is String && e.value is String) e.key! as String: e.value! as String,
            }
          : const <String, String>{},
    );
  }
}

/// Loads and persists [VoiceSettings] (atomic JSON file). Loaded lazily on
/// the first voice use; until then it holds the defaults.
class VoiceSettingsStore extends ValueNotifier<VoiceSettings> {
  VoiceSettingsStore(this._path) : super(const VoiceSettings());

  final Future<String?> Function() _path;
  AtomicJsonFile? _file;
  Future<void>? _loading;

  Future<void> load() => _loading ??= _load();

  Future<void> _load() async {
    try {
      final String? p = await _path();
      if (p == null) return;
      _file = AtomicJsonFile(p);
      value = VoiceSettings.fromJson(await _file!.read());
    } catch (_) {
      // Unreadable settings: keep the defaults.
    }
  }

  Future<void> update(VoiceSettings Function(VoiceSettings s) change) async {
    await load();
    value = change(value);
    try {
      await _file?.write(value.toJson());
    } catch (_) {
      // Preferences only; the in-memory value still applies.
    }
  }

  bool autoReads(String session) => value.autoReadSessions.contains(session);

  Future<void> setAutoRead(String session, bool on) => update((VoiceSettings s) {
        final Set<String> next = <String>{...s.autoReadSessions};
        on ? next.add(session) : next.remove(session);
        return s.copyWith(autoReadSessions: next);
      });

  Future<void> markRead(String session, String replyId) => update((VoiceSettings s) {
        final Map<String, String> next = <String, String>{...s.lastRead}..remove(session);
        next[session] = replyId;
        while (next.length > VoiceSettings.lastReadCap) {
          next.remove(next.keys.first);
        }
        return s.copyWith(lastRead: next);
      });
}
