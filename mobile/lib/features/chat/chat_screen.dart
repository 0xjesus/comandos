import 'dart:async';

import 'package:flutter/material.dart';

import '../../app_navigator.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/ask_prompt.dart';
import '../../core/models/transcript.dart';
import '../../core/outbox/drafts.dart';
import '../../core/outbox/outbox.dart';
import '../../core/outbox/outbox_item.dart';
import '../../core/polling/app_visibility.dart';
import '../../ui/theme.dart';
import '../../ui/widgets/banner_strip.dart';
import '../car/car_mode.dart';
import '../voice/chat_voice.dart';
import 'ask_card.dart';
import 'chat_controller.dart';
import 'composer.dart';
import 'message_view.dart';
import 'outbox_bubble.dart';

/// One session as a chat: transcript, our outbox bubbles, the permission
/// card, and the composer at the bottom.
class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key, required this.session, this.pane, this.label});

  static const String routeName = '/chat';

  final String session;
  final String? pane;
  final String? label;

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> with RouteAware {
  late final ChatController _chat = ChatController(
    session: widget.session,
    pane: widget.pane,
    connection: sl.get<Connection>(),
    outbox: _outbox,
  );
  final Outbox _outbox = sl.get<Outbox>();
  final DraftStore _drafts = sl.get<DraftStore>();
  final AppVisibility _appVisible = sl.get<AppVisibility>();
  late final TextEditingController _text = TextEditingController(text: _drafts.get(widget.session));
  final ScrollController _scroll = ScrollController();
  final GlobalKey _centerKey = GlobalKey(debugLabel: 'chat-center');
  final ValueNotifier<bool> _showNewChip = ValueNotifier<bool>(false);

  /// One widget instance per message object: an unchanged message returns
  /// the identical widget, so Flutter skips rebuilding it on every poll.
  final Expando<Widget> _views = Expando<Widget>('chat-views');

  Widget _view(ChatMessage m) => _views[m] ??= MessageView(m, key: ObjectKey(m));

  bool _covered = false;
  bool _stick = true;
  bool _answering = false;
  int _seenVersion = 0;
  int _outboxCount = 0;

  @override
  void initState() {
    super.initState();
    _appVisible.addListener(_syncVisible);
    _chat.listVersion.addListener(_onListChanged);
    _outbox.addListener(_onOutboxChanged);
    _outboxCount = _outbox.itemsFor(widget.session).length;
    _scroll.addListener(_onScroll);
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
    if (AppNavigator.visibleChat.value == widget.session) AppNavigator.visibleChat.value = null;
    _appVisible.removeListener(_syncVisible);
    _chat.listVersion.removeListener(_onListChanged);
    _outbox.removeListener(_onOutboxChanged);
    unawaited(_drafts.flush());
    _chat.dispose();
    _text.dispose();
    _scroll.dispose();
    _showNewChip.dispose();
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
    final bool visible = _appVisible.value && !_covered;
    _chat.setVisible(visible);
    if (visible) {
      AppNavigator.visibleChat.value = widget.session;
    } else if (AppNavigator.visibleChat.value == widget.session) {
      AppNavigator.visibleChat.value = null;
    }
  }

  // ---- Scroll: stick to bottom only when already there --------------------

  bool get _atBottom {
    if (!_scroll.hasClients) return true;
    final ScrollPosition p = _scroll.position;
    return p.pixels >= p.maxScrollExtent - 64;
  }

  void _onScroll() {
    _stick = _atBottom;
    if (_stick && _showNewChip.value) _showNewChip.value = false;
  }

  void _onListChanged() {
    final int v = _chat.listVersion.value;
    if (v == _seenVersion) return;
    _seenVersion = v;
    _afterContentChange();
  }

  void _onOutboxChanged() {
    final int n = _outbox.itemsFor(widget.session).length;
    if (n > _outboxCount) _stick = true; // our own new message: follow it
    _outboxCount = n;
    _afterContentChange();
  }

  void _afterContentChange() {
    if (_stick) {
      _jumpToBottomSoon();
    } else {
      _showNewChip.value = true;
    }
  }

  void _jumpToBottomSoon() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
      // Lazily built children may correct the extent once laid out.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _scroll.hasClients && _stick) {
          _scroll.jumpTo(_scroll.position.maxScrollExtent);
        }
      });
    });
  }

  void _goToBottom() {
    _stick = true;
    _showNewChip.value = false;
    if (!_scroll.hasClients) return;
    _scroll.animateTo(_scroll.position.maxScrollExtent,
        duration: const Duration(milliseconds: 250), curve: Curves.easeOut);
    _jumpToBottomSoon();
  }

  // ---- Actions -------------------------------------------------------------

  void _say(String msg) {
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _answer(AskOption o) async {
    setState(() => _answering = true);
    final String? err = await _chat.answer(o.key);
    if (!mounted) return;
    setState(() => _answering = false);
    if (err != null) _say(err);
  }

  Future<void> _interrupt() async {
    final String? err = await _chat.interrupt();
    if (!mounted) return;
    _say(err ?? 'Enviado Escape: el agente debería detenerse.');
  }

  Future<void> _edit(OutboxItem item) async {
    final String cur = _text.text;
    final String merged = cur.trim().isEmpty ? item.text : '${item.text}\n${cur.trimLeft()}';
    _text.text = merged;
    _text.selection = TextSelection.collapsed(offset: merged.length);
    // Durable in the draft before it leaves the outbox.
    await _drafts.setNow(widget.session, merged);
    await _outbox.discard(item.id);
  }

  Future<void> _discard(OutboxItem item) async {
    final bool? ok = await showDialog<bool>(
      context: context,
      builder: (BuildContext c) => AlertDialog(
        backgroundColor: Orbita.panel,
        title: const Text('¿Descartar este mensaje?'),
        content: Text(
          item.state == OutboxState.uncertain
              ? 'Puede que ya haya llegado. Si lo descartas, desaparece de aquí.'
              : 'No se enviará y el texto se borrará.',
        ),
        actions: <Widget>[
          TextButton(onPressed: () => Navigator.of(c).pop(false), child: const Text('Cancelar')),
          TextButton(
            onPressed: () => Navigator.of(c).pop(true),
            style: TextButton.styleFrom(foregroundColor: Orbita.err),
            child: const Text('Descartar'),
          ),
        ],
      ),
    );
    if (ok == true) await _outbox.discard(item.id);
  }

  // ---- Build ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        titleSpacing: 0,
        title: ListenableBuilder(
          listenable: _chat,
          builder: (BuildContext context, Widget? _) => Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Text(widget.label ?? widget.session, maxLines: 1, overflow: TextOverflow.ellipsis),
              Text(
                _chat.working ? 'Trabajando…' : (_chat.agent ?? ''),
                style: TextStyle(
                    fontSize: 12, color: _chat.working ? Orbita.working : Orbita.faint),
              ),
            ],
          ),
        ),
        actions: <Widget>[
          ListenableBuilder(
            listenable: _chat,
            builder: (BuildContext context, Widget? _) => _chat.working
                ? IconButton(
                    tooltip: 'Detener (Escape)',
                    icon: const Icon(Icons.stop_circle_outlined, color: Orbita.err),
                    onPressed: _interrupt,
                  )
                : const SizedBox.shrink(),
          ),
          const SpeechStopButton(),
          PopupMenuButton<String>(
            tooltip: 'Más',
            onSelected: (String v) {
              if (v == 'car') CarMode.open(context, session: widget.session, pane: _chat.pane);
              if (v == 'stop') _interrupt();
              if (v == autoReadMenuValue) toggleAutoRead(context, widget.session);
            },
            itemBuilder: (BuildContext c) => <PopupMenuEntry<String>>[
              const PopupMenuItem<String>(value: 'car', child: Text('Modo auto')),
              const PopupMenuItem<String>(value: 'stop', child: Text('Enviar Escape (detener)')),
              ?autoReadMenuItem(widget.session),
            ],
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: Column(
          children: <Widget>[
            ChatAutoReader(chat: _chat, session: widget.session),
            ListenableBuilder(
              listenable: _chat,
              builder: (BuildContext context, Widget? _) => _chat.error == null
                  ? const SizedBox.shrink()
                  : BannerStrip(
                      text: _chat.error!.userMessage,
                      color: _chat.gone ? Orbita.err : Orbita.waiting,
                      onTap: _chat.gone ? null : _chat.poke,
                    ),
            ),
            Expanded(
              child: Stack(
                children: <Widget>[
                  ListenableBuilder(
                    listenable: Listenable.merge(<Listenable>[_chat, _outbox]),
                    builder: (BuildContext context, Widget? _) => _buildScroll(),
                  ),
                  Positioned(
                    right: 12,
                    bottom: 12,
                    child: ValueListenableBuilder<bool>(
                      valueListenable: _showNewChip,
                      builder: (BuildContext context, bool show, Widget? _) => show
                          ? ActionChip(
                              avatar: const Icon(Icons.arrow_downward, size: 16),
                              label: const Text('nuevos'),
                              onPressed: _goToBottom,
                              backgroundColor: Orbita.panel2,
                            )
                          : const SizedBox.shrink(),
                    ),
                  ),
                ],
              ),
            ),
            ListenableBuilder(
              listenable: _chat,
              builder: (BuildContext context, Widget? _) {
                final AskPrompt? ask = _chat.ask;
                if (ask == null) return const SizedBox.shrink();
                return AskCard(ask: ask, busy: _answering, onAnswer: _answer);
              },
            ),
            Composer(
              session: widget.session,
              pane: () => _chat.pane,
              baselineFor: _chat.baselineFor,
              controller: _text,
              onSent: () {
                _stick = true;
                _chat.poke();
              },
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildScroll() {
    if (!_chat.loaded && _chat.error == null) {
      return const Center(child: CircularProgressIndicator());
    }
    final List<ChatMessage> msgs = _chat.messages;
    final int split = _chat.split;
    final List<OutboxItem> mine = _outbox.itemsFor(widget.session);
    final bool working = _chat.working;
    return CustomScrollView(
      controller: _scroll,
      center: _centerKey,
      anchor: 1.0,
      slivers: <Widget>[
        // Older messages grow upwards from the anchor; arrivals below never
        // move them, which keeps the reading position when scrolled up.
        SliverList(
          delegate: SliverChildBuilderDelegate(
            (BuildContext context, int i) {
              return _view(msgs[split - 1 - i]);
            },
            childCount: split,
          ),
        ),
        SliverList(
          key: _centerKey,
          delegate: SliverChildBuilderDelegate(
            (BuildContext context, int i) {
              return _view(msgs[split + i]);
            },
            childCount: msgs.length - split,
          ),
        ),
        if (msgs.isEmpty && mine.isEmpty)
          const SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.all(24),
              child: Text(
                'Todavía no hay conversación que mostrar en esta sesión.',
                textAlign: TextAlign.center,
                style: TextStyle(color: Orbita.faint),
              ),
            ),
          ),
        SliverList(
          delegate: SliverChildBuilderDelegate(
            (BuildContext context, int i) {
              final OutboxItem item = mine[i];
              final bool blocked = i > 0 && mine.sublist(0, i).any((OutboxItem e) => !e.isTerminalOk);
              return OutboxBubble(
                key: ValueKey<String>('o:${item.createdAtMs}:${item.text.hashCode}'),
                item: item,
                blocked: blocked,
                onRetry: () => _outbox.retry(item.id, pane: _chat.pane),
                onEdit: () => _edit(item),
                onDiscard: () => _discard(item),
                onArrived: () => _outbox.confirmArrived(item.id),
              );
            },
            childCount: mine.length,
          ),
        ),
        if (working)
          const SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.fromLTRB(16, 8, 16, 8),
              child: Row(
                children: <Widget>[
                  SizedBox(width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2)),
                  SizedBox(width: 10),
                  Text('Trabajando…', style: TextStyle(color: Orbita.working)),
                ],
              ),
            ),
          ),
        const SliverToBoxAdapter(child: SizedBox(height: 8)),
      ],
    );
  }
}
