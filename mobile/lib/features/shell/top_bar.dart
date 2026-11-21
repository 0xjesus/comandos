import 'dart:async';

import 'package:flutter/material.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// `#topbar header.hdr-ordered` inside the side panel (body.app): ☰ Más, Buscar,
/// Snippets, Remoto, Resúmenes, the clock and Ajustes (40 px), as 34 px «sutil»
/// keys. Each key only reports the tap; the shell decides what it opens.
class SideTopBar extends StatelessWidget {
  const SideTopBar({
    super.key,
    required this.menuKey,
    required this.onMenu,
    required this.onSwitch,
    required this.onSnippets,
    required this.onRemote,
    required this.onNews,
    required this.onSettings,
    this.remoteOn = false,
  });

  /// Anchors the «Más» popover under the ☰ key.
  final GlobalKey menuKey;
  final VoidCallback onMenu;
  final VoidCallback onSwitch;
  final VoidCallback onSnippets;
  final VoidCallback onRemote;
  final VoidCallback onNews;
  final VoidCallback onSettings;

  /// `#btn-remote.on`: remote access is on (`/remote-state` → `remoteOn`).
  final bool remoteOn;

  static Widget _key(WebIconData icon, double iconSize, String tip, VoidCallback onTap,
          {Key? key, double size = 34, bool on = false}) =>
      WebButton(
        key: key,
        size: size,
        tooltip: tip,
        on: on,
        onTap: onTap,
        child: WebIcon(icon, size: iconSize),
      );

  @override
  Widget build(BuildContext context) {
    const Widget gap = SizedBox(width: 10);
    return Container(
      decoration: const BoxDecoration(
        color: Orbita.bg,
        border: Border(bottom: BorderSide(color: Orbita.line)),
      ),
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 12),
      child: Row(
        children: <Widget>[
          _key(WebIcons.menu, 17, 'Más', onMenu, key: menuKey),
          gap,
          _key(WebIcons.search, 16, 'Saltar a cualquier sesión', onSwitch, key: const ValueKey<String>('btn-switch')),
          gap,
          _key(WebIcons.snippet, 16, 'Snippets', onSnippets, key: const ValueKey<String>('btn-snippets')),
          gap,
          _key(WebIcons.smartphone, 17, 'Remoto', onRemote,
              key: const ValueKey<String>('btn-remote'), on: remoteOn),
          gap,
          _key(WebIcons.sparkles, 16, 'Resúmenes', onNews, key: const ValueKey<String>('btn-news')),
          gap,
          const WebClock(),
          gap,
          _key(WebIcons.settings, 17, 'Ajustes', onSettings, key: const ValueKey<String>('btn-settings'), size: 40),
        ],
      ),
    );
  }
}

/// `#clock`: HH:MM (24 h), mono 600 12 px, `--dim`. Rebuilds once a minute.
class WebClock extends StatefulWidget {
  const WebClock({super.key});

  @override
  State<WebClock> createState() => _WebClockState();
}

class _WebClockState extends State<WebClock> {
  Timer? _timer;
  String _text = _now();

  static String _now() {
    final DateTime t = DateTime.now();
    String two(int n) => n.toString().padLeft(2, '0');
    return '${two(t.hour)}:${two(t.minute)}';
  }

  @override
  void initState() {
    super.initState();
    _schedule();
  }

  void _schedule() {
    final DateTime t = DateTime.now();
    final Duration wait = Duration(seconds: 60 - t.second, milliseconds: -t.millisecond + 20);
    _timer = Timer(wait, () {
      if (!mounted) return;
      final String next = _now();
      if (next != _text) setState(() => _text = next);
      _schedule();
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 2),
      child: Text(
        _text,
        style: const TextStyle(
          fontFamily: Orbita.mono,
          fontSize: 12,
          height: 1,
          fontWeight: FontWeight.w600,
          letterSpacing: .3,
          color: Orbita.dim,
          fontFeatures: <FontFeature>[FontFeature.tabularFigures()],
        ),
      ),
    );
  }
}
