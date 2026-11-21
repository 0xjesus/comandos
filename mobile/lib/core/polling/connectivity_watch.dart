import 'dart:async';

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/foundation.dart';

/// Watches the network. [online] is a hint (Android says "connected" even
/// when Tailscale is down); its only job is to fire [onRegained] so queued
/// work is retried right away instead of waiting out a backoff.
class ConnectivityWatch {
  ConnectivityWatch({required this.onRegained});

  final VoidCallback onRegained;
  final ValueNotifier<bool> online = ValueNotifier<bool>(true);
  StreamSubscription<List<ConnectivityResult>>? _sub;

  void start() {
    _sub ??= Connectivity().onConnectivityChanged.listen((List<ConnectivityResult> r) {
      final bool now = r.any((ConnectivityResult c) => c != ConnectivityResult.none);
      final bool was = online.value;
      online.value = now;
      if (now && !was) onRegained();
    }, onError: (Object _) {});
  }

  Future<void> stop() async {
    await _sub?.cancel();
    _sub = null;
  }
}
