// Pure Dart: which dashboard page each web panel loads, the terminal page URL,
// the navigation allow-list and the terminal look. No Flutter imports, unit
// tested in test/features/webpanel_test.dart.
//
// Sources (crates/…):
// - only-panel mode: comandos-web/src/components/foundation_lifecycle.rs
//   (`?panel=` → ONLY_PANEL, `html[data-only-panel]`, `body.only-panel`),
//   comandos-web-view/src/index_page.css (`body.only-panel …` rules) and
//   foundation_system.rs `openCenterPanel` (what the desktop app calls in its
//   centre window, comandos-app/src/ui/app_t18_views.rs).
// - news: comandos-web-content/src/news_reader.rs (`?panel=news&news=` opens the
//   latest summary by itself; comandos-app/src/ui/reader.rs builds the same URL).
// - terminal: comandos-web/src/components/app_coordinator_terminal.rs
//   `ensure_frame` → `{origin}/term/?auth=<token>&arg=<session>&theme=<curTheme>`;
//   comandos-term-web/src/page.rs reads `arg`, `auth`, `theme`, `btn`.

import '../../core/config/paired_server.dart';

/// The rarely used dashboard pages the app shows as they are on the web.
enum WebPanel { settings, servers, snippets, news, remote, fullPage }

/// Title of the thin bar over each panel (the web's own labels).
String webPanelTitle(WebPanel panel) => switch (panel) {
      WebPanel.settings => 'Ajustes',
      WebPanel.servers => 'Servidores',
      WebPanel.snippets => 'Snippets',
      WebPanel.news => 'Resúmenes',
      WebPanel.remote => 'Remoto',
      WebPanel.fullPage => 'ComandOS',
    };

/// The `?panel=` value the dashboard reads (lowercase ASCII only, like
/// `only_panel()`), or null for the whole dashboard.
String? onlyPanelName(WebPanel panel) => switch (panel) {
      WebPanel.settings => 'settings',
      WebPanel.servers => 'servers',
      WebPanel.remote => 'remote',
      WebPanel.news => 'news',
      // No only-panel CSS for it: the page hides the dialog, so the app adds one
      // style rule (see panelScript). Still far lighter than the full page.
      WebPanel.snippets => 'snippets',
      WebPanel.fullPage => null,
    };

/// The dashboard origin, `https://host[:port]`.
Uri dashboardOrigin(PairedServer server) => Uri(scheme: 'https', host: server.host, port: server.port);

/// The page a panel loads. [tokenInUrl] is only for a token that cannot travel
/// as a cookie (see [cookieSafeToken]); the page then stores it and strips it
/// from the address bar itself (`consume_token` in foundation.rs).
Uri webPanelUrl(PairedServer server, WebPanel panel, {bool tokenInUrl = false}) {
  final List<String> query = <String>[];
  final String? only = onlyPanelName(panel);
  if (only != null) query.add('panel=$only');
  // Empty `news` = "open the latest" (news_reader.rs `install`).
  if (panel == WebPanel.news) query.add('news=');
  if (tokenInUrl) query.add('token=${Uri.encodeQueryComponent(server.token)}');
  return Uri(
    scheme: 'https',
    host: server.host,
    port: server.port,
    path: '/',
    query: query.isEmpty ? null : query.join('&'),
  );
}

/// The live terminal page for [session], exactly as the dashboard embeds it.
///
/// The terminal page needs the token as `auth`: it authenticates its WebSocket
/// with it and sends it as `X-Comandos-Token` for `/terminal-panes`, `/prefs`
/// and `/terminal-link` (pane chips, the one-pane crop of a split, links).
/// Never log this URL; use [redactedUrl].
Uri terminalUrl(PairedServer server, String session, {TermLook look = TermLook.fallback}) => Uri(
      scheme: 'https',
      host: server.host,
      port: server.port,
      path: '/term/',
      queryParameters: <String, String>{
        'auth': server.token,
        'arg': session,
        'theme': look.theme,
        'btn': look.buttons,
      },
    );

/// [url] with every secret query value replaced by `***` (for messages/tests).
String redactedUrl(Uri url) {
  if (!url.hasQuery) return url.toString();
  final String query = url.query.split('&').map((String part) {
    final int eq = part.indexOf('=');
    final String key = Uri.decodeQueryComponent(eq < 0 ? part : part.substring(0, eq));
    return _secretKeys.contains(key.toLowerCase()) ? '${eq < 0 ? part : part.substring(0, eq)}=***' : part;
  }).join('&');
  return url.replace(query: query).toString();
}

const Set<String> _secretKeys = <String>{'auth', 'token', 'access_token'};

/// A token the server reads verbatim from `Cookie: cc_token=` (it does not
/// percent-decode cookies) and the WebView cookie API does not alter.
/// `secrets.token_urlsafe` tokens always are.
bool cookieSafeToken(String token) => RegExp(r'^[A-Za-z0-9._~-]{1,512}$').hasMatch(token);

/// Whether the WebView may go to [url]: only https on the paired host and port.
/// Subframes may also be `about:blank` / `about:srcdoc` (empty frames).
bool allowedNavigation(Uri url, PairedServer server, {bool mainFrame = true}) {
  if (!mainFrame && url.scheme == 'about' && (url.path == 'blank' || url.path == 'srcdoc')) {
    return true;
  }
  if (url.scheme != 'https' || url.userInfo.isNotEmpty) return false;
  if (url.host.toLowerCase() != server.host.toLowerCase()) return false;
  return url.port == (server.port ?? 443);
}

/// Built-in themes of the terminal page (comandos-term-web/src/page_theme.rs).
const List<String> kTermThemes = <String>[
  'noche', 'dia', 'calido', 'termius', 'bruno', 'superglass', 'neon', 'contraste', 'ubuntu',
];

/// Button styles the terminal accepts (`parent::button_style`).
const List<String> kButtonStyles = <String>['sutil', 'arcade', 'tecla', 'pixel', 'consola'];

const List<String> _ansi = <String>[
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow',
  'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite',
];

/// The terminal's theme as the dashboard hands it to its terminal frames:
/// `theme=` (`curTheme`, always a built-in base), the button style it inherits
/// from the dashboard root, and for a user theme the palette message
/// (theme_preferences.rs `broadcast`, user_themes.rs `term_palette`).
class TermLook {
  const TermLook({required this.theme, required this.buttons, this.palette});

  static const TermLook fallback = TermLook(theme: 'noche', buttons: 'sutil');

  final String theme;
  final String buttons;
  final Map<String, String>? palette;

  /// From `GET /prefs` (`theme`, `button_style`, `user_themes`).
  factory TermLook.fromPrefs(Map<String, Object?> prefs) {
    final Object? rawTheme = prefs['theme'];
    final String id = rawTheme is String ? rawTheme : '';
    final Object? rawButtons = prefs['button_style'];
    String buttons = rawButtons is String && kButtonStyles.contains(rawButtons) ? rawButtons : 'sutil';
    final Object? users = prefs['user_themes'];
    if (users is List) {
      for (final Object? t in users) {
        if (t is! Map || t['id'] != id || id.isEmpty) continue;
        final Object? base = t['base'];
        final String theme = base is String && kTermThemes.contains(base) ? base : 'noche';
        final Object? style = t['button_style'];
        if (style is String && kButtonStyles.contains(style)) buttons = style;
        final Object? colors = t['colors'];
        return TermLook(
          theme: theme,
          buttons: buttons,
          palette: colors is Map ? termPalette(colors.cast<Object?, Object?>()) : null,
        );
      }
    }
    final String lower = id.toLowerCase();
    return TermLook(theme: kTermThemes.contains(lower) ? lower : 'noche', buttons: buttons);
  }

  /// The `{source, type: theme}` message the terminal page applies.
  Map<String, Object?> get themeMessage => <String, Object?>{
        'source': 'comandos',
        'type': 'theme',
        'theme': theme,
        if (palette != null) 'palette': palette,
      };

  @override
  bool operator ==(Object other) =>
      other is TermLook &&
      other.theme == theme &&
      other.buttons == buttons &&
      _sameMap(other.palette, palette);

  @override
  int get hashCode => Object.hash(theme, buttons, palette?.length);
}

bool _sameMap(Map<String, String>? a, Map<String, String>? b) {
  if (a == null || b == null) return a == b;
  if (a.length != b.length) return false;
  for (final MapEntry<String, String> e in a.entries) {
    if (b[e.key] != e.value) return false;
  }
  return true;
}

/// `user_themes.rs term_palette`. The page rejects the whole message when any
/// colour is not `#RRGGBB`; then it keeps the base theme, so this returns null.
Map<String, String>? termPalette(Map<Object?, Object?> c) {
  final Map<String, Object?> raw = <String, Object?>{
    'background': c['bg'],
    'foreground': c['fg'],
    'cursor': c['cursor'],
    'cursorAccent': c['bg'],
    'selectionBackground': c['line2'],
    'panel': c['panel'],
    'panel2': c['panel2'],
    'line': c['line'],
    'line2': c['line2'],
    'dim': c['dim'],
    'faint': c['faint'],
    'brand': c['brand'],
    for (int i = 0; i < _ansi.length; i++) _ansi[i]: c['pal$i'],
  };
  final RegExp hex = RegExp(r'^#[0-9A-Fa-f]{6}$');
  final Map<String, String> out = <String, String>{};
  for (final MapEntry<String, Object?> e in raw.entries) {
    final Object? v = e.value;
    if (v is! String || !hex.hasMatch(v)) return null;
    out[e.key] = v;
  }
  return out;
}

/// A tmux pane id as the terminal page accepts it (`%` + digits).
bool validPaneId(String? pane) => pane != null && RegExp(r'^%[0-9]+$').hasMatch(pane);

/// Toolbar actions the terminal page accepts from its parent (`parent::toolbar_action`).
const Set<String> kTermActions = <String>{'mode', 'panes', 'history', 'copy', 'paste', 'keyboard', 'ctrl'};

/// Toolbar keys the terminal page accepts (`parent::toolbar_key`).
const Set<String> kTermKeys = <String>{
  'escape', 'ctrlc', 'yes', 'no', 'backspace', 'tab', 'up', 'down', 'right', 'left',
};
