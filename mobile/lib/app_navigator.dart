import 'package:flutter/material.dart';

import 'features/pairing/pairing_screen.dart';
import 'features/shell/app_shell.dart';
import 'features/shell/shell_controller.dart';
import 'features/settings/settings_screen.dart';

/// Global navigation entry points. Usable without a BuildContext (e.g. from a
/// notification tap): `AppNavigator.openChat('term-r1', pane: '%3')`.
abstract final class AppNavigator {
  static final GlobalKey<NavigatorState> key = GlobalKey<NavigatorState>();

  /// Lets screens pause polling while another route covers them.
  static final RouteObserver<ModalRoute<void>> routes = RouteObserver<ModalRoute<void>>();

  static NavigatorState? get _nav => key.currentState;

  /// Session whose chat is on screen (route on top and app visible), else
  /// null. Set by the chat screen; notifications skip that session.
  static final ValueNotifier<String?> visibleChat = ValueNotifier<String?>(null);

  /// True once the sessions list has been shown (navigation is ready).
  static final ValueNotifier<bool> homeShown = ValueNotifier<bool>(false);

  /// Shows [session] in the shell (from a notification tap or car mode).
  /// Works before the shell is mounted: it opens once the shell appears.
  static Future<void> openChat(String session, {String? pane, String? label}) async {
    _nav?.popUntil((Route<dynamic> r) => r.isFirst || r.settings.name == AppShell.routeName);
    ShellController.openSession(session, pane: pane);
  }

  /// Replaces the whole stack with the sessions list.
  static void showSessions() {
    final NavigatorState? nav = _nav;
    if (nav == null) return;
    nav.pushAndRemoveUntil<void>(
      MaterialPageRoute<void>(
        settings: const RouteSettings(name: AppShell.routeName),
        builder: (_) => const AppShell(),
      ),
      (Route<dynamic> _) => false,
    );
    homeShown.value = true;
  }

  /// Replaces the whole stack with pairing (first launch / unpaired).
  static void showPairing() {
    homeShown.value = false;
    _nav?.pushAndRemoveUntil<void>(
      MaterialPageRoute<void>(
        settings: const RouteSettings(name: PairingScreen.routeName),
        builder: (_) => PairingScreen(onPaired: showSessions),
      ),
      (Route<dynamic> _) => false,
    );
  }

  static Future<void> openSettings() async {
    await _nav?.push<void>(MaterialPageRoute<void>(
      settings: const RouteSettings(name: SettingsScreen.routeName),
      builder: (_) => const SettingsScreen(),
    ));
  }
}
