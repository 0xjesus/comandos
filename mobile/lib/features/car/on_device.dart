import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_tts/flutter_tts.dart';
import 'package:speech_to_text/speech_recognition_error.dart';
import 'package:speech_to_text/speech_recognition_result.dart';
import 'package:speech_to_text/speech_to_text.dart';

import '../voice/voice_settings.dart';

/// Android's own text-to-speech (free, offline with the installed voices).
abstract class DeviceSpeech {
  /// Speaks [text]; true only when it played to the end.
  Future<bool> speak(String text, {required SttLanguage language, required double speed});
  Future<void> stop();
}

/// Android's own speech recognizer (free; on-device when the phone has
/// the offline language pack).
abstract class DeviceListener {
  /// Listens once. Completes with what was heard ('' = nothing), or null
  /// when nothing can listen (no recognizer, no microphone permission):
  /// then [problem] says why. [until] ends the listen early as soon as a
  /// partial result satisfies it (a command said mid-countdown).
  Future<String?> listen({
    required Duration listenFor,
    required Duration pauseFor,
    required bool dictation,
    required SttLanguage language,
    bool Function(String partial)? until,
  });

  /// Ends the listen now, keeping what was heard.
  Future<void> stop();

  /// Ends the listen now; it completes with ''.
  Future<void> cancel();

  String? get problem;
}

/// The device engines used by Modo auto (lazily created; tests override).
abstract final class DeviceVoice {
  static DeviceSpeech? _tts;
  static DeviceListener? _stt;

  static DeviceSpeech get tts => _tts ??= FlutterTtsSpeech();
  static DeviceListener get stt => _stt ??= SpeechToTextListener();

  @visibleForTesting
  static void override({DeviceSpeech? tts, DeviceListener? stt}) {
    _tts = tts;
    _stt = stt;
  }
}

String? _ttsLanguage(SttLanguage l) => switch (l) {
      SttLanguage.es => 'es-MX',
      SttLanguage.en => 'en-US',
      SttLanguage.auto => null,
    };

/// [DeviceSpeech] with flutter_tts. Completion comes from the plugin's
/// handlers (not `awaitSpeakCompletion`, which never completes on some
/// engine errors), with a length-based safety timeout.
class FlutterTtsSpeech implements DeviceSpeech {
  final FlutterTts _tts = FlutterTts();
  Future<void>? _setup;
  Completer<bool>? _done;
  Timer? _guard;
  SttLanguage? _language;

  Future<void> _init() async {
    await _tts.awaitSpeakCompletion(false);
    _tts.setCompletionHandler(() => _end(true));
    _tts.setCancelHandler(() => _end(false));
    _tts.setErrorHandler((dynamic _) => _end(false));
  }

  Future<void> _setLanguage(SttLanguage l) async {
    if (_language == l) return;
    _language = l;
    final String? want = _ttsLanguage(l);
    if (want == null) return;
    final List<String> tries = l == SttLanguage.es ? <String>[want, 'es-US', 'es-ES'] : <String>[want];
    for (final String t in tries) {
      try {
        if (await _tts.isLanguageAvailable(t) == true) {
          await _tts.setLanguage(t);
          return;
        }
      } catch (_) {}
    }
  }

  void _end(bool ok) {
    _guard?.cancel();
    _guard = null;
    final Completer<bool>? c = _done;
    _done = null;
    if (c != null && !c.isCompleted) c.complete(ok);
  }

  @override
  Future<bool> speak(String text, {required SttLanguage language, required double speed}) async {
    final String t = text.trim();
    if (t.isEmpty) return true;
    _end(false);
    try {
      await (_setup ??= _init());
      await _setLanguage(language);
      // flutter_tts on Android: 0.5 is the normal rate.
      await _tts.setSpeechRate((0.5 * speed).clamp(0.3, 1.0));
      final Completer<bool> c = _done = Completer<bool>();
      _guard = Timer(Duration(seconds: 15 + t.length ~/ 8), () => _end(false));
      final Object? r = await _tts.speak(t, focus: true);
      if (r != 1 && identical(_done, c)) _end(false);
      return await c.future;
    } catch (_) {
      _end(false);
      return false;
    }
  }

  @override
  Future<void> stop() async {
    _end(false);
    try {
      await _tts.stop();
    } catch (_) {}
  }
}

/// [DeviceListener] with speech_to_text.
class SpeechToTextListener implements DeviceListener {
  final SpeechToText _s = SpeechToText();
  Future<bool>? _ready;
  final Map<SttLanguage, String?> _locales = <SttLanguage, String?>{};
  Completer<String>? _done;
  Timer? _guard;
  String _last = '';
  bool Function(String)? _until;
  String? _problem;

  /// A stop / cancel in flight: the next listen starts after it.
  Future<void>? _stopping;

  /// The current listen reached "listening": only then do "done" and
  /// errors belong to it (not to the session cancelled just before).
  bool _live = false;

  @override
  String? get problem => _problem;

  Future<bool> _init() async {
    try {
      final bool ok = await _s.initialize(
        onError: _onError,
        onStatus: _onStatus,
        finalTimeout: const Duration(milliseconds: 800),
      );
      if (!ok) {
        _problem = 'No puedo escucharte: revisa el permiso de micrófono o instala el reconocimiento de voz de Google.';
        _ready = null; // try again next time (the permission may be granted)
      }
      return ok;
    } catch (_) {
      _problem = 'Este teléfono no tiene reconocimiento de voz.';
      _ready = null;
      return false;
    }
  }

  Future<String?> _locale(SttLanguage l) async {
    if (l == SttLanguage.auto) return null;
    if (_locales.containsKey(l)) return _locales[l];
    String? pick;
    try {
      final List<LocaleName> all = await _s.locales();
      String norm(String id) => id.replaceAll('-', '_').toLowerCase();
      final String lang = l == SttLanguage.es ? 'es' : 'en';
      final LocaleName? sys = await _s.systemLocale();
      if (sys != null && norm(sys.localeId).startsWith(lang)) {
        pick = sys.localeId;
      } else {
        final List<String> prefer = lang == 'es' ? <String>['es_mx', 'es_us', 'es_es'] : <String>['en_us', 'en_gb'];
        for (final String p in prefer) {
          pick ??= all.where((LocaleName x) => norm(x.localeId) == p).firstOrNull?.localeId;
        }
        pick ??= all.where((LocaleName x) => norm(x.localeId).startsWith(lang)).firstOrNull?.localeId;
      }
    } catch (_) {}
    return _locales[l] = pick;
  }

  void _complete(String text) {
    _guard?.cancel();
    _guard = null;
    _until = null;
    final Completer<String>? c = _done;
    _done = null;
    if (c != null && !c.isCompleted) c.complete(text.trim());
  }

  void _onResult(SpeechRecognitionResult r) {
    _last = r.recognizedWords;
    if (r.finalResult) {
      _complete(_last);
    } else if (_until?.call(_last) ?? false) {
      _complete(_last);
      _stopping = _s.cancel().catchError((Object _) {});
    }
  }

  void _onStatus(String status) {
    if (status == SpeechToText.listeningStatus) _live = true;
    // "done": the session ended and its final result (if any) was sent.
    if (status == SpeechToText.doneStatus && _live) _complete(_last);
  }

  void _onError(SpeechRecognitionError e) {
    // error_no_match / error_speech_timeout: nothing was said.
    if (_live) _complete(_last);
  }

  @override
  Future<String?> listen({
    required Duration listenFor,
    required Duration pauseFor,
    required bool dictation,
    required SttLanguage language,
    bool Function(String partial)? until,
  }) async {
    if (!await (_ready ??= _init())) return null;
    _complete(_last);
    final Future<void>? stopping = _stopping;
    if (stopping != null) await stopping;
    final Completer<String> c = _done = Completer<String>();
    _last = '';
    _live = false;
    _until = until;
    try {
      await _s.listen(
        onResult: _onResult,
        listenOptions: SpeechListenOptions(
          listenMode: dictation ? ListenMode.dictation : ListenMode.confirmation,
          partialResults: true,
          cancelOnError: true,
          autoPunctuation: dictation,
          listenFor: listenFor,
          pauseFor: pauseFor,
          localeId: await _locale(language),
        ),
      );
    } catch (_) {
      if (identical(_done, c)) _complete('');
    }
    if (identical(_done, c)) _guardFor(c, listenFor + const Duration(seconds: 3));
    return c.future;
  }

  /// Completes [c] with what was heard after [d], unless it ended before.
  void _guardFor(Completer<String> c, Duration d) {
    _guard?.cancel();
    _guard = Timer(d, () {
      if (identical(_done, c)) _complete(_last);
    });
  }

  @override
  Future<void> stop() async {
    try {
      await (_stopping = _s.stop());
    } catch (_) {
    } finally {
      _stopping = null;
    }
    // The final result normally follows; never wait forever for it.
    final Completer<String>? c = _done;
    if (c != null) _guardFor(c, const Duration(seconds: 2));
  }

  @override
  Future<void> cancel() async {
    _complete('');
    try {
      await (_stopping = _s.cancel());
    } catch (_) {
    } finally {
      _stopping = null;
    }
  }
}
