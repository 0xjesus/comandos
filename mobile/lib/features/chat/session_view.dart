import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show SelectedContent;
import 'package:flutter/services.dart';

import '../../app_navigator.dart';
import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/transcript.dart';
import '../../core/outbox/drafts.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../core/polling/app_visibility.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import '../car/car_mode.dart';
import '../voice/chat_voice.dart';
import '../webpanel/terminal_pane.dart';
import 'ask_card.dart';
import 'chat_controller.dart';
import 'composer.dart';
import 'dock.dart';
import 'markdown.dart';
import 'message_view.dart';
import 'outbox_bubble.dart';
import 'pane_rows.dart';
import 'term_keys.dart';
import 'terminal_tools.dart';

/// Where the collapsed view starts (`chat_view.rs` `cut`): the last agent
/// answer (and what followed it), or the last prompt while no answer exists.
int foldIndex(List<ChatMessage> messages) {
  final int a = messages.lastIndexWhere((ChatMessage m) => m.role == ChatRole.assistant);
  if (a >= 0) return a;
  final int u = messages.lastIndexWhere((ChatMessage m) => m.role == ChatRole.user);
  return u >= 0 ? u : 0;
}

/// «Ver 9 mensajes anteriores» / «Ver 1 mensaje anterior» / «Ocultar mensajes anteriores».
String moreLabel(int folded, {required bool expanded}) {
  if (expanded) return 'Ocultar mensajes anteriores';
  return folded == 1 ? 'Ver 1 mensaje anterior' : 'Ver $folded mensajes anteriores';
}

/// A finger that moved mostly sideways and far enough changes session:
/// +1 next, −1 back, 0 for a scroll, a tap or a hesitant drag (`swipe`).
int swipeStep(double dx, double dy) {
  if (dx.abs() > 70 && dx.abs() > dy.abs() * 2) return dx < 0 ? 1 : -1;
  return 0;
}

/// One session as the mobile web shows it (`#cv-head`, `#chat-view`,
/// `#cv-dock`): the header with ‹ title ›, the agent's conversation (or the
/// live terminal), and the dock with the pane chips, the key row and the
/// composer. The header shows only in chat mode, as on the web.
class SessionView extends StatefulWidget {
  const SessionView({
    super.key,
    required this.session,
    required this.title,
    this.subtitle,
    required this.terminal,
    this.onPrev,
    this.onNext,
  });

  /// tmux session name, e.g. "term-r23327".
  final String session;

  /// Tab label.
  final String title;

  /// Model line under the title, e.g. "fable-5-2 +1".
  final String? subtitle;

  /// false = chat feed, true = live terminal.
  final bool terminal;

  /// The ‹ › buttons of cv-head (and a sideways swipe on the feed).
  final VoidCallback? onPrev, onNext;

  @override
  State<SessionView> createState() => _SessionViewState();
}

class _SessionViewState extends State<SessionView> {
  @override
  Widget build(BuildContext context) {
    // Everything that belongs to one session is rebuilt when the session changes.
    return _SessionBody(key: ValueKey<String>(widget.session), view: widget);
  }
}

class _SessionBody extends StatefulWidget {
  const _SessionBody({super.key, required this.view});
  final SessionView view;

  @override
  State<_SessionBody> createState() => _SessionBodyState();
}

class _SessionBodyState extends State<_SessionBody> with RouteAware {
  SessionView get w => widget.view;
  String get session => w.session;

  final Connection _connection = sl.get<Connection>();
  final Outbox _outbox = sl.get<Outbox>();
  final DraftStore _drafts = sl.get<DraftStore>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();

  late final ChatController _chat = ChatController(
    session: session,
    pane: savedPanes[session],
    connection: _connection,
    outbox: _outbox,
    // The web draws the server's window (≤80 messages); nothing older.
    cap: 80,
  );
  late final PaneTracker _panes = PaneTracker(session: session, connection: _connection);
  late final TermKeys _keys = TermKeys(connection: _connection);
  late final TextEditingController _text = TextEditingController(text: _drafts.get(session));
  final FocusNode _focus = FocusNode(debugLabel: 'cv-compose');
  final ScrollController _scroll = ScrollController();
  final GlobalKey _centerKey = GlobalKey(debugLabel: 'cv-center');
  final BubbleEdits _edits = BubbleEdits();

  /// One widget instance per message object: an unchanged message returns
  /// the identical widget, so Flutter skips rebuilding it on every poll.
  final Expando<Widget> _views = Expando<Widget>('cv-views');

  bool _covered = false;
  bool _stick = true;
  bool _expanded = false;
  bool _answering = false;
  bool _ctrlArmed = false;
  bool _direct = false;
  String _lastText = '';
  String? _pushedPane;
  int _seenVersion = 0;
  int _outboxCount = 0;
  Offset? _swipeFrom;
  bool _swipeCancelled = false;

  Widget _view(ChatMessage m) => _views[m] ??= MessageView(m, key: ObjectKey(m), edits: _edits);

  /// The pane the dock targets: the one chosen here, else what the server read.
  String? get _pane => savedPanes[session] ?? _chat.pane;
  bool get _split => _panes.rows.length > 1;

  @override
  void initState() {
    super.initState();
    _lastText = _text.text;
    _appVisible.addListener(_syncVisible);
    _chat.listVersion.addListener(_onListChanged);
    _chat.addListener(_onChatChanged);
    _outbox.addListener(_onOutboxChanged);
    _panes.addListener(_onPanes);
    _text.addListener(_onText);
    _outboxCount = _outbox.itemsFor(session).length;
    _scroll.addListener(_onScroll);
    markdownLinkTap = (String url) {
      unawaited(Clipboard.setData(ClipboardData(text: url)));
      _say('Enlace copiado: $url');
    };
    _syncVisible();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ModalRoute<void>? route = ModalRoute.of(context);
    if (route != null) AppNavigator.routes.subscribe(this, route);
  }

  @override
  void didUpdateWidget(_SessionBody old) {
    super.didUpdateWidget(old);
    if (old.view.terminal != w.terminal) _syncVisible();
  }

  @override
  void dispose() {
    AppNavigator.routes.unsubscribe(this);
    if (AppNavigator.visibleChat.value == session) AppNavigator.visibleChat.value = null;
    _appVisible.removeListener(_syncVisible);
    _chat.listVersion.removeListener(_onListChanged);
    _chat.removeListener(_onChatChanged);
    _outbox.removeListener(_onOutboxChanged);
    _panes.removeListener(_onPanes);
    _text.removeListener(_onText);
    unawaited(_drafts.flush());
    _chat.dispose();
    _panes.dispose();
    _keys.close();
    _text.dispose();
    _focus.dispose();
    _scroll.dispose();
    _edits.dispose();
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

  /// Polls only while shown: the transcript in chat mode, the pane rows in both.
  void _syncVisible() {
    final bool visible = _appVisible.value && !_covered;
    _chat.setVisible(visible && !w.terminal);
    _panes.setVisible(visible);
    if (visible) {
      AppNavigator.visibleChat.value = session;
    } else if (AppNavigator.visibleChat.value == session) {
      AppNavigator.visibleChat.value = null;
    }
  }

  // ---- Panes ---------------------------------------------------------------

  void _onPanes() {
    final ({String pane, bool closed}) c = choosePane(_panes.rows, savedPanes[session] ?? '');
    if (c.closed) {
      final PaneRow? row = _panes.rows.where((PaneRow r) => r.pane == c.pane).firstOrNull;
      _say('El pane que veías se cerró; ahora estás en ${row == null ? c.pane : paneLabel(row)}');
    }
    if (_panes.rows.isNotEmpty && c.pane.isNotEmpty) savedPanes[session] = c.pane;
    _pushPane();
    if (mounted) setState(() {});
  }

  void _pushPane() {
    final String? p = savedPanes[session];
    if (p == null || p == _pushedPane) return;
    _pushedPane = p;
    _expanded = false;
    _stick = true;
    if (_edits.editing case final ChatMessage m) _edits.restore(m);
    _chat.setPane(p);
  }

  void _selectPane(String pane) {
    if (savedPanes[session] == pane) return;
    savedPanes[session] = pane;
    _pushPane();
    setState(() {});
  }

  // ---- Scroll: stick to the bottom only when already there -----------------

  bool get _atBottom {
    if (!_scroll.hasClients) return true;
    final ScrollPosition p = _scroll.position;
    return p.pixels >= p.maxScrollExtent - 120;
  }

  void _onScroll() => _stick = _atBottom;

  void _onListChanged() {
    final int v = _chat.listVersion.value;
    if (v == _seenVersion) return;
    _seenVersion = v;
    if (_stick && _edits.editing == null) _jumpToBottomSoon();
  }

  String? _askSeen;
  bool _workingSeen = false;

  /// A prompt or the typing dots appearing below the last message keep a
  /// reader who was at the bottom there.
  void _onChatChanged() {
    final String? ask = _chat.ask?.signature;
    final bool working = _chat.working;
    if (ask == _askSeen && working == _workingSeen) return;
    _askSeen = ask;
    _workingSeen = working;
    if (_stick && _edits.editing == null) _jumpToBottomSoon();
  }

  void _onOutboxChanged() {
    final int n = _outbox.itemsFor(session).length;
    if (n > _outboxCount) _stick = true; // our own new message: follow it
    _outboxCount = n;
    if (_stick) _jumpToBottomSoon();
  }

  void _jumpToBottomSoon() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
      // Lazily built children may correct the extent once laid out.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _scroll.hasClients && _stick) _scroll.jumpTo(_scroll.position.maxScrollExtent);
      });
    });
  }

  // ---- Messages to the person ---------------------------------------------

  void _say(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(msg)));
  }

  // ---- Composer: Ctrl and Teclado directo ----------------------------------

  void _onText() {
    final String t = _text.text;
    final String before = _lastText;
    _lastText = t;
    if (!_ctrlArmed || t.length != before.length + 1) return;
    // The next letter typed while «Ctrl» is armed goes to the pane as Ctrl+letter.
    int at = 0;
    while (at < before.length && before.codeUnitAt(at) == t.codeUnitAt(at)) {
      at++;
    }
    final String ch = t[at];
    _lastText = before;
    _text.value = TextEditingValue(text: before, selection: TextSelection.collapsed(offset: at.clamp(0, before.length)));
    setState(() => _ctrlArmed = false);
    unawaited(_keys.ctrl(ch, session: session, pane: _pane, split: _split).then((String? e) {
      if (e != null) _say(e);
    }));
  }

  Future<void> _sendEnter() async {
    if (_chat.ask != null && !w.terminal) {
      _say(kAnswerPromptFirst);
      return;
    }
    final String? e = await _keys.press(TermKey.enter, session: session, pane: _pane, split: _split);
    if (e != null) _say(e);
    _chat.poke();
  }

  // ---- Dock keys -----------------------------------------------------------

  Future<void> _onKey(DockKey k) async {
    final TermKey? term = switch (k) {
      DockKey.escape => TermKey.escape,
      DockKey.ctrlc => TermKey.ctrlc,
      DockKey.backspace => TermKey.backspace,
      DockKey.tab => TermKey.tab,
      DockKey.left => TermKey.left,
      DockKey.up => TermKey.up,
      DockKey.down => TermKey.down,
      DockKey.right => TermKey.right,
      _ => null,
    };
    if (term != null) {
      final String? e = await _keys.press(term, session: session, pane: _pane, split: _split);
      if (e != null) _say(e);
      if (term == TermKey.escape || term == TermKey.ctrlc) _chat.poke();
      return;
    }
    switch (k) {
      case DockKey.one || DockKey.two:
        final String? e = await _keys.httpKey(k == DockKey.one ? '1' : '2', session: session, pane: _pane);
        if (e != null) _say(e);
        _chat.poke();
      case DockKey.select || DockKey.history:
        await showTerminalHistory(context, session: session, pane: _pane);
      case DockKey.copy:
        await showTerminalHistory(context,
            session: session, pane: _pane, status: 'Selecciona texto para copiar, o pulsa Copiar todo.');
      case DockKey.panes:
        final String? p = await showPanes(context, session: session);
        if (p != null && p.startsWith('%')) {
          _selectPane(p);
          unawaited(_panes.refresh());
        }
      case DockKey.paste:
        await _paste();
      case DockKey.keyboard:
        setState(() {
          _direct = !_direct;
          _ctrlArmed = false;
        });
      case DockKey.ctrl:
        setState(() => _ctrlArmed = !_ctrlArmed);
        if (_ctrlArmed && !_direct) _focus.requestFocus();
      default:
        break;
    }
  }

  /// «Pegar»: in the chat it fills the draft; over the terminal it pastes into
  /// the pane (bracketed, no Enter), as the web does.
  Future<void> _paste() async {
    final ClipboardData? clip = await Clipboard.getData(Clipboard.kTextPlain);
    final String text = clip?.text ?? '';
    if (text.isEmpty) {
      _say('No se pudo leer el portapapeles; mantén presionado el campo para pegar');
      return;
    }
    if (!w.terminal) {
      final String next = '${_text.text}$text';
      _text.value = TextEditingValue(text: next, selection: TextSelection.collapsed(offset: next.length));
      _focus.requestFocus();
      return;
    }
    final ComandosApi? api = _connection.api;
    if (api == null) return;
    try {
      await api.requestOk('POST', '/paste', body: <String, Object?>{'session': session, 'pane': ?_pane, 'text': text});
    } on ApiException catch (e) {
      _say(e.userMessage);
    }
  }

  // ---- Feed actions -------------------------------------------------------

  Future<void> _answer(AskPrompt shown, AskOption o) async {
    setState(() => _answering = true);
    final String? err = await _chat.answerPrompt(shown, o);
    if (!mounted) return;
    setState(() => _answering = false);
    if (err != null) _say(err);
  }

  /// "Editar" on a failed message: the text goes back to the composer. If
  /// later messages of this session wait behind it, the item stays in the
  /// queue holding them until the edited text is sent (it takes the item's
  /// place) or the person discards the edit.
  Future<void> _editOutbox(OutboxItem item) async {
    final String cur = _text.text;
    final String merged = cur.trim().isEmpty ? item.text : '${item.text}\n${cur.trimLeft()}';
    _text.text = merged;
    _text.selection = TextSelection.collapsed(offset: merged.length);
    await _drafts.setNow(session, merged);
    final List<OutboxItem> all = _outbox.itemsFor(session);
    final int at = all.indexWhere((OutboxItem i) => i.id == item.id);
    final bool holdsOthers = at >= 0 && all.skip(at + 1).any((OutboxItem i) => i.isPending || i.needsHuman);
    if (holdsOthers) {
      await _outbox.beginEdit(item.id);
    } else {
      await _outbox.discard(item.id);
    }
  }

  void _toggleMore() {
    _edits.finishOpen();
    setState(() => _expanded = !_expanded);
  }

  // ---- Swipe between sessions ---------------------------------------------

  void _pointerDown(PointerDownEvent e) {
    _swipeFrom = e.position;
    _swipeCancelled = _edits.editing != null;
  }

  void _pointerUp(PointerUpEvent e) {
    final Offset? from = _swipeFrom;
    _swipeFrom = null;
    if (from == null || _swipeCancelled) return;
    final Offset d = e.position - from;
    final int step = swipeStep(d.dx, d.dy);
    if (step > 0) w.onNext?.call();
    if (step < 0) w.onPrev?.call();
  }

  bool _onInnerScroll(ScrollNotification n) {
    // Scrolling code or a table sideways is not a swipe.
    if (n.metrics.axis == Axis.horizontal && n is ScrollStartNotification) _swipeCancelled = true;
    return false;
  }

  // ---- Build ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final double keyboard = MediaQuery.viewInsetsOf(context).bottom;
    return ColoredBox(
      color: Orbita.bg,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          if (!w.terminal) _Head(title: w.title, subtitle: w.subtitle, onPrev: w.onPrev, onNext: w.onNext),
          Expanded(
            child: w.terminal
                ? TerminalPane(session: session, pane: _pane, label: w.title)
                : Stack(
                    fit: StackFit.expand,
                    children: <Widget>[
                      ChatAutoReader(chat: _chat, session: session),
                      _feed(),
                    ],
                  ),
          ),
          _dock(keyboard),
        ],
      ),
    );
  }

  Widget _feed() {
    return Listener(
      onPointerDown: _pointerDown,
      onPointerUp: _pointerUp,
      child: NotificationListener<ScrollNotification>(
        onNotification: _onInnerScroll,
        child: SelectionArea(
          onSelectionChanged: (SelectedContent? c) {
            // Selecting text with a long press is not a swipe.
            if (c != null && c.plainText.isNotEmpty) _swipeCancelled = true;
          },
          child: ListenableBuilder(
            listenable: Listenable.merge(<Listenable>[_chat, _outbox]),
            builder: (BuildContext context, Widget? _) => _buildScroll(),
          ),
        ),
      ),
    );
  }

  Widget _buildScroll() {
    final ApiException? error = _chat.error;
    final Widget scroll = _scrollView();
    if (error == null) return scroll;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _Strip(text: error.userMessage, color: _chat.gone ? Orbita.err : Orbita.waiting, onTap: _chat.gone ? null : _chat.poke),
        Expanded(child: scroll),
      ],
    );
  }

  Widget _scrollView() {
    if (!_chat.loaded && _chat.error == null) {
      return ListView(padding: const EdgeInsets.fromLTRB(12, 14, 12, 20), children: const <Widget>[ChatSkeleton()]);
    }
    final List<ChatMessage> msgs = _chat.messages;
    final List<OutboxItem> all = _outbox.itemsFor(session);
    // The item being edited lives in the composer now.
    final List<OutboxItem> mine = all.where((OutboxItem i) => !i.editing).toList();
    final AskPrompt? ask = _chat.ask;
    final bool working = _chat.working && ask == null;
    final int folded = foldIndex(msgs);
    final int from = _expanded ? 0 : folded;
    final int split = _chat.split.clamp(0, msgs.length);
    final int olderCount = split > from ? split - from : 0;
    final int newerFrom = split > from ? split : from;
    final bool more = folded > 0;
    String? empty;
    if (_chat.noAgent) {
      empty = _split
          ? 'Este pane es una terminal sin agente con historial. Elige otro pane arriba del teclado o usa la terminal.'
          : 'Esta sesión no tiene un agente con historial (Claude o Codex). Usa la terminal.';
    } else if (msgs.isEmpty && mine.isEmpty) {
      empty = 'Sin conversación todavía. Escribe abajo o vuelve a la terminal.';
    }
    const EdgeInsets side = EdgeInsets.symmetric(horizontal: 12);
    Widget gap(Widget child) => Padding(padding: const EdgeInsets.only(top: 12), child: child);
    return CustomScrollView(
      controller: _scroll,
      center: _centerKey,
      anchor: 1.0,
      slivers: <Widget>[
        // Older messages grow upwards from the anchor; arrivals below never
        // move them, which keeps the reading position when scrolled up.
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(12, 14, 12, 0),
          sliver: SliverList(
            delegate: SliverChildBuilderDelegate(
              (BuildContext context, int i) {
                if (i < olderCount) {
                  final Widget v = _view(msgs[split - 1 - i]);
                  return i == olderCount - 1 && !more ? v : gap(v);
                }
                return _MorePill(label: moreLabel(folded, expanded: _expanded), onTap: _toggleMore);
              },
              childCount: olderCount + (more ? 1 : 0),
            ),
          ),
        ),
        SliverPadding(
          key: _centerKey,
          padding: side,
          sliver: SliverList(
            delegate: SliverChildBuilderDelegate(
              (BuildContext context, int i) {
                final Widget v = _view(msgs[newerFrom + i]);
                return olderCount == 0 && !more && i == 0 ? v : gap(v);
              },
              childCount: msgs.length - newerFrom,
            ),
          ),
        ),
        if (empty != null)
          SliverFillRemaining(
            hasScrollBody: false,
            child: Center(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 32 * 8.0),
                  child: Text(empty,
                      textAlign: TextAlign.center,
                      style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, height: 1.4, color: Orbita.dim)),
                ),
              ),
            ),
          ),
        SliverPadding(
          padding: side,
          sliver: SliverList(
            delegate: SliverChildBuilderDelegate(
              (BuildContext context, int i) {
                final OutboxItem item = mine[i];
                final int at = all.indexOf(item);
                final bool blocked = at > 0 && all.sublist(0, at).any((OutboxItem e) => !e.isTerminalOk);
                return gap(OutboxBubble(
                  key: ValueKey<String>('o:${item.id}'),
                  item: item,
                  blocked: blocked,
                  onRetry: () => _outbox.retry(item.id, pane: _pane),
                  onEdit: () => _editOutbox(item),
                  onDiscard: () => _outbox.discard(item.id),
                  onArrived: () => _outbox.confirmArrived(item.id),
                ));
              },
              childCount: mine.length,
            ),
          ),
        ),
        if (ask != null)
          SliverPadding(
            padding: side,
            sliver: SliverToBoxAdapter(
              child: gap(AskCard(
                key: ValueKey<String>(ask.signature),
                ask: ask,
                busy: _answering,
                onAnswer: (AskOption o) => _answer(ask, o),
              )),
            ),
          ),
        if (working) SliverPadding(padding: side, sliver: SliverToBoxAdapter(child: gap(const TypingBubble()))),
        const SliverToBoxAdapter(child: SizedBox(height: 20)),
      ],
    );
  }

  Widget _dock(double keyboard) {
    final double safe = keyboard > 0 ? 0 : MediaQuery.paddingOf(context).bottom;
    final List<PaneRow> rows = _panes.rows;
    final String pane = _pane ?? '';
    return DecoratedBox(
      decoration: const BoxDecoration(
        color: Orbita.panel,
        border: Border(top: BorderSide(color: Orbita.line)),
      ),
      child: Padding(
        padding: EdgeInsets.only(bottom: keyboard + safe),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            PaneChips(rows: rows, pane: pane, onSelect: _selectPane),
            DockKeys(
              onKey: (DockKey k) => unawaited(_onKey(k)),
              ctrlArmed: _ctrlArmed,
              direct: _direct,
              extra: <Widget>[
                AutoReadKey(
                  session: session,
                  builder: (String label, bool on, VoidCallback onTap) => DockKeyButton(label: label, on: on, onTap: onTap),
                ),
                if (sl.maybe<CarMode>() != null)
                  DockKeyButton(
                    label: 'Modo auto',
                    onTap: () => unawaited(CarMode.open(context, session: session, pane: _pane)),
                  ),
              ],
            ),
            ListenableBuilder(
              listenable: _outbox,
              builder: (BuildContext context, Widget? _) {
                final OutboxItem? editing = _outbox.editingFor(session);
                if (editing == null) return const SizedBox.shrink();
                return EditHoldNote(held: _outbox.heldBehindEdit(session), onDiscard: () => _outbox.discard(editing.id));
              },
            ),
            if (_direct)
              _DirectKeyboard(
                ctrlArmed: _ctrlArmed,
                onCtrlUsed: () => setState(() => _ctrlArmed = false),
                send: (String bytes) => _keys.typeRaw(bytes, session: session, pane: _pane, split: _split),
                onSay: _say,
              )
            else
              ListenableBuilder(
                listenable: _chat,
                builder: (BuildContext context, Widget? _) => Composer(
                  session: session,
                  pane: () => _pane,
                  baselineFor: _chat.baselineFor,
                  controller: _text,
                  focusNode: _focus,
                  hint: composerHint(rows, pane),
                  onSay: _say,
                  // A message typed into a pane showing a permission prompt
                  // could answer it: sending waits until the prompt is gone.
                  sendBlockedReason: !w.terminal && _chat.ask != null ? kAnswerPromptFirst : null,
                  onEmptySend: () => unawaited(_sendEnter()),
                  onSent: () {
                    _stick = true;
                    _jumpToBottomSoon();
                    _chat.poke();
                  },
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// `#cv-head`: ‹ (40 px, radius 12) · centred bold title over the mono model
/// line · ›, on `--panel` with a bottom rule.
class _Head extends StatelessWidget {
  const _Head({required this.title, this.subtitle, this.onPrev, this.onNext});
  final String title;
  final String? subtitle;
  final VoidCallback? onPrev, onNext;

  @override
  Widget build(BuildContext context) {
    Widget arrow(String glyph, String label, VoidCallback? onTap) => WebButton(
          onTap: onTap,
          tooltip: label,
          size: 40,
          radius: 12,
          lip: 2,
          face: Colors.transparent,
          ink: Orbita.text,
          child: Text(glyph, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 22, height: 1, color: Orbita.text)),
        );
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
      decoration: const BoxDecoration(
        color: Orbita.panel,
        border: Border(bottom: BorderSide(color: Orbita.line)),
      ),
      child: Row(
        children: <Widget>[
          arrow('‹', 'Sesión anterior', onPrev),
          const SizedBox(width: 6),
          Expanded(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Text(title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.center,
                    style: const TextStyle(fontFamily: Orbita.sans, fontSize: 15, fontWeight: FontWeight.w600, color: Orbita.text)),
                if (subtitle != null && subtitle!.isNotEmpty) ...<Widget>[
                  const SizedBox(height: 1),
                  Text(subtitle!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      textAlign: TextAlign.center,
                      style: const TextStyle(fontFamily: Orbita.mono, fontSize: 10, fontWeight: FontWeight.w600, color: Orbita.faint)),
                ],
              ],
            ),
          ),
          const SizedBox(width: 6),
          arrow('›', 'Sesión siguiente', onNext),
        ],
      ),
    );
  }
}

/// `.cv-more`: the pill that unfolds the older messages.
class _MorePill extends StatelessWidget {
  const _MorePill({required this.label, required this.onTap});
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Center(
        child: SelectionContainer.disabled(
          child: WebButton(
            onTap: onTap,
            face: Orbita.panel,
            ink: Orbita.dim,
            radius: 999,
            lip: 2,
            minHeight: 36,
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Text(label, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, fontWeight: FontWeight.w600)),
          ),
        ),
      );
}

/// A one-line notice at the top of the feed (connection trouble, session gone).
class _Strip extends StatelessWidget {
  const _Strip({required this.text, required this.color, this.onTap});
  final String text;
  final Color color;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => GestureDetector(
        onTap: onTap,
        child: Container(
          margin: const EdgeInsets.fromLTRB(12, 10, 12, 0),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
          decoration: BoxDecoration(
            color: color.withValues(alpha: .08),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: Orbita.line),
          ),
          child: Text(text, style: TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: color)),
        ),
      );
}

/// «Teclado directo»: every key typed here goes straight to the pane over the
/// terminal WebSocket (letters, ⌫ as Backspace, ↵ as Enter), like typing into
/// the terminal itself. Nothing is kept as a draft; a key that cannot go is
/// dropped and reported, never queued.
class _DirectKeyboard extends StatefulWidget {
  const _DirectKeyboard({required this.send, required this.ctrlArmed, required this.onCtrlUsed, required this.onSay});

  final Future<String?> Function(String bytes) send;
  final bool ctrlArmed;
  final VoidCallback onCtrlUsed;
  final void Function(String message) onSay;

  @override
  State<_DirectKeyboard> createState() => _DirectKeyboardState();
}

class _DirectKeyboardState extends State<_DirectKeyboard> {
  /// A fixed base the field returns to after every key, so a Backspace on an
  /// "empty" field is still seen.
  static const String _base = '​​';
  final TextEditingController _c = TextEditingController(text: _base);

  @override
  void initState() {
    super.initState();
    _c.selection = const TextSelection.collapsed(offset: _base.length);
    _c.addListener(_onChange);
  }

  @override
  void dispose() {
    _c.removeListener(_onChange);
    _c.dispose();
    super.dispose();
  }

  void _onChange() {
    final String t = _c.text;
    if (t == _base) return;
    int p = 0;
    while (p < t.length && p < _base.length && t.codeUnitAt(p) == _base.codeUnitAt(p)) {
      p++;
    }
    final int deleted = _base.length - p;
    String typed = t.substring(p).replaceAll('​', '');
    _c.value = const TextEditingValue(text: _base, selection: TextSelection.collapsed(offset: _base.length));
    String bytes = '\x7f' * deleted;
    if (widget.ctrlArmed && typed.isNotEmpty) {
      final int? b = controlByte(typed[0]);
      widget.onCtrlUsed();
      if (b != null) {
        bytes += String.fromCharCode(b);
        typed = typed.substring(1);
      }
    }
    bytes += typed.replaceAll('\n', '\r');
    if (bytes.isEmpty) return;
    unawaited(widget.send(bytes).then((String? e) {
      if (e != null) widget.onSay(e);
    }));
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 2, 8, 8),
      child: Row(
        children: <Widget>[
          Expanded(
            child: TextField(
              controller: _c,
              autofocus: true,
              autocorrect: false,
              enableSuggestions: false,
              keyboardType: TextInputType.visiblePassword,
              textInputAction: TextInputAction.newline,
              maxLines: 1,
              onSubmitted: (_) => unawaited(widget.send('\r').then((String? e) {
                if (e != null) widget.onSay(e);
              })),
              cursorColor: Orbita.brand,
              style: const TextStyle(fontFamily: Orbita.mono, fontSize: 16, color: Colors.transparent),
              decoration: InputDecoration(
                // The field always holds two invisible characters, so the hint
                // is a prefix (shown while it has content).
                prefixText: 'Teclado directo: cada tecla va a la terminal',
                prefixStyle: TextStyle(fontFamily: Orbita.sans, fontSize: 15, color: Orbita.text.withValues(alpha: .5)),
                filled: true,
                fillColor: Orbita.bg,
                isDense: true,
                contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
                border: _b(Orbita.brand),
                enabledBorder: _b(Orbita.brand),
                focusedBorder: _b(Orbita.brand),
              ),
            ),
          ),
          const SizedBox(width: 8),
          WebButton(
            tooltip: 'Enter',
            onTap: () => unawaited(widget.send('\r').then((String? e) {
              if (e != null) widget.onSay(e);
            })),
            brand: true,
            size: 44,
            radius: 22,
            lip: 2,
            child: const Text('↵', style: TextStyle(fontSize: 20, color: Colors.white)),
          ),
        ],
      ),
    );
  }

  static OutlineInputBorder _b(Color c) => OutlineInputBorder(borderRadius: BorderRadius.circular(14), borderSide: BorderSide(color: c));
}
