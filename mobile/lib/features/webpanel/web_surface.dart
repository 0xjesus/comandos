import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:webview_flutter/webview_flutter.dart';

import '../../ui/theme.dart';
import 'web_scripts.dart';

/// What a [WebSurface] reports back. Callbacks only; never carries the URL.
class WebSurfaceEvents {
  const WebSurfaceEvents({
    required this.allow,
    required this.onPageFinished,
    required this.onMainFrameError,
    required this.onMessage,
  });

  /// Navigation policy (called for page-initiated navigations).
  final bool Function(Uri url, {required bool mainFrame}) allow;

  /// The main document finished loading.
  final VoidCallback onPageFinished;

  /// The main document failed (network, TLS, HTTP ≥ 400 on the page itself).
  final VoidCallback onMainFrameError;

  /// A raw message on the [kAppChannel] channel (validate before use).
  final void Function(String message) onMessage;
}

/// The small slice of a WebView the panels and the terminal use. The real one
/// is [FlutterWebSurface]; widget tests use a fake (WebViews cannot run in
/// `flutter test`).
abstract class WebSurface {
  Future<void> load(Uri url);
  Future<void> runJs(String source);

  /// The platform view. Build it in at most one place at a time.
  Widget view();

  /// Stops the page (sockets, timers) and releases the WebView.
  Future<void> dispose();
}

typedef WebSurfaceFactory = WebSurface Function(WebSurfaceEvents events);

/// Cookie and storage access of the app's WebViews.
abstract class WebStore {
  /// Sets the dashboard session cookie `cc_token` for https://[host].
  Future<void> setTokenCookie(String host, String token);

  /// Removes every WebView cookie (the app sets only `cc_token`).
  Future<void> clearCookies();

  /// Cookies, localStorage/IndexedDB and HTTP cache of every page.
  Future<void> clearAll();
}

/// [WebSurface] over `webview_flutter`.
///
/// - JavaScript on (the pages are WASM apps), one channel [kAppChannel].
/// - Navigation restricted by [WebSurfaceEvents.allow]: anything else opens
///   nowhere (no external browser, no new windows).
/// - Console messages are swallowed: Android would otherwise copy them to
///   logcat together with the page URL (the terminal URL carries the token).
/// - Permission requests (camera, microphone…) are denied.
/// - No file access (Android default for targetSdk ≥ 30), no mixed content
///   (Android default MIXED_CONTENT_NEVER_ALLOW), no zoom.
class FlutterWebSurface implements WebSurface {
  FlutterWebSurface(this._events)
      : _controller = WebViewController(
          onPermissionRequest: (WebViewPermissionRequest r) => r.deny(),
        ) {
    _setup = _configure();
  }

  final WebSurfaceEvents _events;
  final WebViewController _controller;
  late final Future<void> _setup;
  Uri? _current;
  bool _disposed = false;

  Future<void> _configure() async {
    final WebViewController c = _controller;
    await c.setJavaScriptMode(JavaScriptMode.unrestricted);
    await c.setBackgroundColor(Orbita.bg);
    await c.enableZoom(false);
    await c.setOnConsoleMessage((JavaScriptConsoleMessage _) {});
    await c.addJavaScriptChannel(kAppChannel, onMessageReceived: (JavaScriptMessage m) {
      if (!_disposed) _events.onMessage(m.message);
    });
    await c.setNavigationDelegate(NavigationDelegate(
      onNavigationRequest: (NavigationRequest r) {
        final Uri? url = Uri.tryParse(r.url);
        final bool ok = url != null && _events.allow(url, mainFrame: r.isMainFrame);
        return ok ? NavigationDecision.navigate : NavigationDecision.prevent;
      },
      onPageFinished: (String _) {
        if (!_disposed) _events.onPageFinished();
      },
      onWebResourceError: (WebResourceError e) {
        if (!_disposed && (e.isForMainFrame ?? true)) _events.onMainFrameError();
      },
      onHttpError: (HttpResponseError e) {
        final Uri? failed = e.request?.uri;
        final int status = e.response?.statusCode ?? 0;
        if (_disposed || status < 400 || failed == null || _current == null) return;
        // Only the document itself; a failing image or API call is the page's business.
        if (failed.path == _current!.path && failed.query == _current!.query) {
          _events.onMainFrameError();
        }
      },
      onSslAuthError: (SslAuthError e) => e.cancel(),
      onHttpAuthRequest: (HttpAuthRequest r) => r.onCancel(),
    ));
  }

  @override
  Future<void> load(Uri url) async {
    await _setup;
    if (_disposed) return;
    _current = url;
    await _controller.loadRequest(url);
  }

  @override
  Future<void> runJs(String source) async {
    await _setup;
    if (_disposed) return;
    try {
      await _controller.runJavaScript(source);
    } on Object {
      // The page navigated away or is not ready; the scripts retry by themselves.
    }
  }

  @override
  Widget view() => WebViewWidget(
        controller: _controller,
        // The page owns every touch (terminal scroll, selection, long-press):
        // no ancestor scrollable may steal a drag.
        gestureRecognizers: <Factory<OneSequenceGestureRecognizer>>{
          Factory<OneSequenceGestureRecognizer>(EagerGestureRecognizer.new),
        },
      );

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _current = null;
    try {
      await _setup;
      // Ends the page now (WebSocket, timers, polling); the native WebView is
      // released once this Dart object is collected.
      await _controller.loadHtmlString('<!doctype html><title></title>');
      await _controller.removeJavaScriptChannel(kAppChannel);
    } on Object {
      // Already gone.
    }
  }
}

/// [WebStore] over `webview_flutter`.
class FlutterWebStore implements WebStore {
  const FlutterWebStore();

  @override
  Future<void> setTokenCookie(String host, String token) => WebViewCookieManager().setCookie(
        WebViewCookie(name: 'cc_token', value: token, domain: 'https://$host', path: '/'),
      );

  @override
  Future<void> clearCookies() async {
    await WebViewCookieManager().clearCookies();
  }

  @override
  Future<void> clearAll() async {
    await clearCookies();
    // Storage and cache are per WebView profile; a throwaway controller reaches
    // them (it is only created here, at unpair).
    final WebViewController c = WebViewController();
    try {
      await c.clearLocalStorage();
      await c.clearCache();
    } on Object {
      // Nothing stored.
    }
  }
}
