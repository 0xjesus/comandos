import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/platform/native_channel.dart';

/// «OK ComandOS» in what the phone heard offline (`WakeWord.kt`, Vosk).
///
/// The recognizer writes it as «okay comandos», «oye comandos», «ok y
/// comandos» or «aunque hay comandos»: «comandos» with a call word among the
/// three words before it. Talk about commands («los comandos de la terminal»)
/// has an article or a preposition right before it and never counts.
bool isWakePhrase(String heard) {
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
  }
  return false;
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
}

/// [WakeListener] on the phone (`WakeWord.kt` through `comandos/native`).
class NativeWake implements WakeListener {
  NativeWake([NativeChannel? native]) : _native = native ?? NativeChannel.instance;

  final NativeChannel _native;
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
      if (!identical(_wait, c) || !isWakePhrase(text)) return;
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
