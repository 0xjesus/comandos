// Pure Dart: the few lines of JavaScript the app runs in the dashboard pages
// after they load. They never contain the token and talk back only through the
// `ComandosApp` channel with tiny JSON messages ({t: open|closed|fail|ready|term}).

import 'dart:convert';

import 'web_urls.dart';

/// Name of the one JavaScript channel the app adds to its WebViews.
const String kAppChannel = 'ComandosApp';

/// How long a page may take to show its panel or terminal before the app shows
/// the error state with «Reintentar».
const Duration kPageReadyTimeout = Duration(seconds: 15);

const String _send = 'function send(o){try{$kAppChannel.postMessage(JSON.stringify(o));}catch(e){}}';

/// After a panel page loads: open the panel the way the desktop app does in its
/// centre window (`openCenterPanel`), say when it is on screen ({t:'open'}) and
/// when the person closed it with the page's own ✕ ({t:'closed'}).
String panelScript(WebPanel panel) {
  final String sel;
  final String shown;
  String open = '';
  String style = '';
  switch (panel) {
    case WebPanel.settings:
    case WebPanel.servers:
    case WebPanel.remote:
      final String name = onlyPanelName(panel)!;
      sel = '#$name';
      shown = 'el.classList.contains("open")';
      final String request = name == 'settings' ? '{panel:"settings",tab:""}' : '{panel:"$name"}';
      open = 'if(typeof openCenterPanel==="function"&&window.CENTER_PANELS){called=true;openCenterPanel($request);}';
    case WebPanel.news:
      // `?panel=news&news=` opens the latest summary by itself.
      sel = '#news-reader';
      shown = '!el.hidden&&el.isConnected';
      open = 'called=true;';
    case WebPanel.snippets:
      sel = '#dlg-snippets';
      shown = 'el.open';
      open = 'if(typeof openSnippets==="function"&&window.snipDlg){called=true;openSnippets();}';
      // only-panel hides every other body child; the snippets dialog fills the
      // view like the centre panels do (index_page.css "Modales en medio").
      style = 'var s=document.createElement("style");s.textContent='
          '${jsonEncode('html[data-only-panel="snippets"] body.only-panel>dialog#dlg-snippets[open]'
              '{display:block!important;width:100vw!important;height:100%!important;max-width:none!important;'
              'max-height:none!important;margin:0!important;border:0!important;border-radius:0!important;'
              'box-shadow:none!important}'
              'html[data-only-panel="snippets"] body.only-panel>dialog#dlg-snippets::backdrop{background:none}')}'
          ';document.head.appendChild(s);';
    case WebPanel.fullPage:
      return '(function(){$_send send({t:"open"});})();';
  }
  final int limit = kPageReadyTimeout.inMilliseconds;
  return '(function(){if(window.__cmdPanel)return;window.__cmdPanel=1;$_send'
      '$style'
      'var called=false,opened=false,t0=Date.now();'
      'var timer=setInterval(function(){try{'
      'var el=document.querySelector(${jsonEncode(sel)});'
      'if(!called&&el){$open}'
      'if(el&&($shown)){if(!opened){opened=true;send({t:"open"});}}'
      'else if(opened){clearInterval(timer);send({t:"closed"});}'
      'else if(Date.now()-t0>$limit){clearInterval(timer);send({t:"fail"});}'
      '}catch(e){clearInterval(timer);send({t:"fail"});}},250);})();';
}

/// After the terminal page loads: forward what it says to a parent
/// (`source: "comandos-term"`; top-level, its parent is itself) and report
/// `ready` once its terminal is live (`window.__comandosTerm`, published right
/// after its message listener is installed).
String terminalScript() {
  final int limit = kPageReadyTimeout.inMilliseconds;
  return '(function(){if(window.__cmdTerm)return;window.__cmdTerm=1;$_send'
      'window.addEventListener("message",function(e){'
      'if(e.origin!==location.origin)return;var d=e.data;'
      'if(typeof d==="string"){try{d=JSON.parse(d);}catch(_){return;}}'
      'if(!d||d.source!=="comandos-term"||typeof d.type!=="string")return;'
      'send({t:"term",type:d.type,selecting:!!d.selecting,pane:typeof d.pane==="string"?d.pane:""});});'
      'var t0=Date.now();(function wait(){'
      'if(window.__comandosTerm){send({t:"ready"});return;}'
      'if(Date.now()-t0>$limit){send({t:"fail"});return;}'
      'setTimeout(wait,100);})();})();';
}

/// The pane header reads its title from the dashboard (`parent.openTerms`);
/// top-level, the page is its own parent, so the app provides that map.
String tabLabelScript(String session, String? label) {
  final String name = (label == null || label.trim().isEmpty) ? session : label.trim();
  return 'try{window.openTerms=new Map([[${jsonEncode(session)},{label:${jsonEncode(name)}}]]);}catch(e){}';
}

/// Posts [message] to the page as its parent would (`origin` = its own).
String postToPage(Map<String, Object?> message) =>
    'window.postMessage(${jsonEncode(message)},location.origin);';

/// `{source:"comandos", type:"dock", on:true}`: the app draws the keys and the
/// composer (chat_view.rs `DOCK`), the page hides its own.
const Map<String, Object?> kDockMessage = <String, Object?>{'source': 'comandos', 'type': 'dock', 'on': true};

Map<String, Object?> selectPaneMessage(String pane) =>
    <String, Object?>{'source': 'comandos', 'type': 'select-pane', 'pane': pane};

Map<String, Object?> toolbarMessage(Map<String, Object?> term) =>
    <String, Object?>{'source': 'comandos', 'type': 'toolbar', 'term': term};

Map<String, Object?> interactionMessage({required bool known, required bool busy, required bool selecting}) =>
    <String, Object?>{
      'source': 'comandos',
      'type': 'interaction-state',
      'known': known,
      'busy': busy,
      'selecting': selecting,
    };

/// A message from [kAppChannel], parsed and validated. Anything else is null.
class PageMessage {
  const PageMessage(this.t, {this.type = '', this.selecting = false, this.pane = ''});

  final String t;
  final String type;
  final bool selecting;
  final String pane;

  static const Set<String> _kinds = <String>{'open', 'closed', 'fail', 'ready', 'term'};

  static PageMessage? parse(String raw) {
    if (raw.length > 2048) return null;
    Object? v;
    try {
      v = jsonDecode(raw);
    } on FormatException {
      return null;
    }
    if (v is! Map) return null;
    final Object? t = v['t'];
    if (t is! String || !_kinds.contains(t)) return null;
    final Object? type = v['type'];
    final Object? pane = v['pane'];
    return PageMessage(
      t,
      type: type is String ? type : '',
      selecting: v['selecting'] == true,
      pane: pane is String && validPaneId(pane) ? pane : '',
    );
  }
}
