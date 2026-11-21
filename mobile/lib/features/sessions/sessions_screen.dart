import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/api/api_errors.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/session_feed.dart';
import '../../core/outbox/outbox.dart';
import '../../core/polling/app_visibility.dart';
import '../../core/polling/connectivity_watch.dart';
import '../../ui/format.dart';
import '../../ui/theme.dart';
import '../../ui/widgets/banner_strip.dart';
import '../../ui/widgets/outbox_indicator.dart';
import '../car/car_mode.dart';
import 'sections.dart';
import 'session_row.dart';
import 'sessions_controller.dart';

/// Home: every live session, grouped by what needs the person first.
class SessionsScreen extends StatefulWidget {
  const SessionsScreen({super.key});

  static const String routeName = '/sessions';

  @override
  State<SessionsScreen> createState() => _SessionsScreenState();
}

class _SessionsScreenState extends State<SessionsScreen> with RouteAware {
  late final SessionsController _controller = SessionsController(sl.get<Connection>());
  final Outbox _outbox = sl.get<Outbox>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  final ValueNotifier<String> _query = ValueNotifier<String>('');
  final TextEditingController _search = TextEditingController();
  bool _covered = false;

  /// Row widget cache: session → (signature, widget). Reusing the identical
  /// widget instance makes Flutter skip rebuilding unchanged rows.
  final Map<String, (String, Widget)> _rows = <String, (String, Widget)>{};

  @override
  void initState() {
    super.initState();
    _appVisible.addListener(_syncVisible);
    sl.maybe<ConnectivityWatch>()?.online.addListener(_onOnline);
    _syncVisible();
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
    _controller.dispose();
    _query.dispose();
    _search.dispose();
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

  void _syncVisible() => _controller.setVisible(_appVisible.value && !_covered);

  void _onOnline() {
    if (sl.get<ConnectivityWatch>().online.value) _controller.poke();
  }

  void _open(SessionSummary s) =>
      AppNavigator.openChat(s.session, pane: s.pane, label: s.label);

  void _openBySession(String session) {
    SessionSummary? found;
    for (final SessionSummary s in _controller.feed?.sessions ?? const <SessionSummary>[]) {
      if (s.session == session) found = s;
    }
    AppNavigator.openChat(session, pane: found?.pane, label: found?.label);
  }

  Widget _row(SessionSummary s, double now) {
    final String when = relativeTime(now, s.ts);
    final int unsettled = _outbox.unsettledFor(s.session);
    final String sig = '${s.signature}\u0002$when\u0002$unsettled';
    final (String, Widget)? cached = _rows[s.session];
    if (cached != null && cached.$1 == sig) return cached.$2;
    final Widget w = SessionRow(
      key: ValueKey<String>(s.session),
      summary: s,
      when: when,
      unsettled: unsettled,
      onTap: () => _open(s),
    );
    _rows[s.session] = (sig, w);
    return w;
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('ComandOS'),
        actions: <Widget>[
          IconButton(
            tooltip: 'Modo auto',
            icon: const Icon(Icons.directions_car_outlined),
            onPressed: () => CarMode.open(context),
          ),
          IconButton(
            tooltip: 'Ajustes',
            icon: const Icon(Icons.settings_outlined),
            onPressed: AppNavigator.openSettings,
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: Column(
          children: <Widget>[
            ValueListenableBuilder<bool>(
              valueListenable: sl.get<Connection>().tokenRejected,
              builder: (BuildContext context, bool rejected, Widget? _) => rejected
                  ? const BannerStrip(
                      text: 'Tu equipo rechazó el token. Vuelve a vincular en Ajustes.',
                      onTap: AppNavigator.openSettings,
                    )
                  : const SizedBox.shrink(),
            ),
            Expanded(
              child: ListenableBuilder(
                listenable: Listenable.merge(<Listenable>[_controller, _query, _outbox]),
                builder: (BuildContext context, Widget? _) => _buildList(),
              ),
            ),
            OutboxIndicator(outbox: _outbox, onOpen: _openBySession),
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
              child: TextField(
                controller: _search,
                onChanged: (String v) => _query.value = v,
                textInputAction: TextInputAction.search,
                decoration: InputDecoration(
                  hintText: 'Buscar sesión',
                  prefixIcon: const Icon(Icons.search, color: Orbita.faint),
                  suffixIcon: ValueListenableBuilder<String>(
                    valueListenable: _query,
                    builder: (BuildContext context, String q, Widget? _) => q.isEmpty
                        ? const SizedBox.shrink()
                        : IconButton(
                            tooltip: 'Borrar búsqueda',
                            icon: const Icon(Icons.close, color: Orbita.dim),
                            onPressed: () {
                              _search.clear();
                              _query.value = '';
                            },
                          ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildList() {
    final SessionFeed? feed = _controller.feed;
    final ApiException? error = _controller.error;
    if (feed == null) {
      if (error != null) {
        return _Message(
          text: error.userMessage,
          action: 'Reintentar',
          onAction: _controller.refresh,
        );
      }
      return const Center(child: CircularProgressIndicator());
    }
    final List<SessionSection> sections = buildSections(feed.sessions, query: _query.value);
    // Drop cached rows for sessions that disappeared.
    final Set<String> live = <String>{for (final SessionSummary s in feed.sessions) s.session};
    _rows.removeWhere((String k, (String, Widget) _) => !live.contains(k));

    final List<Object> entries = <Object>[];
    for (final SessionSection sec in sections) {
      entries.add(sec);
      entries.addAll(sec.sessions);
    }
    return Column(
      children: <Widget>[
        if (error != null)
          BannerStrip(text: error.userMessage, color: Orbita.waiting, onTap: _controller.refresh),
        Expanded(
          child: RefreshIndicator(
            onRefresh: _controller.refresh,
            child: entries.isEmpty
                ? ListView(
                    physics: const AlwaysScrollableScrollPhysics(),
                    children: <Widget>[
                      const SizedBox(height: 120),
                      Center(
                        child: Text(
                          _query.value.isEmpty
                              ? 'No hay sesiones con agentes abiertas en tu equipo.'
                              : 'Ninguna sesión coincide con «${_query.value}».',
                          style: const TextStyle(color: Orbita.dim),
                        ),
                      ),
                    ],
                  )
                : ListView.builder(
                    physics: const AlwaysScrollableScrollPhysics(),
                    itemCount: entries.length,
                    itemBuilder: (BuildContext context, int i) {
                      final Object e = entries[i];
                      if (e is SessionSection) {
                        return SectionHeader(e.kind.title, e.sessions.length,
                            key: ValueKey<String>('h:${e.kind.name}'));
                      }
                      return _row(e as SessionSummary, feed.now);
                    },
                  ),
          ),
        ),
      ],
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.text, required this.action, required this.onAction});

  final String text;
  final String action;
  final Future<void> Function() onAction;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            Text(text, textAlign: TextAlign.center, style: const TextStyle(color: Orbita.dim, fontSize: 15)),
            const SizedBox(height: 16),
            OutlinedButton(onPressed: onAction, child: Text(action)),
          ],
        ),
      ),
    );
  }
}
