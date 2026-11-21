import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_client.dart';
import '../../core/config/paired_server.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/polling/app_visibility.dart';
import 'web_surface.dart';
import 'web_urls.dart';

/// Everything the web panels need from the rest of the app, replaceable in
/// tests (`WebPorts.surfaces = FakeSurface.new`, …; `WebPorts.reset()`).
abstract final class WebPorts {
  static WebSurfaceFactory surfaces = FlutterWebSurface.new;
  static WebStore store = const FlutterWebStore();
  static PairedServer? Function() server = _server;
  static Future<ApiResponse> Function(String method, String path,
      {Map<String, String>? query, Map<String, Object?>? body}) request = _request;
  static ValueListenable<bool>? Function() visibility = _visibility;

  /// Kept-warm terminal lifetime after it leaves the screen (see terminal_pane.dart).
  static Duration terminalWarm = const Duration(seconds: 30);

  static void reset() {
    surfaces = FlutterWebSurface.new;
    store = const FlutterWebStore();
    server = _server;
    request = _request;
    visibility = _visibility;
    terminalWarm = const Duration(seconds: 30);
  }

  static PairedServer? _server() => sl.maybe<Connection>()?.server;

  static ValueListenable<bool>? _visibility() => sl.maybe<AppVisibility>();

  static Future<ApiResponse> _request(String method, String path,
      {Map<String, String>? query, Map<String, Object?>? body}) {
    final ComandosApi? api = sl.maybe<Connection>()?.api;
    if (api == null) return Future<ApiResponse>.error(StateError('unpaired'));
    return api.request(method, path, query: query, body: body);
  }
}

/// The dashboard session of the app's WebViews.
///
/// Auth: the token goes in as the `cc_token` cookie, set for the paired host
/// right before a page loads (the server accepts it on every request and on the
/// WebSocket upgrade: `presented_token` in comandos-core/src/dashboard_access.rs).
/// It is never put in a dashboard URL, never in localStorage, never logged. The
/// cookie lives only while some WebView of the app is alive: the last one to
/// go clears it. Exception: the terminal page itself needs `?auth=` (see
/// [terminalUrl]); that URL is never logged and its console output is dropped.
abstract final class WebSession {
  static int _users = 0;
  static Future<void>? _queue;
  static Future<void>? _clearing;
  static Connection? _watched;
  static String? _pairedHost;

  /// Called before a page loads. Returns false when the token cannot travel
  /// as a cookie (the panel then passes it once as `?token=`, as the web does).
  static Future<bool> acquire(PairedServer server) {
    _watch();
    _users++;
    final bool cookie = cookieSafeToken(server.token);
    return _serial(() async {
      // A wipe in progress (unpair, other desktop) finishes first.
      await _clearing;
      if (cookie) await WebPorts.store.setTokenCookie(server.host, server.token);
    }).then((_) => cookie);
  }

  /// Called when a WebView is gone. The last one clears the cookie.
  static Future<void> release() {
    if (_users > 0) _users--;
    if (_users > 0) return Future<void>.value();
    return _serial(() async {
      if (_users == 0) await WebPorts.store.clearCookies();
    });
  }

  static Future<void> _serial(Future<void> Function() job) {
    final Future<void> prev = _queue ?? Future<void>.value();
    final Future<void> next = prev.then((_) => job()).catchError((Object _) {});
    _queue = next;
    return next;
  }

  /// Unpair / a different desktop: drop the pages, then cookies and storage.
  static void _watch() {
    final Connection? c = sl.maybe<Connection>();
    if (c == null || identical(c, _watched)) return;
    _watched?.removeListener(_onConnection);
    _watched = c;
    _pairedHost = c.server?.host;
    c.addListener(_onConnection);
  }

  static void _onConnection() {
    final String? host = _watched?.server?.host;
    if (host == _pairedHost) return;
    _pairedHost = host;
    unawaited(clearWebData());
  }

  @visibleForTesting
  static int get users => _users;

  @visibleForTesting
  static void resetForTest() {
    _watched?.removeListener(_onConnection);
    _watched = null;
    _pairedHost = null;
    _users = 0;
    _queue = null;
    _clearing = null;
  }
}

/// Listeners run by [clearWebData] before the store is wiped (the kept-warm
/// terminal registers itself here).
final List<Future<void> Function()> webDataDroppers = <Future<void> Function()>[];

/// Forgets everything the app's WebViews stored: closes the kept-warm terminal,
/// removes the `cc_token` cookie, localStorage/IndexedDB and the HTTP cache.
///
/// Call it on unpair (Ajustes › Desvincular, right after `Connection.unpair()`).
/// It also runs by itself when the paired host changes while a web panel or the
/// terminal has been used in this run.
Future<void> clearWebData() {
  final Future<void> done = _wipe();
  WebSession._clearing = done;
  return done;
}

Future<void> _wipe() async {
  for (final Future<void> Function() drop in List<Future<void> Function()>.of(webDataDroppers)) {
    try {
      await drop();
    } on Object {
      // Keep wiping.
    }
  }
  try {
    await WebPorts.store.clearAll();
  } on Object {
    // No WebView support on this device: nothing was stored either.
  }
}
