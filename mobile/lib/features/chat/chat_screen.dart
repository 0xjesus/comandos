import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import 'pane_rows.dart';
import 'session_view.dart';

/// A session's chat as a route of its own. The app's one screen shows
/// [SessionView] inside the shell; this thin route stays for the callers
/// that still open a chat by route (`AppNavigator.openChat`: notification
/// taps, car mode).
class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key, required this.session, this.pane, this.label});

  static const String routeName = '/chat';

  final String session;

  /// The pane to read and write (e.g. the one a notification is about); null:
  /// the one chosen on this phone, else tmux's active one, as on the web.
  final String? pane;
  final String? label;

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  @override
  void initState() {
    super.initState();
    final String? pane = widget.pane;
    if (pane != null && pane.startsWith('%')) savePane(widget.session, pane);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Orbita.bg,
      // The dock lifts itself above the keyboard.
      resizeToAvoidBottomInset: false,
      body: SafeArea(
        bottom: false,
        child: SessionView(
          session: widget.session,
          title: widget.label ?? widget.session,
          terminal: false,
          onPrev: () => Navigator.of(context).maybePop(),
        ),
      ),
    );
  }
}
