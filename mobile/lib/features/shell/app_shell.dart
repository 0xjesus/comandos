import 'dart:async';

import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/api/api_errors.dart';
import '../../core/api/api_response.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/session_feed.dart';
import '../../core/outbox/outbox.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../../ui/widgets/banner_strip.dart';
import '../../ui/widgets/outbox_indicator.dart';
import '../car/car_mode.dart';
import '../chat/session_view.dart';
import '../new_session/new_session.dart';
import '../sessions/sections.dart';
import '../sessions/sessions_controller.dart';
import '../usage/usage_summary.dart';
import '../webpanel/web_panel.dart';
import 'nav_bar.dart';
import 'peek_card.dart';
import 'session_switcher.dart';
import 'shell_controller.dart';
import 'shell_menu.dart';
import 'side_panel.dart';
import 'tabs_controller.dart';
import 'top_bar.dart';

/// The whole app once paired: a native copy of the ComandOS mobile web page.
///
/// ```
/// #app-navigation   [panel] [+]            [chat/terminal] [layers]
/// #tabbar           ( ⌂ local ) ( pagos ) ( tienda ) …
/// status strip      (token rejected / feed error / outbox; only when needed)
/// body              side panel (topbar, Sesiones | Consumos | servers, feed)
///                   or SessionView(session) in chat or terminal mode
/// ```
class AppShell extends StatefulWidget {
  const AppShell({super.key, this.store});

  /// Where the last session and mode survive a cold start (default:
  /// `<app support>/shell.json`).
  final ShellStore? store;

  static const String routeName = '/shell';

  @override
  State<AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<AppShell> with RouteAware {
  late final ShellController _shell = ShellController(store: widget.store ?? FileShellStore());
  late final SessionsController _feed = SessionsController(sl.get<Connection>());
  late final TabsController _tabs = TabsController(sl.get<Connection>());
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  final GlobalKey _menuKey = GlobalKey();
  final GlobalKey _navKey = GlobalKey();
  bool _covered = false;
  bool _remoteOn = false;
  String? _seenFor;

  bool get _visible => _appVisible.value && !_covered;

  @override
  void initState() {
    super.initState();
    ShellController.current = _shell;
    _appVisible.addListener(_syncVisible);
    sl.maybe<ConnectivityWatch>()?.online.addListener(_onOnline);
    _shell.addListener(_onShell);
    _feed.addListener(_onFeed);
    _syncVisible();
    unawaited(_shell.restore());
    unawaited(_loadRemote());
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ModalRoute<void>? route = ModalRoute.of(context);
    if (route != null) AppNavigator.routes.subscribe(this, route);
  }

  @override
  void dispose() {
    AppNavigator.routes.unsubscribe(this);
    _appVisible.removeListener(_syncVisible);
    sl.maybe<ConnectivityWatch>()?.online.removeListener(_onOnline);
    _shell.removeListener(_onShell);
    _feed.removeListener(_onFeed);
    if (AppNavigator.visibleChat.value == _shell.active) AppNavigator.visibleChat.value = null;
    _feed.dispose();
    _tabs.dispose();
    _shell.dispose();
    super.dispose();
  }

  @override
  void didPushNext() {
    _covered = true;
    _syncVisible();
  }

  @override
  void didPopNext() {
    _covered = false;
    _syncVisible();
  }

  void _syncVisible() {
    _feed.setVisible(_visible);
    _tabs.setVisible(_visible);
    _syncSeen(force: false);
  }

  void _onOnline() {
    if (sl.get<ConnectivityWatch>().online.value) {
      _feed.poke();
      _tabs.poke();
    }
  }

  void _onShell() {
    _syncSeen(force: true);
  }

  void _onFeed() => _syncSeen(force: false);

  /// The session on screen is «seen» (`POST /session-seen`, like the web's
  /// `viewed()` / `seen_visible()`), and notifications skip it.
  void _syncSeen({required bool force}) {
    final String? on = (!_shell.panelOpen && _visible) ? _shell.active : null;
    if (AppNavigator.visibleChat.value != on) AppNavigator.visibleChat.value = on;
    if (on == null) {
      _seenFor = null;
      return;
    }
    final bool fresh = _seenFor != on;
    _seenFor = on;
    _feed.markSeen(on, onlyIfNew: !(force && fresh));
  }

  Future<void> _loadRemote() async {
    try {
      final ApiResponse? r = await sl.get<Connection>().api?.request('GET', '/remote-state');
      final bool on = r != null && r.ok && r.map['remoteOn'] == true;
      if (mounted && on != _remoteOn) setState(() => _remoteOn = on);
    } catch (_) {
      // The key just stays unlit.
    }
  }

  // ---- actions ------------------------------------------------------------

  void _open(String session, {String? pane}) => _shell.open(session, pane: pane);

  Future<void> _newSession() async {
    final String? s = await showNewSession(context);
    if (s == null || !mounted) return;
    _tabs.poke();
    _feed.poke();
    _open(s);
  }

  void _toggleChat() {
    if (!_shell.toggleChat()) showWebToast(context, 'Abre una sesión primero');
  }

  Future<void> _switcher() async {
    final String? s = await showSessionSwitcher(context);
    if (s == null || !mounted) return;
    _tabs.poke();
    _open(s);
  }

  /// ‹ › of cv-head: previous / next tab chip, wrapping around (chat_view.rs `step`).
  void _step(int delta) {
    final List<TabEntry> tabs = _tabs.tabs;
    if (tabs.isEmpty) return;
    final int at = tabs.indexWhere((TabEntry t) => t.session == _shell.active);
    final int next = ((at < 0 ? 0 : at) + delta) % tabs.length;
    _open(tabs[next < 0 ? next + tabs.length : next].session);
  }

  Future<void> _askClose(TabEntry tab) async {
    final bool? yes = await showDialog<bool>(
      context: context,
      barrierColor: const Color(0x8C000000),
      builder: (BuildContext ctx) => _CloseTabDialog(label: tab.label),
    );
    if (yes != true || !mounted) return;
    final bool ok = await _tabs.close(tab.session);
    if (!mounted) return;
    if (ok) {
      _shell.forget(tab.session);
    } else {
      showWebToast(context, 'No se pudo cerrar la pestaña', error: true);
    }
  }

  void _peek(String session) {
    final SessionSummary? e = _feed.entry(session);
    final double now = _feed.feed?.now ?? 0;
    if (e == null) return;
    final RenderBox? nav = _navKey.currentContext?.findRenderObject() as RenderBox?;
    final double top = nav == null ? 120 : nav.localToGlobal(Offset(0, nav.size.height)).dy + 6;
    unawaited(showPeekCard(context, entry: e, now: now, top: top, onOpen: () => _open(session, pane: e.pane)));
  }

  void _menu() {
    final Outbox outbox = sl.get<Outbox>();
    final int pending = outbox.pendingCount + outbox.attentionCount;
    unawaited(showShellMenu(
      context,
      anchor: _menuKey,
      rows: <MenuAction>[
        MenuAction('Apariencia', () => openWebPanel(context, WebPanel.settings), icon: WebIcons.moon),
        MenuAction('Soberanía', () => openWebPanel(context, WebPanel.fullPage), icon: WebIcons.lock),
      ],
      groups: <List<MenuAction>>[
        <MenuAction>[
          MenuAction('Todas las sesiones', _switcher),
          MenuAction('Perfiles', () => openWebPanel(context, WebPanel.fullPage)),
          MenuAction('Uso de herramientas', () => openWebPanel(context, WebPanel.fullPage)),
        ],
        // The app's own extras (no web equivalent).
        <MenuAction>[
          MenuAction('Modo auto', () => CarMode.open(context, session: _shell.active, pane: _shell.pane)),
          MenuAction('Voz y lectura', AppNavigator.openSettings),
          MenuAction('Avisos en segundo plano', AppNavigator.openSettings),
          MenuAction('Mensajes pendientes', AppNavigator.openSettings,
              detail: pending == 0 ? null : '$pending'),
          MenuAction('Vincular o desvincular', AppNavigator.openSettings),
        ],
      ],
    ));
  }

  // ---- build ----------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Orbita.bg,
      resizeToAvoidBottomInset: true,
      body: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            Container(
              key: _navKey,
              color: Orbita.bg,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: <Widget>[
                  ListenableBuilder(
                    listenable: _shell,
                    builder: (BuildContext context, Widget? _) => NavKeys(
                      panelOpen: _shell.panelOpen,
                      chatMode: _shell.chatMode,
                      onPanel: _shell.togglePanel,
                      onNew: _newSession,
                      onChat: _toggleChat,
                      onOpen: _switcher,
                      onCar: sl.maybe<CarMode>() == null
                          ? null
                          : () => unawaited(CarMode.open(context, session: _shell.active, pane: _shell.pane)),
                    ),
                  ),
                  ListenableBuilder(
                    listenable: Listenable.merge(<Listenable>[_tabs, _feed, _shell]),
                    builder: (BuildContext context, Widget? _) => TabChips(
                      tabs: _tabs.tabs,
                      feed: _feed.feed,
                      active: _shell.panelOpen ? null : _shell.active,
                      onTap: _open,
                      onClose: _askClose,
                      onLongPress: _peek,
                    ),
                  ),
                ],
              ),
            ),
            _StatusStrip(feed: _feed, onOpen: _open),
            Expanded(
              child: ListenableBuilder(
                listenable: _shell,
                builder: (BuildContext context, Widget? _) =>
                    _shell.panelOpen ? _panel() : _session(_shell.active!),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _panel() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        SideTopBar(
          menuKey: _menuKey,
          remoteOn: _remoteOn,
          onMenu: _menu,
          onSwitch: _switcher,
          onSnippets: () => openWebPanel(context, WebPanel.snippets),
          onRemote: () => openWebPanel(context, WebPanel.remote),
          onNews: () => openWebPanel(context, WebPanel.news),
          onSettings: () => openWebPanel(context, WebPanel.settings),
        ),
        Expanded(
          child: SidePanelBody(
            feed: _feed,
            chip: _shell.chip,
            usageTab: _shell.usageTab,
            active: _shell.active,
            onOpen: (SessionSummary e) => _open(e.session, pane: e.pane),
            onServers: () => openWebPanel(context, WebPanel.servers),
            usage: const UsageSummary(),
          ),
        ),
      ],
    );
  }

  Widget _session(String session) {
    return ListenableBuilder(
      listenable: Listenable.merge(<Listenable>[_tabs, _feed]),
      builder: (BuildContext context, Widget? _) {
        final SessionSummary? e = _feed.entry(session);
        final String title = _tabs.find(session)?.label ?? e?.label ?? session;
        final String model = modelLine(e);
        return SessionView(
          key: ValueKey<String>('sv:$session'),
          session: session,
          title: title,
          subtitle: model.isEmpty ? null : model,
          terminal: !_shell.chatMode,
          onPrev: () => _step(-1),
          onNext: () => _step(1),
        );
      },
    );
  }
}

/// Token rejected, feed unreachable (after it loaded once) and the outbox, each a
/// thin `.alert`-like strip under the tabs; nothing when all is well.
class _StatusStrip extends StatelessWidget {
  const _StatusStrip({required this.feed, required this.onOpen});

  final SessionsController feed;
  final void Function(String session) onOpen;

  @override
  Widget build(BuildContext context) {
    final Connection connection = sl.get<Connection>();
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        ValueListenableBuilder<bool>(
          valueListenable: connection.tokenRejected,
          builder: (BuildContext context, bool rejected, Widget? _) => rejected
              ? const BannerStrip(
                  text: 'Tu equipo rechazó el token. Vuelve a vincular en Ajustes.',
                  onTap: AppNavigator.openSettings,
                )
              : const SizedBox.shrink(),
        ),
        ListenableBuilder(
          listenable: feed,
          builder: (BuildContext context, Widget? _) {
            final ApiException? e = feed.error;
            if (e == null || !feed.loadedOnce || e is UnauthorizedException) return const SizedBox.shrink();
            return BannerStrip(text: e.userMessage, color: Orbita.waiting, onTap: feed.refresh);
          },
        ),
        OutboxIndicator(outbox: sl.get<Outbox>(), onOpen: onOpen),
      ],
    );
  }
}

/// The web's `#tabclose` modal: «¿Cerrar la pestaña «X»?», Cancelar / Cerrar pestaña.
class _CloseTabDialog extends StatelessWidget {
  const _CloseTabDialog({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: const Alignment(0, -.7),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16),
        child: Material(
          type: MaterialType.transparency,
          child: Container(
            constraints: const BoxConstraints(maxWidth: 430),
            padding: const EdgeInsets.fromLTRB(24, 24, 24, 22),
            decoration: BoxDecoration(
              color: Orbita.panel2,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: Orbita.line),
              boxShadow: const <BoxShadow>[BoxShadow(color: Color(0x66000000), blurRadius: 60, offset: Offset(0, 20))],
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                Container(
                  padding: const EdgeInsets.only(bottom: 10),
                  margin: const EdgeInsets.only(bottom: 6),
                  decoration: const BoxDecoration(border: Border(bottom: BorderSide(color: Orbita.line))),
                  child: Text(
                    '¿Cerrar la pestaña «$label»?'.toUpperCase(),
                    style: const TextStyle(
                      fontFamily: Orbita.mono,
                      fontSize: 10,
                      letterSpacing: 2.4,
                      fontWeight: FontWeight.w600,
                      color: Orbita.faint,
                    ),
                  ),
                ),
                const Text(
                  'La sesión tmux sigue viva; puedes recuperarla en Recientes. Se cierra aquí y en el escritorio.',
                  style: TextStyle(fontFamily: Orbita.sans, fontSize: 11, height: 1.55, color: Orbita.faint),
                ),
                const SizedBox(height: 14),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: <Widget>[
                    WebButton(
                      radius: 6,
                      lip: 2,
                      face: Orbita.panel,
                      onTap: () => Navigator.of(context).pop(false),
                      child: const Text('Cancelar', style: TextStyle(fontFamily: Orbita.mono, fontSize: 11)),
                    ),
                    const SizedBox(width: 8),
                    WebButton(
                      key: const ValueKey<String>('tabclose-yes'),
                      radius: 6,
                      lip: 2,
                      face: Orbita.panel,
                      ink: Orbita.waiting,
                      onTap: () => Navigator.of(context).pop(true),
                      child: const Text('Cerrar pestaña', style: TextStyle(fontFamily: Orbita.mono, fontSize: 11)),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
