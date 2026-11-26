import 'package:flutter/material.dart';

import '../../core/locator.dart';
import 'orchestrator_screen.dart';

/// Builds the "Modo auto" screen. [session]/[pane] are set when opened from a
/// chat, null when opened from the sessions list.
typedef CarModeBuilder = Widget Function(BuildContext context, String? session, String? pane);

/// Entry point used by the sessions screen and the chat. The car work
/// registers its own builder in [registerCarMode]; the screens never change.
class CarMode {
  const CarMode(this.builder);

  final CarModeBuilder builder;

  static const String routeName = '/car';

  static Future<void> open(BuildContext context, {String? session, String? pane}) {
    final CarMode mode = sl.get<CarMode>();
    return Navigator.of(context).push<void>(MaterialPageRoute<void>(
      settings: const RouteSettings(name: routeName),
      builder: (BuildContext c) => mode.builder(c, session, pane),
    ));
  }
}

/// Registers Modo auto: a call with the orchestrator session
/// (`orchestrator_screen.dart`), wherever it is opened from.
void registerCarMode() {
  sl.register<CarMode>(CarMode((BuildContext context, String? session, String? pane) => const OrchestratorScreen()));
}
