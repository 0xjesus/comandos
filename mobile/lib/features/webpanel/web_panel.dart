import 'dart:async';

import 'package:flutter/material.dart' show Material, MaterialType;
import 'package:flutter/widgets.dart';

import '../../core/config/paired_server.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'web_scripts.dart';
import 'web_session.dart';
import 'web_states.dart';
import 'web_surface.dart';
import 'web_urls.dart';

export 'web_session.dart' show clearWebData;
export 'web_urls.dart' show WebPanel, webPanelTitle;

/// Opens [panel] full screen: the dashboard's own page (Ajustes, Servidores,
/// Snippets, Resúmenes, Remoto or the whole dashboard) under one thin bar with
/// «‹» and the title. Android back closes it, and so does the page's own ✕.
/// The WebView exists only while the route is open.
Future<void> openWebPanel(BuildContext context, WebPanel panel) =>
    Navigator.of(context).push<void>(PageRouteBuilder<void>(
      settings: RouteSettings(name: '/web/${panel.name}'),
      transitionDuration: Duration.zero,
      reverseTransitionDuration: Duration.zero,
      pageBuilder: (BuildContext _, Animation<double> _, Animation<double> _) => WebPanelScreen(panel: panel),
    ));

/// The route body of [openWebPanel].
class WebPanelScreen extends StatefulWidget {
  const WebPanelScreen({super.key, required this.panel});

  final WebPanel panel;

  @override
  State<WebPanelScreen> createState() => _WebPanelScreenState();
}

enum _Phase { loading, shown, error }

class _WebPanelScreenState extends State<WebPanelScreen> {
  WebSurface? _surface;
  bool _acquired = false;
  _Phase _phase = _Phase.loading;
  Timer? _deadline;
  Uri? _url;
  bool _closing = false;
  bool _disposed = false;

  String get _title => webPanelTitle(widget.panel);

  @override
  void initState() {
    super.initState();
    // Right after this build pass: no WebView work inside initState.
    scheduleMicrotask(_open);
  }

  Future<void> _open() async {
    final PairedServer? server = WebPorts.server();
    if (server == null) {
      _fail();
      return;
    }
    if (_acquired) unawaited(WebSession.release());
    _acquired = true;
    final bool cookie = await WebSession.acquire(server);
    if (_disposed) return;
    _url = webPanelUrl(server, widget.panel, tokenInUrl: !cookie);
    _surface = WebPorts.surfaces(WebSurfaceEvents(
      allow: (Uri url, {required bool mainFrame}) => allowedNavigation(url, server, mainFrame: mainFrame),
      onPageFinished: _pageFinished,
      onMainFrameError: _fail,
      onMessage: _message,
    ));
    setState(() {});
    _load();
  }

  void _load() {
    final WebSurface? s = _surface;
    final Uri? url = _url;
    if (s == null || url == null) return;
    _deadline?.cancel();
    _deadline = Timer(kPageReadyTimeout + const Duration(seconds: 5), _fail);
    unawaited(s.load(url));
  }

  void _pageFinished() {
    if (_disposed || _phase == _Phase.error) return;
    unawaited(_surface?.runJs(panelScript(widget.panel)));
  }

  void _message(String raw) {
    final PageMessage? m = PageMessage.parse(raw);
    if (m == null || _disposed) return;
    switch (m.t) {
      case 'open':
        _deadline?.cancel();
        if (_phase != _Phase.shown) setState(() => _phase = _Phase.shown);
      case 'closed':
        _close();
      case 'fail':
        _fail();
    }
  }

  void _fail() {
    _deadline?.cancel();
    if (_disposed || _phase == _Phase.error) return;
    setState(() => _phase = _Phase.error);
  }

  void _retry() {
    if (_surface == null) {
      setState(() => _phase = _Phase.loading);
      unawaited(_open());
      return;
    }
    setState(() => _phase = _Phase.loading);
    _load();
  }

  void _close() {
    if (_closing || _disposed) return;
    _closing = true;
    Navigator.of(context).maybePop();
  }

  @override
  void dispose() {
    _disposed = true;
    _deadline?.cancel();
    final WebSurface? s = _surface;
    _surface = null;
    if (s != null) unawaited(s.dispose());
    if (_acquired) unawaited(WebSession.release());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final WebSurface? s = _surface;
    return Material(
      type: MaterialType.canvas,
      color: Orbita.bg,
      child: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            // Once shown, the page brings its own ✕: a second bar would repeat it.
            if (_phase != _Phase.shown) WebPanelBar(title: _title, onBack: _close),
            Expanded(
              child: Stack(
                fit: StackFit.expand,
                children: <Widget>[
                  if (s != null) KeyedSubtree(key: ObjectKey(s), child: s.view()),
                  if (_phase == _Phase.loading) const WebLoading(),
                  if (_phase == _Phase.error)
                    WebError(
                      message: 'No se pudo abrir $_title.',
                      detail: 'Revisa que tu equipo esté encendido y en la red.',
                      onRetry: _retry,
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The thin bar over a web panel: «‹» (raised key, like `header .hdr-btn`) and
/// the title (`.modal-title`: 13 px, uppercase, .16em, `--dim`), 1 px `--line`
/// under it.
class WebPanelBar extends StatelessWidget {
  const WebPanelBar({super.key, required this.title, required this.onBack});

  final String title;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.fromLTRB(10, 6, 16, 8),
        decoration: const BoxDecoration(
          color: Orbita.bg,
          border: Border(bottom: BorderSide(color: Orbita.line)),
        ),
        child: Row(
          children: <Widget>[
            WebButton(
              size: 44,
              tooltip: 'Volver',
              onTap: onBack,
              child: const WebIcon(WebIcons.chevronLeft, size: 18),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Text(
                title.toUpperCase(),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  fontFamily: Orbita.sans,
                  fontSize: 13,
                  letterSpacing: 13 * .16,
                  fontWeight: FontWeight.w600,
                  color: Orbita.dim,
                ),
              ),
            ),
          ],
        ),
      );
}
