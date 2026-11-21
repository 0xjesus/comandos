import 'dart:async';

/// Milliseconds since epoch. Injected so tests control time.
typedef NowMs = int Function();

int systemNowMs() => DateTime.now().millisecondsSinceEpoch;

/// Creates a one-shot timer. Injected so tests can drive work by hand.
typedef TimerFactory = Timer Function(Duration delay, void Function() callback);

Timer systemTimer(Duration delay, void Function() callback) =>
    Timer(delay, callback);
