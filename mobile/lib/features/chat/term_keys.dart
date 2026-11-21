import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/config/paired_server.dart';
import '../../core/connection.dart';

/// A key of the dock's key row (`data-key` in `chat_view.rs` `BARS`).
enum TermKey { escape, ctrlc, backspace, tab, left, up, down, right, enter }

/// Bytes the terminal page types for each key (`comandos-term-web` `parent.rs`
/// `toolbar_key`).
const Map<TermKey, String> termKeyBytes = <TermKey, String>{
  TermKey.escape: '\x1b',
  TermKey.ctrlc: '\x03',
  TermKey.backspace: '\x7f',
  TermKey.tab: '\t',
  TermKey.up: '\x1b[A',
  TermKey.down: '\x1b[B',
  TermKey.right: '\x1b[C',
  TermKey.left: '\x1b[D',
  TermKey.enter: '\r',
};

/// `POST /key` names (the route allows Enter Escape Up Down Tab y n 1-9).
/// Keys missing here (Ctrl+C, ⌫, ←, →) only exist on the terminal WebSocket.
const Map<TermKey, String> termKeyHttp = <TermKey, String>{
  TermKey.escape: 'Escape',
  TermKey.tab: 'Tab',
  TermKey.up: 'Up',
  TermKey.down: 'Down',
  TermKey.enter: 'Enter',
};

/// ASCII Ctrl mapping of the armed «Ctrl» key (`parent.rs` `control_byte`):
/// space → NUL, `@A-Z[\]^_` and `a-z` → 0x00–0x1f. Null: not a Ctrl key.
int? controlByte(String ch) {
  if (ch.length != 1) return null;
  final int b = ch.codeUnitAt(0);
  if (b == 0x20) return 0;
  if ((b >= 64 && b <= 95) || (b >= 97 && b <= 122)) return b & 31;
  return null;
}

/// Something that can type raw bytes into a pane (the terminal WebSocket).
abstract class TermInput {
  /// Types [bytes] into [pane] of [session]. Null when done, else the
  /// Spanish reason. Never queued for later: a press that cannot go now is
  /// dropped and reported.
  Future<String?> type({required String session, String? pane, required bool split, required String bytes});

  void close();
}

/// Sends dock keys: `POST /key` where the route allows the key (it targets the
/// pane explicitly and refuses a pane that is gone), the terminal WebSocket
/// (`/term/ws`, subprotocol `comandos.term.v1`) for the rest.
class TermKeys {
  TermKeys({required Connection connection, TermInput? input})
      : _connection = connection,
        _input = input ?? debugInput?.call(connection) ?? TermSocket(connection);

  /// Tests: replaces the WebSocket of every new [TermKeys].
  @visibleForTesting
  static TermInput Function(Connection connection)? debugInput;

  final Connection _connection;
  final TermInput _input;

  /// Null when sent, else a Spanish message for a toast.
  Future<String?> press(TermKey key, {required String session, String? pane, required bool split}) async {
    final String? name = termKeyHttp[key];
    if (name != null) return httpKey(name, session: session, pane: pane);
    return _input.type(session: session, pane: pane, split: split, bytes: termKeyBytes[key]!);
  }

  /// `POST /key` with a digit or a key name.
  Future<String?> httpKey(String key, {required String session, String? pane}) async {
    final ComandosApi? api = _connection.api;
    if (api == null) return 'Sin vincular.';
    try {
      await api.key(session: session, pane: pane, key: key);
      return null;
    } on ApiException catch (e) {
      return e is NotFoundException ? 'Ese pane ya no existe.' : 'No se pudo enviar la tecla';
    }
  }

  /// Ctrl + [ch] over the WebSocket. Null when sent.
  Future<String?> ctrl(String ch, {required String session, String? pane, required bool split}) {
    final int? b = controlByte(ch);
    if (b == null) return Future<String?>.value('Ctrl solo combina con una letra');
    return _input.type(session: session, pane: pane, split: split, bytes: String.fromCharCode(b));
  }

  /// Raw bytes over the WebSocket («Teclado directo»). Null when sent.
  Future<String?> typeRaw(String bytes, {required String session, String? pane, required bool split}) =>
      _input.type(session: session, pane: pane, split: split, bytes: bytes);

  void close() => _input.close();
}

/// A short-lived terminal WebSocket: opened on the first key that needs it,
/// closed after [idle] without presses or when the view goes away. On a split
/// it focuses the phone's pane for this client only (`/terminal-panes`
/// `select` with `scope: client`, then the returned `clientKeys`), as the
/// session's terminal frame does before the web forwards a key to it.
class TermSocket implements TermInput {
  TermSocket(this._connection, {this.idle = const Duration(seconds: 30)});

  final Connection _connection;
  final Duration idle;

  WebSocket? _ws;
  String? _session;
  String? _focused;
  Future<WebSocket?>? _opening;
  Timer? _idleTimer;
  Future<void> _chain = Future<void>.value();

  @override
  Future<String?> type({required String session, String? pane, required bool split, required String bytes}) {
    // One press at a time, in order.
    final Completer<String?> done = Completer<String?>();
    _chain = _chain.then((_) async {
      done.complete(await _type(session, pane, split, bytes));
    });
    return done.future;
  }

  Future<String?> _type(String session, String? pane, bool split, String bytes) async {
    final PairedServer? server = _connection.server;
    final ComandosApi? api = _connection.api;
    if (server == null || api == null) return 'Sin vincular.';
    if (_session != session) _drop();
    WebSocket? ws = _ws;
    if (ws == null || ws.readyState != WebSocket.open) {
      _drop();
      ws = await (_opening ??= _open(server, session));
      _opening = null;
      if (ws == null) return 'No se pudo enviar la tecla: la terminal remota no responde';
      _ws = ws;
      _session = session;
    }
    if (split && pane != null && pane.isNotEmpty && _focused != pane) {
      final String? keys = await _focusKeys(api, session, pane);
      if (keys == null) return 'No se pudo enfocar ese pane; la tecla no se envió';
      ws.add('0$keys');
      _focused = pane;
    }
    if (ws.readyState != WebSocket.open) {
      _drop();
      return 'No se pudo enviar la tecla: la terminal se desconectó';
    }
    ws.add('0$bytes');
    _idleTimer?.cancel();
    _idleTimer = Timer(idle, _drop);
    return null;
  }

  Future<WebSocket?> _open(PairedServer server, String session) async {
    final Uri base = server.baseUri;
    final String origin = base.origin;
    try {
      final WebSocket ws = await WebSocket.connect(
        base.replace(scheme: 'wss', path: '/term/ws').toString(),
        protocols: const <String>['comandos.term.v1'],
        headers: <String, Object>{'X-Comandos-Token': server.token, 'Origin': origin},
      ).timeout(const Duration(seconds: 6));
      // `ignoreSize`: a phone never resizes the shared desktop window.
      ws.add(jsonEncode(<String, Object>{'v': 1, 'cols': 80, 'rows': 24, 'session': session, 'ignoreSize': true}));
      // Output is not drawn here; drain it so the socket never backs up.
      ws.listen((Object? _) {}, onError: (Object _) => _dropIf(ws), onDone: () => _dropIf(ws), cancelOnError: true);
      return ws;
    } catch (_) {
      return null;
    }
  }

  Future<String?> _focusKeys(ComandosApi api, String session, String pane) async {
    try {
      final ApiResponse list = await api.request('POST', '/terminal-panes',
          body: <String, Object?>{'session': session, 'action': 'list'});
      final Object? panes = list.map['panes'];
      if (!list.ok || panes is! List) return null;
      final Object? row = panes.firstWhere((Object? p) => p is Map && p['id'] == pane, orElse: () => null);
      if (row is! Map) return null;
      final ApiResponse sel = await api.request('POST', '/terminal-panes', body: <String, Object?>{
        'session': session,
        'action': 'select',
        'pane': row['id'],
        'identity': row['identity'],
        'scope': 'client',
      });
      final Object? keys = sel.map['clientKeys'];
      return sel.ok && keys is String ? keys : null;
    } on ApiException {
      return null;
    }
  }

  void _dropIf(WebSocket ws) {
    if (identical(ws, _ws)) _drop();
  }

  void _drop() {
    _idleTimer?.cancel();
    _idleTimer = null;
    final WebSocket? ws = _ws;
    _ws = null;
    _focused = null;
    _session = null;
    if (ws != null) unawaited(ws.close().catchError((Object _) {}));
  }

  @override
  void close() => _drop();
}
