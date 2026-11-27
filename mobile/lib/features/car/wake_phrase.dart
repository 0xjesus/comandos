import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/platform/native_channel.dart';

/// «OK ComandOS» in what the phone heard offline (`CarListener.kt`, Vosk).
///
/// The recognizer writes it as «okay comandos», «oye comandos», «ok y
/// comandos» or «aunque hay comandos»: «comandos» with a call word among the
/// three words before it. The short «OK» is often lost at the start of what
/// it hears, so an utterance that STARTS with «comandos» (after at most two
/// short words: «o que comandos», «y comandos») counts too, unless the next
/// word makes it talk about commands («comandos de la terminal»); when
/// «comandos» is all there is yet, it waits for the recognizer's final
/// result ([last]). Talk about commands («los comandos de la terminal») has
/// an article or a preposition right before it and never counts.
bool isWakePhrase(String heard, {bool last = false}) {
  final List<String> w = heard
      .toLowerCase()
      .replaceAll(RegExp(r'[^a-záéíóúüñ ]'), ' ')
      .split(' ')
      .where((String s) => s.isNotEmpty)
      .toList();
  for (int i = 0; i < w.length; i++) {
    if (w[i] != 'comandos' && w[i] != 'comando') continue;
    if (i > 0 && _notCall.contains(w[i - 1])) continue;
    for (int j = i - 1; j >= 0 && j >= i - 3; j--) {
      if (_call.contains(w[j])) return true;
    }
    // Only the plural: «comando especial…» is someone talking about a commando.
    final bool opens = w[i] == 'comandos' && i <= 2 && w.take(i).every((String x) => x.length <= 4 && !_notCall.contains(x));
    if (!opens) continue;
    if (i + 1 >= w.length) return last;
    if (!_aboutCommands.contains(w[i + 1])) return true;
  }
  return false;
}

/// Words after «comandos» that make it a noun («comandos de la terminal»).
const Set<String> _aboutCommands = <String>{'de', 'del', 'para', 'con', 'sin', 'en', 'al', 'por'};

final RegExp _wordRe = RegExp(r"[\p{L}']+", unicode: true);
final RegExp _comandosRe = RegExp(r'^c[oa]mm?andos?$');
final RegExp _commandRe = RegExp(r'^c[oa]mm?and$');

/// Words of [heard] up to and including «comandos» (the end of the wake
/// phrase), or 0 when it is not there.
int wakePhraseWords(String heard) {
  final List<String> w = _wordRe.allMatches(heard).map((RegExpMatch m) => m[0]!.toLowerCase()).toList();
  for (int i = 0; i < w.length; i++) {
    if (_comandosRe.hasMatch(w[i])) return i + 1;
  }
  return 0;
}

/// [heard] without the wake phrase it starts with, as OpenAI or the offline
/// recognizer wrote it: «OK ComandOS, ¿qué hay de nuevo?» → «Qué hay de
/// nuevo?». A few words said before it (the take keeps ~2 s from before the
/// phrase) go with it. Text that does not start with the phrase is left as
/// it is: «revisa los comandos» stays.
String stripWakePhrase(String heard) {
  final String h = heard.trim();
  final List<RegExpMatch> ms = _wordRe.allMatches(h).toList();
  final List<String> w = ms.map((RegExpMatch m) => m[0]!.toLowerCase().replaceAll("'", '')).toList();
  for (int i = 0; i < w.length && i < 10; i++) {
    final int end;
    if (_comandosRe.hasMatch(w[i])) {
      end = i;
    } else if (_commandRe.hasMatch(w[i]) && i + 1 < w.length && w[i + 1] == 'os') {
      end = i + 1; // «Command OS»
    } else {
      continue;
    }
    if (i > 0 && _notCall.contains(w[i - 1])) continue;
    bool called = false;
    for (int j = i - 1; j >= 0 && j >= i - 3; j--) {
      if (_call.contains(w[j])) called = true;
    }
    final String rest = h.substring(ms[end].end);
    // «ComandOS, revisa…» alone at the start is the phrase too.
    if (!called && !(i == 0 && RegExp(r'^\s*[,.:!]').hasMatch(rest))) continue;
    final String t = rest.replaceFirst(RegExp(r'^[^\p{L}\p{N}]+', unicode: true), '').trim();
    return t.isEmpty ? '' : '${t[0].toUpperCase()}${t.substring(1)}';
  }
  return h;
}

const Set<String> _call = <String>{'ok', 'okay', 'okey', 'oye', 'oiga', 'hey', 'ey', 'aunque', 'hola'};

const Set<String> _notCall = <String>{
  'los', 'el', 'un', 'unos', 'mis', 'mi', 'tu', 'su', 'tus', 'sus', 'nuestros', 'estos', 'esos', 'aquellos', 'otros', 'varios', 'muchos',
  'pocos', 'dos', 'tres', 'de', 'con', 'sin', 'por', 'para', 'tantos', 'cuantos', 'nuevos', 'algunos',
};

enum WakeResult { heard, stopped, unavailable }

/// Waits for the wake phrase.
abstract class WakeListener {
  /// Until «OK ComandOS» ([WakeResult.heard]), [stop] ([WakeResult.stopped])
  /// or a reason it cannot listen ([WakeResult.unavailable], see [problem]).
  Future<WakeResult> wait();

  /// Stops listening; the microphone is free when the future completes.
  Future<void> stop();

  String? get problem;

  /// What matched the phrase last time [wait] gave [WakeResult.heard].
  String get heard;
}

/// [WakeListener] on the phone (`CarListener.kt` through `comandos/native`).
///
/// With [keepMic] the listener keeps the microphone when the phrase matches:
/// the call turns it into the person's take right away
/// (`CarPorts.takeAfterWake`), so what is said in the same breath is kept.
/// Without it the listener stops and the microphone is free.
class NativeWake implements WakeListener {
  NativeWake({NativeChannel? native, this.keepMic = false}) : _native = native ?? NativeChannel.instance;

  final NativeChannel _native;
  final bool keepMic;
  String _heard = '';

  @override
  String get heard => _heard;
  Completer<WakeResult>? _wait;
  String? _problem;

  @override
  String? get problem => _problem;

  void _finish(WakeResult r) {
    final Completer<WakeResult>? c = _wait;
    _wait = null;
    if (c != null && !c.isCompleted) c.complete(r);
  }

  @override
  Future<WakeResult> wait() async {
    _finish(WakeResult.stopped);
    final Completer<WakeResult> c = _wait = Completer<WakeResult>();
    _native.onWake = (String text, bool last) {
      if (!identical(_wait, c) || !isWakePhrase(text, last: last)) return;
      _heard = text;
      if (keepMic) {
        _finish(WakeResult.heard);
        return;
      }
      unawaited(_native.stopWake().whenComplete(() => _finish(WakeResult.heard)));
    };
    _native.onWakeError = (String why) {
      if (!identical(_wait, c)) return;
      _problem = why;
      _finish(WakeResult.unavailable);
    };
    final bool started = await _native.startWake();
    if (!started && identical(_wait, c)) {
      _problem ??= 'Este teléfono no puede escuchar «OK ComandOS».';
      _finish(WakeResult.unavailable);
    }
    return c.future;
  }

  @override
  Future<void> stop() async {
    _finish(WakeResult.stopped);
    await _native.stopWake();
  }

  @visibleForTesting
  bool get waiting => _wait != null;
}
