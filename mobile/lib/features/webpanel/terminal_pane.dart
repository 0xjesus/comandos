import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../../core/api/api_response.dart';
import '../../core/config/paired_server.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'term_interaction.dart';
import 'web_scripts.dart';
import 'web_session.dart';
import 'web_surface.dart';
import 'web_urls.dart';

/// The live terminal of [session], the dashboard's own terminal page
/// (`/term/?auth=…&arg=<session>&theme=…`), full-bleed in the space it gets.
///
/// - The page owns every touch: scroll, long-press selection, the selection
///   toolbar, «Historial», the pane dialog — exactly as in the mobile web.
/// - It opens docked (`{type:"dock"}`): its own key bar and draft are hidden,
///   the app's dock draws them (SessionView). The dock reaches the page through
///   [TerminalBridge.active].
/// - On a phone the page never resizes the shared desktop window (it opens
///   with `ignoreSize`, `crop::ignore_size`) and shows only [pane] of a split.
/// - One WebView at most. Changing [session] navigates the same WebView,
///   changing [pane] posts `select-pane`. Leaving keeps it warm for
///   [WebPorts.terminalWarm] (30 s: flipping chat ↔ terminal does not reload
///   the page and its socket), then it is destroyed; same when the app stays
///   in the background that long. Nothing is created before the first
///   TerminalPane is shown.
class TerminalPane extends StatefulWidget {
  const TerminalPane({super.key, required this.session, this.pane, this.label});

  /// Tab name the page's pane header shows (as the dashboard gives it).
  final String? label;

  final String session;

  /// tmux pane id like `%12`; null = the session's active pane.
  final String? pane;

  @override
  State<TerminalPane> createState() => _TerminalPaneState();
}

class _TerminalPaneState extends State<TerminalPane> {
  TerminalEngine? _engine;
  ValueListenable<bool>? _visible;
  Timer? _hiddenTimer;

  @override
  void initState() {
    super.initState();
    _visible = WebPorts.visibility();
    _visible?.addListener(_visibilityChanged);
    if (_visible?.value ?? true) _attach();
  }

  void _attach() {
    final PairedServer? server = WebPorts.server();
    if (server == null) return;
    final TerminalEngine engine = TerminalEngine.obtain(server);
    _engine = engine;
    engine.attach(this, _changed);
    engine.show(widget.session, widget.pane, label: widget.label);
  }

  void _visibilityChanged() {
    final bool visible = _visible?.value ?? true;
    if (visible) {
      _hiddenTimer?.cancel();
      _hiddenTimer = null;
      final TerminalEngine? e = _engine;
      if (e == null || e.destroyed) {
        setState(_attach);
      } else {
        e.resume();
      }
      return;
    }
    _engine?.pause();
    _hiddenTimer ??= Timer(WebPorts.terminalWarm, () {
      _hiddenTimer = null;
      final TerminalEngine? e = _engine;
      if (e == null || !mounted) return;
      _engine = null;
      e.detach(this, destroy: true);
      setState(() {});
    });
  }

  @override
  void didUpdateWidget(TerminalPane old) {
    super.didUpdateWidget(old);
    if (old.session != widget.session || old.pane != widget.pane || old.label != widget.label) {
      _engine?.show(widget.session, widget.pane, label: widget.label);
    }
  }

  @override
  void dispose() {
    _hiddenTimer?.cancel();
    _visible?.removeListener(_visibilityChanged);
    _engine?.detach(this);
    _engine = null;
    super.dispose();
  }

  /// Engine news. Deferred: it can arrive while the tree is building or
  /// unmounting (attach/detach run in initState/dispose).
  void _changed() {
    scheduleMicrotask(() {
      if (!mounted) return;
      setState(() {
        // Destroyed under us (another desktop paired, data cleared): start
        // again if there is still a desktop and the app is on screen.
        final TerminalEngine? e = _engine;
        if (e != null && e.destroyed && (_visible?.value ?? true)) {
          _engine = null;
          _attach();
        }
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final TerminalEngine? e = _engine;
    final TermStatus status = e?.status ?? TermStatus.loading;
    return ColoredBox(
      color: Orbita.bg,
      child: Stack(
        fit: StackFit.expand,
        children: <Widget>[
          if (e != null && !e.destroyed) KeyedSubtree(key: ObjectKey(e), child: e.surface.view()),
          // The page paints its own background; until it is docked and
          // connected only the terminal background shows (no flash of its
          // own key bar).
          if (status == TermStatus.loading) const ColoredBox(color: Orbita.bg),
          if (status == TermStatus.error)
            _TermError(onRetry: () => e?.retry()),
        ],
      ),
    );
  }
}

class _TermError extends StatelessWidget {
  const _TermError({required this.onRetry});

  final VoidCallback onRetry;

  // The page's own `#err` look: brand colour, mono, top-left, 16 px padding.
  @override
  Widget build(BuildContext context) => ColoredBox(
        color: Orbita.bg,
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              const Text(
                'No pude conectar con la terminal.',
                style: TextStyle(fontFamily: Orbita.mono, fontSize: 14, height: 1.4, color: Orbita.brand),
              ),
              const SizedBox(height: 12),
              WebButton(
                onTap: onRetry,
                padding: const EdgeInsets.symmetric(horizontal: 18),
                child: const Text('Reintentar'),
              ),
            ],
          ),
        ),
      );
}

enum TermStatus { loading, ready, error }

/// What the dock can ask the live terminal page (chat_view.rs posts the same
/// `toolbar` messages to its terminal frame).
class TerminalCommands {
  TerminalCommands._(this._engine);

  final TerminalEngine _engine;

  /// Session shown.
  String? get session => _engine._session;

  /// Pane the page was asked to show (null = active).
  String? get pane => _engine._pane;

  /// A toolbar action: mode (Seleccionar/Interactuar), panes (Paneles),
  /// history (Historial), copy (Copiar), paste (Pegar), keyboard (Teclado
  /// directo), ctrl (Ctrl). False if unknown or the page is not ready.
  bool action(String action) =>
      kTermActions.contains(action) && _engine._post(toolbarMessage(<String, Object?>{'type': 'action', 'action': action}));

  /// A key through the page's socket (escape, ctrlc, backspace, tab, arrows,
  /// yes, no). The web dock sends keys with `POST /key` instead; both work.
  bool key(String key) =>
      kTermKeys.contains(key) && _engine._post(toolbarMessage(<String, Object?>{'type': 'toolbar', 'key': key}));

  /// Types [text] into the terminal as a bracketed paste (no Enter).
  bool paste(String text) =>
      text.isNotEmpty && _engine._post(toolbarMessage(<String, Object?>{'type': 'paste', 'text': text}));

  /// Focuses the terminal (direct keyboard).
  bool focus() => _engine._post(toolbarMessage(const <String, Object?>{'type': 'focus'}));
}

/// The terminal currently on screen and ready, for the dock. Null otherwise.
abstract final class TerminalBridge {
  static final ValueNotifier<TerminalCommands?> active = ValueNotifier<TerminalCommands?>(null);
}

/// The one terminal WebView of the app and what the dashboard does for its
/// terminal frames (dock, theme, pane, «Seleccionar»).
class TerminalEngine {
  TerminalEngine._(this._server) {
    surface = WebPorts.surfaces(WebSurfaceEvents(
      allow: (Uri url, {required bool mainFrame}) => allowedNavigation(url, _server, mainFrame: mainFrame),
      onPageFinished: _pageFinished,
      onMainFrameError: _fail,
      onMessage: _message,
    ));
    _cookie = WebSession.acquire(_server);
    webDataDroppers.add(_drop);
  }

  static TerminalEngine? _live;

  /// The live engine for [server] (the warm one if any), or a new one.
  static TerminalEngine obtain(PairedServer server) {
    final TerminalEngine? live = _live;
    if (live != null && !live.destroyed && live._server.host == server.host &&
        live._server.port == server.port && live._server.token == server.token) {
      live._reaper?.cancel();
      live._reaper = null;
      return live;
    }
    live?._destroy();
    return _live = TerminalEngine._(server);
  }

  @visibleForTesting
  static TerminalEngine? get live => _live;

  /// Last look read from `/prefs` (kept for the next terminal of this run).
  static TermLook _look = TermLook.fallback;

  final PairedServer _server;
  late final WebSurface surface;
  late final Future<bool> _cookie;
  TermStatus status = TermStatus.loading;
  bool destroyed = false;

  String? _session;
  String? _pane;
  TermLook _urlLook = TermLook.fallback;
  bool _ready = false;
  bool _paused = false;
  TermInteraction? _interaction;
  Object? _owner;
  VoidCallback? _onChanged;
  Timer? _reaper;
  Timer? _deadline;
  TerminalCommands? _commands;

  /// [owner] shows this engine; [onChanged] runs when [status] changes.
  void attach(Object owner, VoidCallback onChanged) {
    _owner = owner;
    _onChanged = onChanged;
    _reaper?.cancel();
    _reaper = null;
    _publish();
  }

  /// The pane left the screen: keep warm for a while, then destroy.
  void detach(Object owner, {bool destroy = false}) {
    if (!identical(_owner, owner)) return;
    _owner = null;
    _onChanged = null;
    if (destroyed) return;
    _publish();
    unawaited(_interaction?.restore());
    if (destroy) {
      _destroy();
      return;
    }
    _reaper?.cancel();
    _reaper = Timer(WebPorts.terminalKeep, _destroy);
  }

  /// App hidden: give tmux its mouse back (the web does it on page hide).
  void pause() {
    _paused = true;
    unawaited(_interaction?.restore());
  }

  /// App visible again: re-read tmux's mouse mode.
  void resume() {
    if (!_paused) return;
    _paused = false;
    if (_ready) unawaited(_interaction?.sync());
  }

  String? _label;

  void show(String session, String? pane, {String? label}) {
    if (destroyed) return;
    if (label != _label) {
      _label = label;
      if (_ready && session == _session) unawaited(surface.runJs(tabLabelScript(session, label)));
    }
    final String? wanted = validPaneId(pane) ? pane : null;
    if (session != _session) {
      final TermInteraction? old = _interaction;
      old?.close();
      unawaited(old?.restore());
      final bool inPlace = _ready && _session != null;
      _session = session;
      _pane = wanted;
      _interaction = TermInteraction(session: session, request: _request, post: _post);
      if (inPlace) {
        // The page is up: switch its socket to the other session, no reload.
        _post(sessionMessage(session));
        if (wanted != null) _post(selectPaneMessage(wanted));
        unawaited(surface.runJs(tabLabelScript(session, label)));
        if (!_paused) unawaited(_interaction?.sync());
        _publish();
        return;
      }
      unawaited(_load());
      return;
    }
    if (wanted != _pane) {
      _pane = wanted;
      if (_ready && wanted != null) _post(selectPaneMessage(wanted));
      _publish();
    }
  }

  void retry() {
    if (destroyed || _session == null) return;
    unawaited(_load());
  }

  Future<ApiResponse> _request(String method, String path,
          {Map<String, String>? query, Map<String, Object?>? body}) =>
      WebPorts.request(method, path, query: query, body: body);

  Future<void> _load() async {
    final String? session = _session;
    if (session == null) return;
    _ready = false;
    _setStatus(TermStatus.loading);
    _deadline?.cancel();
    _deadline = Timer(kPageReadyTimeout + const Duration(seconds: 5), _fail);
    await _cookie;
    if (destroyed || session != _session) return;
    _urlLook = _look;
    await surface.load(terminalUrl(_server, session, look: _urlLook));
    unawaited(_refreshLook());
  }

  Future<void> _refreshLook() async {
    try {
      final ApiResponse r = await WebPorts.request('GET', '/prefs');
      if (!r.ok || destroyed) return;
      _look = TermLook.fromPrefs(r.map);
      if (_ready) _applyLook();
    } on Object {
      // Keep the current look.
    }
  }

  void _applyLook() {
    final TermLook look = _look;
    if (look.palette != null || look.theme != _urlLook.theme) _post(look.themeMessage);
    if (look.buttons != _urlLook.buttons) {
      _post(<String, Object?>{'source': 'comandos', 'type': 'button-style', 'style': look.buttons});
    }
  }

  void _pageFinished() {
    if (destroyed) return;
    final String? session = _session;
    if (session != null) unawaited(surface.runJs(tabLabelScript(session, _label)));
    unawaited(surface.runJs(terminalScript()));
  }

  void _message(String raw) {
    final PageMessage? m = PageMessage.parse(raw);
    if (m == null || destroyed) return;
    switch (m.t) {
      case 'ready':
        _deadline?.cancel();
        _ready = true;
        _post(kDockMessage);
        _applyLook();
        final String? pane = _pane;
        if (pane != null) _post(selectPaneMessage(pane));
        _setStatus(TermStatus.ready);
        if (!_paused) unawaited(_interaction?.sync());
      case 'fail':
        _fail();
      case 'term':
        if (m.type == 'interaction-request') unawaited(_interaction?.request(m.selecting));
    }
  }

  void _fail() {
    _deadline?.cancel();
    if (destroyed) return;
    _ready = false;
    _setStatus(TermStatus.error);
  }

  void _setStatus(TermStatus s) {
    if (status == s) return;
    status = s;
    _publish();
    _onChanged?.call();
  }

  bool _post(Map<String, Object?> message) {
    if (!_ready || destroyed) return false;
    unawaited(surface.runJs(postToPage(message)));
    return true;
  }

  /// Updates [TerminalBridge.active] in a microtask: listeners (the dock) may
  /// rebuild, and attach/detach run while the widget tree is busy.
  void _publish() {
    scheduleMicrotask(() {
      final bool live = _owner != null && _ready && !destroyed;
      if (live) {
        _commands ??= TerminalCommands._(this);
        if (TerminalBridge.active.value != _commands) TerminalBridge.active.value = _commands;
      } else if (_commands != null && identical(TerminalBridge.active.value, _commands)) {
        TerminalBridge.active.value = null;
      }
    });
  }

  Future<void> _drop() async => _destroy();

  void _destroy() {
    if (destroyed) return;
    destroyed = true;
    _ready = false;
    _reaper?.cancel();
    _deadline?.cancel();
    final TermInteraction? i = _interaction;
    i?.close();
    unawaited(i?.restore());
    _publish();
    webDataDroppers.remove(_drop);
    if (identical(_live, this)) _live = null;
    unawaited(surface.dispose());
    unawaited(_cookie.then((_) => WebSession.release()));
    _onChanged?.call();
  }

  @visibleForTesting
  static void resetForTest() {
    _live?._destroy();
    _live = null;
    _look = TermLook.fallback;
  }
}
