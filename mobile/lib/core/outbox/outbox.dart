import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import '../util/clock.dart';
import 'message_transport.dart';
import 'outbox_item.dart';
import 'outbox_store.dart';
import 'send_result.dart';

/// Thrown by [Outbox.enqueue] when the text cannot be accepted. The composer
/// keeps the text and shows [message].
class OutboxRejection implements Exception {
  const OutboxRejection(this.message);
  final String message;
  @override
  String toString() => 'OutboxRejection: $message';
}

/// Removes the control characters the desktop refuses (it answers 422 and
/// a retry would fail the same way). Line breaks become `\n` (as the
/// desktop does); `\n` and `\t` are kept.
String sanitizeOutgoing(String text) => text
    .replaceAll('\r\n', '\n')
    .replaceAll('\r', '\n')
    .replaceAll(RegExp(r'[\u0000-\u0008\u000B-\u001F\u007F-\u009F]'), '');

/// Persistent, idempotent, strictly ordered outgoing message queue.
///
/// Guarantees:
/// * [enqueue] completes only after the message is durable on disk; the
///   composer clears after that, so pressing send never loses text.
/// * Every automatic retry reuses the same `clientMessageId`.
/// * Strict FIFO per session: message 2 is not attempted while message 1 is
///   queued, sending, failed or uncertain (until delivered or discarded).
/// * Definitive rejections (404 pane/session gone, 4xx, 401/403) are never
///   retried automatically; the person chooses Reintentar/Editar/Descartar.
/// * The legacy `/send` route is used at most once per item and its failures
///   become [OutboxState.uncertain], never an automatic resend.
///
/// Time and timers are injected; tests drive the queue with [pump].
class Outbox extends ChangeNotifier {
  Outbox({
    required OutboxStore store,
    required MessageTransport transport,
    NowMs now = systemNowMs,
    TimerFactory timer = systemTimer,
    String Function()? newId,
  })  : _store = store,
        _transport = transport,
        _now = now,
        _timer = timer,
        _newId = newId ?? (() => const Uuid().v4());

  /// Longest accepted message. Longer text is refused, never truncated.
  static const int maxChars = 20000;

  /// Legacy `/send` silently truncates above this; we refuse instead.
  static const int legacyMaxChars = 4000;

  /// Retry delays after network errors / 5xx / 202 (then [retryCap] forever).
  static const List<Duration> retryDelays = <Duration>[
    Duration(seconds: 1),
    Duration(seconds: 2),
    Duration(seconds: 5),
    Duration(seconds: 10),
    Duration(seconds: 30),
  ];
  static const Duration retryCap = Duration(seconds: 60);

  /// Retry delays while the pane is busy (409/429).
  static const List<Duration> busyDelays = <Duration>[
    Duration(milliseconds: 700),
    Duration(seconds: 1),
    Duration(seconds: 2),
    Duration(seconds: 3),
  ];
  static const Duration busyCap = Duration(seconds: 5);

  /// Retry spacing while a permission prompt is open in the pane.
  static const Duration promptRetry = Duration(seconds: 3);

  /// How long a delivered bubble stays if the transcript never shows it.
  static const Duration deliveredLinger = Duration(minutes: 2);

  /// Automatic transcript checks for an uncertain item, and their spacing.
  static const int uncertainChecks = 8;
  static const Duration uncertainCheckEvery = Duration(seconds: 15);

  final OutboxStore _store;
  final MessageTransport _transport;
  final NowMs _now;
  final TimerFactory _timer;
  final String Function() _newId;

  List<OutboxItem> _items = <OutboxItem>[];
  final Map<String, Future<void>> _inFlight = <String, Future<void>>{};
  final Set<String> _notDurable = <String>{};
  Timer? _wake;
  int _wakeAtMs = 0;
  bool _active = true;
  bool _loaded = false;
  String? _host;

  /// All items, oldest first.
  List<OutboxItem> get items => List<OutboxItem>.unmodifiable(_items);

  /// Items of one session, oldest first.
  List<OutboxItem> itemsFor(String session) =>
      _items.where((OutboxItem i) => i.session == session).toList(growable: false);

  /// Queued or being sent.
  int get pendingCount => _items.where((OutboxItem i) => i.isPending).length;

  /// Failed or uncertain: waiting for the person.
  int get attentionCount => _items.where((OutboxItem i) => i.needsHuman).length;

  /// The session of the oldest item that is pending or needs the person.
  String? get firstUnsettledSession {
    for (final OutboxItem i in _items) {
      if (i.isPending || i.needsHuman) return i.session;
    }
    return null;
  }

  /// The item of [session] whose text the person is editing ("Editar"),
  /// holding the later messages of that session; null when none.
  OutboxItem? editingFor(String session) {
    for (final OutboxItem i in _items) {
      if (i.session == session && i.editing) return i;
    }
    return null;
  }

  /// Messages of [session] waiting behind the one being edited.
  int heldBehindEdit(String session) {
    bool behind = false;
    int n = 0;
    for (final OutboxItem i in _items) {
      if (i.session != session) continue;
      if (i.editing) {
        behind = true;
      } else if (behind && (i.isPending || i.needsHuman)) {
        n++;
      }
    }
    return n;
  }

  /// Unsettled (pending + needs human) count for one session.
  int unsettledFor(String session) => _items
      .where((OutboxItem i) => i.session == session && (i.isPending || i.needsHuman))
      .length;

  OutboxItem? byId(String id) {
    for (final OutboxItem i in _items) {
      if (i.id == id) return i;
    }
    return null;
  }

  /// Loads the persisted queue and repairs items that were mid-flight when
  /// the process died. Safe to call once at boot.
  Future<void> load() async {
    final List<OutboxItem> loaded = await _store.load();
    final int now = _now();
    _items = <OutboxItem>[
      for (final OutboxItem i in loaded)
        if (i.state == OutboxState.sending)
          i.legacyTried
              // Legacy /send has no id: we cannot know. Ask the person.
              ? i.copyWith(
                  state: OutboxState.uncertain,
                  failCode: FailCode.uncertain,
                  checksLeft: uncertainChecks,
                  nextAttemptAtMs: now)
              // Idempotent route: ask for the receipt, then resend the same id.
              : i.copyWith(state: OutboxState.queued, ambiguous: true, nextAttemptAtMs: now)
        else
          i,
    ];
    _loaded = true;
    notifyListeners();
  }

  /// Sets the paired host. Items are only ever sent to the host they were
  /// written for; null (unpaired) holds everything.
  void setHost(String? host) {
    _host = host;
    _kick();
  }

  /// Foreground/background. While inactive no new attempts start and no
  /// timer is armed; work in flight finishes. [setActive(true)] also nudges.
  void setActive(bool active) {
    if (_active == active) return;
    _active = active;
    if (!active) {
      _wake?.cancel();
      _wake = null;
    } else {
      nudge();
    }
  }

  /// Connectivity came back or the app resumed: retry waiting items now.
  void nudge() {
    final int now = _now();
    bool changed = false;
    _items = <OutboxItem>[
      for (final OutboxItem i in _items)
        if (i.state == OutboxState.queued && i.nextAttemptAtMs > now)
          () {
            changed = true;
            return i.copyWith(nextAttemptAtMs: now);
          }()
        else
          i,
    ];
    if (changed) notifyListeners();
    _kick();
  }

  /// Adds a message. Completes after it is durable; then delivery starts.
  ///
  /// [transcriptBaseline] is how many user messages with this exact text the
  /// chat currently shows (lets the chat hide the bubble once it appears).
  ///
  /// [replaces]: the id of the item being edited ([beginEdit]); the new
  /// message takes its place in the queue, ahead of the messages it held.
  ///
  /// Control characters the desktop refuses are removed first (`\n` and
  /// `\t` stay), so a paste never fails for good.
  Future<OutboxItem> enqueue({
    required String session,
    String? pane,
    required String text,
    int? transcriptBaseline,
    String? replaces,
  }) async {
    text = sanitizeOutgoing(text);
    if (!_loaded) throw const OutboxRejection('Un momento: todavía estoy cargando.');
    final String host = _host ?? '';
    if (host.isEmpty) {
      throw const OutboxRejection('Vincula el teléfono con tu equipo antes de enviar.');
    }
    if (text.trim().isEmpty) throw const OutboxRejection('El mensaje está vacío.');
    if (text.length > maxChars) {
      throw OutboxRejection('El mensaje tiene ${text.length} caracteres; el máximo es '
          '$maxChars. Divídelo en partes; no lo recorto.');
    }
    int? baseline = transcriptBaseline;
    if (baseline != null) {
      final String norm = normalizeForMatch(text);
      baseline += _items
          .where((OutboxItem i) =>
              i.session == session &&
              i.state != OutboxState.failed &&
              normalizeForMatch(i.text) == norm)
          .length;
    }
    final OutboxItem item = OutboxItem(
      id: _newId(),
      host: host,
      session: session,
      pane: pane,
      text: text,
      createdAtMs: _now(),
      nextAttemptAtMs: _now(),
      transcriptBaseline: baseline,
    );
    // The item joins the in-memory list first so any concurrent write also
    // carries it, but it is not attempted until its own write is durable.
    final int at = replaces == null
        ? -1
        : _items.indexWhere((OutboxItem i) => i.id == replaces && i.editing && i.session == session);
    final OutboxItem? replaced = at >= 0 ? _items[at] : null;
    _items = at >= 0
        ? <OutboxItem>[..._items.sublist(0, at), item, ..._items.sublist(at + 1)]
        : <OutboxItem>[..._items, item];
    _notDurable.add(item.id);
    try {
      await _store.save(_items);
    } catch (_) {
      // Not durable: take it back out (and put back what it replaced). The
      // caller keeps the text.
      _items = <OutboxItem>[
        for (final OutboxItem i in _items)
          if (i.id != item.id) i else ?replaced,
      ];
      unawaited(_persist());
      throw const OutboxRejection('No pude guardar el mensaje en el teléfono. '
          'Sigue en el cuadro de texto; inténtalo de nuevo.');
    } finally {
      _notDurable.remove(item.id);
    }
    notifyListeners();
    _kick();
    return item;
  }

  /// Person-initiated retry of a failed or uncertain item.
  ///
  /// "Reintentar" on a failed item keeps its clientMessageId: the desktop
  /// keeps no receipt for a rejection, and if the message did arrive after
  /// all (a timeout whose receipt check was then refused), it answers
  /// `duplicate` instead of typing it twice. "Reenviar" on an uncertain item
  /// is the person's explicit resend: a fresh id.
  ///
  /// [pane] targets the session's current pane. After `pane_gone`, a pane
  /// equal to the dead one (or none) is cleared, so the desktop picks the
  /// session's current pane.
  Future<void> retry(String id, {String? pane}) async {
    final OutboxItem? item = byId(id);
    if (item == null || !item.needsHuman) return;
    final bool stalePane = item.failCode == FailCode.paneGone && (pane == null || pane == item.pane);
    await _replace(item.copyWith(
      id: item.state == OutboxState.uncertain ? _newId() : item.id,
      host: _host ?? item.host,
      pane: stalePane ? null : pane,
      clearPane: stalePane,
      clearWaitCode: true,
      editing: false,
      state: OutboxState.queued,
      retryStreak: 0,
      nextAttemptAtMs: _now(),
      clearLastError: true,
      clearFailCode: true,
      ambiguous: false,
      legacyTried: false,
      checksLeft: 0,
    ), oldId: id);
    _kick();
  }

  /// "Editar" on a failed / uncertain item: its text goes to the composer,
  /// and the item stays where it is, holding the later messages of its
  /// session, until the edited text replaces it (`enqueue(replaces:)`) or
  /// the person discards it. Nothing jumps ahead silently.
  Future<void> beginEdit(String id) async {
    final OutboxItem? item = byId(id);
    if (item == null || !item.needsHuman || item.editing) return;
    await _replace(item.copyWith(editing: true));
  }

  /// Removes an item (after the person confirmed), or after its text was put
  /// back in the composer ("Editar"). Unblocks the session's queue.
  Future<void> discard(String id) async {
    final List<OutboxItem> next = _items.where((OutboxItem i) => i.id != id).toList();
    if (next.length == _items.length) return;
    _items = next;
    notifyListeners();
    await _persist();
    _kick();
  }

  /// The person says an uncertain message did arrive.
  Future<void> confirmArrived(String id) => discard(id);

  /// The chat saw a new transcript window for [session]: hides delivered
  /// bubbles whose text appeared and settles uncertain items that appeared.
  void observeTranscript(String session, List<String> userTexts) {
    bool changed = false;
    final Set<String> remove = <String>{};
    final List<OutboxItem> updated = <OutboxItem>[];
    final List<String> lastThree =
        userTexts.length <= 3 ? userTexts : userTexts.sublist(userTexts.length - 3);
    for (final OutboxItem i in _items) {
      if (i.session != session ||
          (i.state != OutboxState.delivered && i.state != OutboxState.uncertain)) {
        updated.add(i);
        continue;
      }
      final int? base = i.transcriptBaseline;
      bool seen;
      if (base != null) {
        final int count = userTexts.where((String t) => transcriptTextMatches(t, i.text)).length;
        seen = count > base;
      } else {
        // Without a baseline only a delivered item may be hidden (the server
        // already confirmed it); an uncertain one stays for the person.
        seen = i.state == OutboxState.delivered &&
            lastThree.any((String t) => transcriptTextMatches(t, i.text));
      }
      if (seen) {
        remove.add(i.id);
        changed = true;
      } else {
        updated.add(i);
      }
    }
    if (!changed) return;
    _items = updated;
    notifyListeners();
    unawaited(_persist());
    _kick();
  }

  /// Runs every attempt that is due now and waits until all have finished
  /// (including follow-ups they unblock). Production code never needs to
  /// await this; tests use it as the clock's "tick".
  Future<void> pump() async {
    _kick();
    while (_inFlight.isNotEmpty) {
      await Future.wait(_inFlight.values.toList());
    }
  }

  @override
  void dispose() {
    _wake?.cancel();
    super.dispose();
  }

  // ---- Engine -------------------------------------------------------------

  void _kick() {
    if (!_loaded || !_active) return;
    final int now = _now();
    final Set<String> sessionsSeen = <String>{};
    final List<String> expired = <String>[];
    for (final OutboxItem i in _items) {
      if (i.state == OutboxState.delivered) {
        if (now - (i.deliveredAtMs ?? now) >= deliveredLinger.inMilliseconds) expired.add(i.id);
        continue; // delivered items never block the queue
      }
      if (!sessionsSeen.add(i.session)) continue; // not the head of its session
      if (i.editing) continue; // the person is rewriting it: the session waits
      if (_notDurable.contains(i.id)) continue;
      if (_inFlight.containsKey(i.session)) continue;
      if (i.state == OutboxState.failed) continue; // blocked until the person acts
      if (i.state == OutboxState.uncertain) {
        if (i.checksLeft > 0 && i.transcriptBaseline != null && i.nextAttemptAtMs <= now) {
          _start(i.session, _check(i));
        }
        continue;
      }
      if (i.host != _host) {
        if (_host != null && _host!.isNotEmpty) {
          _start(i.session, _replace(i.copyWith(
            state: OutboxState.failed,
            failCode: FailCode.otherHost,
          )));
        }
        continue;
      }
      if (i.nextAttemptAtMs <= now) _start(i.session, _attempt(i));
    }
    if (expired.isNotEmpty) {
      _items = _items.where((OutboxItem i) => !expired.contains(i.id)).toList();
      notifyListeners();
      unawaited(_persist());
    }
    _armTimer();
  }

  void _start(String session, Future<void> work) {
    late final Future<void> tracked;
    tracked = work.catchError((Object _) {}).whenComplete(() {
      if (identical(_inFlight[session], tracked)) _inFlight.remove(session);
      _kick();
    });
    _inFlight[session] = tracked;
  }

  void _armTimer() {
    if (!_active) return;
    final int now = _now();
    int? earliest;
    for (final OutboxItem i in _items) {
      int? due;
      if (i.editing) {
        due = null;
      } else if (i.state == OutboxState.queued && !_inFlight.containsKey(i.session)) {
        due = i.nextAttemptAtMs;
      } else if (i.state == OutboxState.uncertain &&
          i.checksLeft > 0 &&
          i.transcriptBaseline != null) {
        due = i.nextAttemptAtMs;
      } else if (i.state == OutboxState.delivered) {
        due = (i.deliveredAtMs ?? now) + deliveredLinger.inMilliseconds;
      }
      if (due != null && due > now && (earliest == null || due < earliest)) earliest = due;
    }
    if (earliest == null) {
      _wake?.cancel();
      _wake = null;
      return;
    }
    if (_wake != null && _wakeAtMs == earliest) return;
    _wake?.cancel();
    _wakeAtMs = earliest;
    _wake = _timer(Duration(milliseconds: earliest - now), () {
      _wake = null;
      _kick();
    });
  }

  Duration _backoff(int streak, List<Duration> table, Duration cap) =>
      streak < table.length ? table[streak] : cap;

  Future<void> _attempt(OutboxItem start) async {
    OutboxItem item = start.copyWith(
      state: OutboxState.sending,
      attempts: start.attempts + 1,
    );
    await _replace(item);

    SendResult result;
    try {
      if (item.ambiguous) {
        result = await _transport.lookup(item);
        if (result is UnknownReceipt || result is RouteMissing || result is TransientFailure) {
          // Not there (or cannot ask): sending the same id again is safe.
          result = await _transport.send(item);
        }
      } else {
        result = await _transport.send(item);
      }
    } catch (_) {
      result = const TransientFailure('Error inesperado; reintentando.');
    }

    // The item may have been discarded meanwhile.
    final OutboxItem? current = byId(item.id);
    if (current == null) return;
    item = result is PromptOpen ? current : current.copyWith(clearWaitCode: true);
    final int now = _now();

    switch (result) {
      case Delivered(:final String? pane):
        await _replace(item.copyWith(
          state: OutboxState.delivered,
          deliveredAtMs: now,
          pane: pane,
          clearLastError: true,
          clearFailCode: true,
          ambiguous: false,
          retryStreak: 0,
        ));
      case InProgress():
        await _requeue(item, 'Tu equipo lo está escribiendo…', ambiguous: true);
      case PromptOpen():
        // Nothing typed, no receipt: same id again in a few seconds.
        await _replace(item.copyWith(
          state: OutboxState.queued,
          waitCode: WaitCode.promptOpen,
          retryStreak: 0,
          nextAttemptAtMs: now + promptRetry.inMilliseconds,
          lastError: kWaitingForPrompt,
        ));
      case Busy():
        await _replace(item.copyWith(
          state: OutboxState.queued,
          retryStreak: item.retryStreak + 1,
          nextAttemptAtMs: now + _backoff(item.retryStreak, busyDelays, busyCap).inMilliseconds,
          lastError: 'La sesión está ocupada; reintento en un momento.',
        ));
      case TransientFailure(:final String message):
        await _requeue(item, message, ambiguous: true);
      case UnknownReceipt():
        await _requeue(item, 'Reintentando…', ambiguous: false);
      case Rejected(:final String code):
        await _replace(item.copyWith(
          state: OutboxState.failed,
          failCode: code,
          clearLastError: true,
        ));
      case Uncertain():
        await _markUncertain(item);
      case RouteMissing():
        await _legacy(item);
    }
  }

  Future<void> _requeue(OutboxItem item, String message, {required bool ambiguous}) =>
      _replace(item.copyWith(
        state: OutboxState.queued,
        ambiguous: ambiguous || item.ambiguous,
        retryStreak: item.retryStreak + 1,
        nextAttemptAtMs:
            _now() + _backoff(item.retryStreak, retryDelays, retryCap).inMilliseconds,
        lastError: message,
      ));

  Future<void> _markUncertain(OutboxItem item) => _replace(item.copyWith(
        state: OutboxState.uncertain,
        failCode: FailCode.uncertain,
        clearLastError: true,
        checksLeft: uncertainChecks,
        nextAttemptAtMs: _now() + const Duration(seconds: 3).inMilliseconds,
      ));

  /// Old server without `/session/message`: one shot at `/send`.
  Future<void> _legacy(OutboxItem item) async {
    if (item.legacyTried) {
      await _markUncertain(item);
      return;
    }
    if (item.text.length > legacyMaxChars) {
      await _replace(item.copyWith(state: OutboxState.failed, failCode: FailCode.legacyTooLong));
      return;
    }
    final int? baseline = item.transcriptBaseline ??
        await _transport.countUserMessages(item.session, item.pane, item.text);
    // Persist "legacy tried" BEFORE the request: if we die mid-request, the
    // item comes back as uncertain, never as a blind resend.
    OutboxItem marked = item.copyWith(
      legacyTried: true,
      state: OutboxState.sending,
      transcriptBaseline: baseline,
    );
    if (byId(item.id) == null) return;
    await _replace(marked);
    SendResult r;
    try {
      r = await _transport.legacySend(marked);
    } catch (_) {
      r = const Uncertain();
    }
    marked = byId(item.id) ?? marked;
    if (byId(item.id) == null) return;
    switch (r) {
      case Delivered():
        await _replace(marked.copyWith(
          state: OutboxState.delivered,
          deliveredAtMs: _now(),
          clearLastError: true,
        ));
      case Rejected(:final String code):
        await _replace(marked.copyWith(state: OutboxState.failed, failCode: code));
      default:
        await _markUncertain(marked);
    }
  }

  /// Uncertain item: does the transcript now show it?
  Future<void> _check(OutboxItem item) async {
    final int? count =
        await _transport.countUserMessages(item.session, item.pane, item.text);
    final OutboxItem? current = byId(item.id);
    if (current == null || current.state != OutboxState.uncertain || current.editing) return;
    final int? base = current.transcriptBaseline;
    if (count != null && base != null && count > base) {
      // It arrived. Show it as delivered until the chat sees it too.
      await _replace(current.copyWith(
        state: OutboxState.delivered,
        deliveredAtMs: _now(),
        clearFailCode: true,
        checksLeft: 0,
      ));
      return;
    }
    await _replace(current.copyWith(
      checksLeft: current.checksLeft - 1,
      nextAttemptAtMs: _now() + uncertainCheckEvery.inMilliseconds,
    ));
  }

  Future<void> _replace(OutboxItem item, {String? oldId}) async {
    final String match = oldId ?? item.id;
    bool found = false;
    _items = <OutboxItem>[
      for (final OutboxItem i in _items)
        if (i.id == match)
          () {
            found = true;
            return item;
          }()
        else
          i,
    ];
    if (!found) return;
    notifyListeners();
    await _persist();
  }

  Future<void> _persist() async {
    try {
      await _store.save(_items);
    } catch (e) {
      // The in-memory queue keeps working; the next successful write
      // includes everything. Surface it for debugging only.
      debugPrint('outbox: could not persist (${e.runtimeType})');
    }
  }
}
