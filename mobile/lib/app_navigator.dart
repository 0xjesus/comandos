import 'package:flutter/material.dart';

import 'features/chat/chat_screen.dart';
import 'features/pairing/pairing_screen.dart';
import 'features/sessions/sessions_screen.dart';
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

  /// Opens (or brings back) the chat of [session]. If that chat is already
  /// the top route, nothing happens.
  static Future<void> openChat(String session, {String? pane, String? label}) async {
    final NavigatorState? nav = _nav;
    if (nav == null) return;
    bool alreadyTop = false;
    nav.popUntil((Route<dynamic> r) {
      if (r.settings.name == ChatScreen.routeName && r.settings.arguments == session) {
        alreadyTop = true;
        return true;
      }
      return r.isFirst || r.settings.name == SessionsScreen.routeName;
    });
    if (alreadyTop) return;
    await nav.push<void>(MaterialPageRoute<void>(
      settings: RouteSettings(name: ChatScreen.routeName, arguments: session),
      builder: (_) => ChatScreen(session: session, pane: pane, label: label),
    ));
  }

  /// Replaces the whole stack with the sessions list.
  static void showSessions() {
    final NavigatorState? nav = _nav;
    if (nav == null) return;
    nav.pushAndRemoveUntil<void>(
      MaterialPageRoute<void>(
        settings: const RouteSettings(name: SessionsScreen.routeName),
        builder: (_) => const SessionsScreen(),
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
