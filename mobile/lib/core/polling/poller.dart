import 'dart:async';

import '../util/clock.dart';

/// Runs [task] repeatedly, one call in flight at a time, with a one-shot
/// timer between runs (never a periodic timer). [task] returns the delay
/// before the next run, or null for [interval].
///
/// [stop] cancels the timer; a run already in flight finishes but does not
/// schedule another. Nothing keeps running once stopped.
class Poller {
  Poller({
    required this.interval,
    required Future<Duration?> Function() task,
    TimerFactory timer = systemTimer,
  })  : _task = task,
        _timer = timer;

  final Duration interval;
  final Future<Duration?> Function() _task;
  final TimerFactory _timer;

  Timer? _next;
  bool _running = false;
  bool _inFlight = false;
  bool _runAgain = false;
  int _generation = 0;

  bool get isRunning => _running;

  /// Starts polling, running the first time immediately.
  void start() {
    if (_running) return;
    _running = true;
    _generation++;
    _run();
  }

  void stop() {
    _running = false;
    _generation++;
    _next?.cancel();
    _next = null;
  }

  /// Run now (pull to refresh, connectivity back). Coalesces with a run in
  /// flight.
  void poke() {
    if (!_running) return;
    if (_inFlight) {
      _runAgain = true;
      return;
    }
    _next?.cancel();
    _run();
  }

  Future<void> _run() async {
    final int gen = _generation;
    _inFlight = true;
    Duration? delay;
    try {
      delay = await _task();
    } catch (_) {
      delay = null;
    } finally {
      _inFlight = false;
    }
    if (!_running || gen != _generation) return;
    if (_runAgain) {
      _runAgain = false;
      _run();
      return;
    }
    _next = _timer(delay ?? interval, () {
      _next = null;
      if (_running) _run();
    });
  }
}

/// Error backoff shared by the pollers: 2.5 s → 5 → 10 → 20 → 30 s cap.
Duration errorBackoff(int consecutiveErrors, {Duration base = const Duration(milliseconds: 2500)}) {
  final int factor = 1 << (consecutiveErrors.clamp(0, 4));
  final Duration d = base * factor;
  return d > const Duration(seconds: 30) ? const Duration(seconds: 30) : d;
}
