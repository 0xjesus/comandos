import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

import '../../core/util/atomic_json_file.dart';
import '../sessions/sections.dart';

/// Where the shell keeps the last session and the chat/terminal choice across cold
/// starts (the web keeps them in localStorage: `comandos.viewMode`, device focus).
abstract class ShellStore {
  Future<Map<String, Object?>?> read();
  Future<void> write(Map<String, Object?> value);
}

/// `<app support>/shell.json`, written atomically.
class FileShellStore implements ShellStore {
  AtomicJsonFile? _file;

  Future<AtomicJsonFile> _open() async =>
      _file ??= AtomicJsonFile('${(await getApplicationSupportDirectory()).path}/shell.json');

  @override
  Future<Map<String, Object?>?> read() async {
    try {
      final Object? v = await (await _open()).read();
      return v is Map<String, Object?> ? v : null;
    } catch (_) {
      return null;
    }
  }

  @override
  Future<void> write(Map<String, Object?> value) async {
    try {
      await (await _open()).write(value);
    } catch (_) {
      // Best effort: losing the last session is harmless.
    }
  }
}

/// What the one-screen shell shows: the side panel or one session, in chat or
/// terminal mode, plus the feed filter. Mirrors the web's `activeView`
/// (`panel` / `term:<session>`), `activeTerm` and `comandos.viewMode`.
///
/// [AppNavigator] reaches the live shell through [ShellController.current] (or
/// [ShellController.openSession], which also works before the shell exists).
class ShellController extends ChangeNotifier {
  ShellController({ShellStore? store}) : _store = store;

  final ShellStore? _store;

  /// The mounted shell's controller, if any.
  static ShellController? current;

  /// A session asked for before the shell existed (notification tap at boot).
  static (String, String?)? _pending;

  /// Opens [session] in the shell (from a notification, car mode, …). If the shell
  /// is not mounted yet, it opens that session as soon as it is.
  static void openSession(String session, {String? pane}) {
    final ShellController? c = current;
    if (c != null) {
      c.open(session, pane: pane);
    } else {
      _pending = (session, pane);
    }
  }

  String? _active;
  String? _pane;
  bool _panel = true;
  bool _chat = true; // phones open in chat unless the person chose the terminal
  bool _restored = false;

  /// Feed filter chip (kept while the panel closes and opens).
  final ValueNotifier<FeedChip> chip = ValueNotifier<FeedChip>(FeedChip.all);

  /// Side panel tab: false = «Sesiones», true = «Consumos».
  final ValueNotifier<bool> usageTab = ValueNotifier<bool>(false);

  /// Session shown when the panel is closed (`activeTerm`).
  String? get active => _active;

  /// Pane asked for by the last [open] (notifications); null = the session's own.
  String? get pane => _pane;

  /// Side panel on screen (`activeView == "panel"`).
  bool get panelOpen => _panel || _active == null;

  /// Chat (true) or live terminal (false).
  bool get chatMode => _chat;

  /// Loads the last session / mode and takes a pending [openSession].
  Future<void> restore() async {
    final Map<String, Object?>? saved = await _store?.read();
    if (saved != null && !_restored) {
      final Object? s = saved['session'];
      final Object? mode = saved['mode'];
      if (_active == null && s is String && s.isNotEmpty) _active = s;
      if (mode == 'term') _chat = false;
    }
    _restored = true;
    final (String, String?)? p = _pending;
    _pending = null;
    if (p != null) {
      open(p.$1, pane: p.$2);
    } else {
      notifyListeners();
    }
  }

  /// Shows [session] (tab chip, feed row, switcher, notification).
  void open(String session, {String? pane}) {
    if (session.isEmpty) return;
    final bool changed = _active != session || _panel || _pane != pane;
    _active = session;
    _pane = pane;
    _panel = false;
    if (changed) {
      _save();
      notifyListeners();
    }
  }

  /// `#tab-panel`: panel ⇄ the active session.
  void togglePanel() {
    if (_active == null) return;
    _panel = !_panel;
    notifyListeners();
  }

  void showPanel() {
    if (_panel) return;
    _panel = true;
    notifyListeners();
  }

  /// `#tab-chat`. Returns false when there is no session to show
  /// («Abre una sesión primero»). Turning the chat on also leaves the panel.
  bool toggleChat() {
    if (!_chat && _active == null) return false;
    _chat = !_chat;
    if (_chat) _panel = false;
    _save();
    notifyListeners();
    return true;
  }

  /// The active tab was closed: back to the panel.
  void forget(String session) {
    if (_active != session) return;
    _active = null;
    _pane = null;
    _panel = true;
    _save();
    notifyListeners();
  }

  void _save() {
    final ShellStore? s = _store;
    if (s == null) return;
    unawaited(s.write(<String, Object?>{'session': _active, 'mode': _chat ? 'chat' : 'term'}));
  }

  @override
  void dispose() {
    if (identical(current, this)) current = null;
    chip.dispose();
    usageTab.dispose();
    super.dispose();
  }
}
