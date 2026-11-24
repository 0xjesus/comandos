import 'dart:async';
import 'dart:convert';
import 'dart:typed_data' show BytesBuilder;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:webview_flutter/webview_flutter.dart';

import '../../ui/theme.dart';

/// What an internet address can be drawn as in the chat
/// (`comandos_core::chat_markdown::Media`).
enum MediaKind { image, video, pdf }

/// The media an http(s) address points to, by its extension (query and
/// fragment aside), as the web and the desktop decide it. Files on the computer
/// are never drawn: only internet addresses.
MediaKind? mediaOf(String url) {
  final String lower = url.trim().toLowerCase();
  if (!(lower.startsWith('https://') || lower.startsWith('http://'))) return null;
  final String path = lower.split(RegExp(r'[?#]')).first;
  final int dot = path.lastIndexOf('.');
  final String ext = dot < 0 ? '' : path.substring(dot + 1);
  return switch (ext) {
    'png' || 'jpg' || 'jpeg' || 'gif' || 'webp' || 'avif' || 'svg' || 'bmp' => MediaKind.image,
    'mp4' || 'webm' || 'mov' || 'm4v' || 'ogv' => MediaKind.video,
    'pdf' => MediaKind.pdf,
    _ => null,
  };
}

/// Media found in a block of text: what it is, where, and its alt/link text.
class MediaRef {
  const MediaRef(this.kind, this.url, this.alt);
  final MediaKind kind;
  final String url;
  final String alt;
}

String _fileName(String url) {
  final String last = url.split(RegExp(r'[?#]')).first.split('/').last;
  return last.isEmpty ? url : Uri.decodeComponent(last.replaceAll('+', ' '));
}

/// Pinned libraries (cdnjs, exact version, SRI), the same as `chat_media.js`.
const String _cdn = 'https://cdnjs.cloudflare.com/ajax/libs/';
const (String, String) _mermaid = (
  '${_cdn}mermaid/10.9.1/mermaid.min.js',
  'sha384-WmdflGW9aGfoBdHc4rRyWzYuAjEmDwMdGdiPNacbwfGKxBW/SO6guzuQ76qjnSlr',
);
const (String, String) _pdf = (
  '${_cdn}pdf.js/3.11.174/pdf.min.js',
  'sha384-/1qUCSGwTur9vjf/z9lmu/eCUYbpOTgSjmpbMQZ1/CtX2v/WcAIKqRv+U1DUCG6e',
);
const (String, String) _pdfWorker = (
  '${_cdn}pdf.js/3.11.174/pdf.worker.min.js',
  'sha384-SnzOobpRMLXZ52iJvZm/C0fYw0OQemTXzTjIsdsfMcrCtCEe9qgzxTd3RSklO5x2',
);

/// The largest PDF the viewer downloads.
const int kPdfLimit = 25 * 1024 * 1024;

String _attr(String s) =>
    s.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

/// JSON for a `<script type="application/json">`: no `</script>` can close it.
String _json(Object v) => jsonEncode(v).replaceAll('<', r'\u003c');

String _script((String, String) lib) =>
    '<script src="${lib.$1}" integrity="${lib.$2}" crossorigin="anonymous" referrerpolicy="no-referrer"></script>';

const String _style = '''
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=6,user-scalable=yes">
<style>
html,body{margin:0;background:#0A0D13;color:#EAF0FB;font:14px/1.5 system-ui,sans-serif}
#out{padding:12px;display:flex;flex-direction:column;align-items:center;gap:12px}
#out svg{max-width:100%;height:auto}
canvas{max-width:100%;height:auto;background:#fff;border-radius:6px}
video{width:100%;max-height:92vh;background:#000}
.msg{color:#9AA6BF;padding:24px;text-align:center}
</style>''';

/// The page a viewer shows (self-contained; only the pinned libraries load).
@visibleForTesting
String viewerHtml(MediaKind? kind, {String url = '', String mermaid = ''}) {
  if (kind == MediaKind.image) {
    return '<!doctype html><html><head>$_style</head><body><div id="out">'
        '<img src="${_attr(url)}" referrerpolicy="no-referrer" style="max-width:100%;height:auto"></div></body></html>';
  }
  if (kind == MediaKind.video) {
    return '<!doctype html><html><head>$_style</head><body><div id="out">'
        '<video src="${_attr(url)}" controls playsinline preload="metadata"></video></div></body></html>';
  }
  if (kind == MediaKind.pdf) {
    return '<!doctype html><html><head>$_style${_script(_pdf)}</head><body><div id="out"><div class="msg">Abriendo PDF…</div></div>'
        '<script>'
        'var W=${_json(<String>[_pdfWorker.$1, _pdfWorker.$2])};'
        'function fail(m){document.getElementById("out").innerHTML="";var d=document.createElement("div");d.className="msg";d.textContent=m;document.getElementById("out").appendChild(d);}'
        'window.showPdf=function(b64){'
        'var raw=atob(b64),bytes=new Uint8Array(raw.length);for(var i=0;i<raw.length;i++)bytes[i]=raw.charCodeAt(i);'
        'fetch(W[0],{integrity:W[1],mode:"cors",referrerPolicy:"no-referrer"}).then(function(r){if(!r.ok)throw 0;return r.text();})'
        '.then(function(code){pdfjsLib.GlobalWorkerOptions.workerSrc=URL.createObjectURL(new Blob([code],{type:"text/javascript"}));'
        'return pdfjsLib.getDocument({data:bytes,isEvalSupported:false}).promise;})'
        '.then(function(doc){var out=document.getElementById("out");out.innerHTML="";var w=Math.min(out.clientWidth-24,900)*(window.devicePixelRatio||1);'
        'var next=1;function page(){if(next>doc.numPages)return;var n=next++;doc.getPage(n).then(function(p){'
        'var s=Math.min(3,w/p.getViewport({scale:1}).width),vp=p.getViewport({scale:s}),c=document.createElement("canvas");'
        'c.width=Math.round(vp.width);c.height=Math.round(vp.height);out.appendChild(c);'
        'p.render({canvasContext:c.getContext("2d"),viewport:vp}).promise.then(page);});}page();})'
        '.catch(function(){fail("No pude abrir este PDF.");});};'
        'window.failPdf=fail;'
        '</script></body></html>';
  }
  return '<!doctype html><html><head>$_style${_script(_mermaid)}</head><body><div id="out"><div class="msg">Dibujando…</div></div>'
      '<script type="application/json" id="src">${_json(mermaid)}</script>'
      '<script>'
      'try{mermaid.initialize({startOnLoad:false,securityLevel:"strict",theme:"dark"});'
      'mermaid.render("d1",JSON.parse(document.getElementById("src").textContent)).then(function(r){document.getElementById("out").innerHTML=r.svg;})'
      '.catch(function(){document.getElementById("out").innerHTML="<div class=msg>Este diagrama tiene un error; el código está en el chat.</div>";});'
      '}catch(e){document.getElementById("out").innerHTML="<div class=msg>Sin conexión para dibujar el diagrama.</div>";}'
      '</script></body></html>';
}

/// Full screen: a video, a PDF (every page) or a Mermaid diagram, drawn by a
/// WebView that only loads the pinned libraries; it navigates nowhere, asks for
/// no permission and never opens another app.
class MediaViewerPage extends StatefulWidget {
  const MediaViewerPage({super.key, required this.title, this.kind, this.url = '', this.mermaid = ''});

  final String title;

  /// Null: a Mermaid diagram ([mermaid] is its code).
  final MediaKind? kind;
  final String url;
  final String mermaid;

  static Future<void> open(BuildContext context, {required String title, MediaKind? kind, String url = '', String mermaid = ''}) =>
      Navigator.of(context).push(MaterialPageRoute<void>(
        builder: (_) => MediaViewerPage(title: title, kind: kind, url: url, mermaid: mermaid),
      ));

  @override
  State<MediaViewerPage> createState() => _MediaViewerPageState();
}

class _MediaViewerPageState extends State<MediaViewerPage> {
  late final WebViewController _web = WebViewController(onPermissionRequest: (WebViewPermissionRequest r) => r.deny());
  bool _loaded = false;

  @override
  void initState() {
    super.initState();
    unawaited(_start());
  }

  Future<void> _start() async {
    final WebViewController c = _web;
    await c.setJavaScriptMode(JavaScriptMode.unrestricted);
    await c.setBackgroundColor(Orbita.bg);
    await c.enableZoom(true);
    await c.setOnConsoleMessage((JavaScriptConsoleMessage _) {});
    await c.setNavigationDelegate(NavigationDelegate(
      // The page itself only; links inside a PDF or a diagram open nothing.
      onNavigationRequest: (NavigationRequest r) =>
          r.url == 'about:blank' || r.url.startsWith('data:') ? NavigationDecision.navigate : NavigationDecision.prevent,
      onPageFinished: (String _) {
        if (!_loaded) {
          _loaded = true;
          if (widget.kind == MediaKind.pdf) unawaited(_sendPdf());
        }
      },
      onSslAuthError: (SslAuthError e) => e.cancel(),
      onHttpAuthRequest: (HttpAuthRequest r) => r.onCancel(),
    ));
    await c.loadHtmlString(viewerHtml(widget.kind, url: widget.url, mermaid: widget.mermaid));
  }

  /// The phone downloads the PDF (no CORS to worry about) and hands it over.
  Future<void> _sendPdf() async {
    String? why;
    try {
      final http.Client client = http.Client();
      try {
        final http.StreamedResponse r =
            await client.send(http.Request('GET', Uri.parse(widget.url))).timeout(const Duration(seconds: 30));
        if (r.statusCode != 200) {
          why = 'El servidor respondió ${r.statusCode}.';
        } else if ((r.contentLength ?? 0) > kPdfLimit) {
          why = 'El PDF pesa más de 25 MB.';
        } else {
          final BytesBuilder bytes = BytesBuilder(copy: false);
          await for (final List<int> chunk in r.stream.timeout(const Duration(seconds: 60))) {
            bytes.add(chunk);
            if (bytes.length > kPdfLimit) {
              why = 'El PDF pesa más de 25 MB.';
              break;
            }
          }
          if (why == null && mounted) {
            await _web.runJavaScript('showPdf("${base64Encode(bytes.takeBytes())}")');
          }
        }
      } finally {
        client.close();
      }
    } on Object {
      why = 'No pude descargar el PDF.';
    }
    if (why != null && mounted) await _web.runJavaScript('failPdf(${jsonEncode(why)})');
  }

  @override
  void dispose() {
    unawaited(_web.loadHtmlString('<!doctype html><title></title>'));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
        backgroundColor: Orbita.bg,
        appBar: AppBar(
          backgroundColor: Orbita.bar,
          foregroundColor: Orbita.text,
          title: Text(widget.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 15)),
        ),
        body: WebViewWidget(
          controller: _web,
          gestureRecognizers: <Factory<OneSequenceGestureRecognizer>>{
            Factory<OneSequenceGestureRecognizer>(EagerGestureRecognizer.new),
          },
        ),
      );
}

/// Full screen image with pinch to zoom.
class ImageViewerPage extends StatelessWidget {
  const ImageViewerPage({super.key, required this.url, required this.title});
  final String url;
  final String title;

  @override
  Widget build(BuildContext context) => Scaffold(
        backgroundColor: Colors.black,
        appBar: AppBar(
          backgroundColor: Colors.black,
          foregroundColor: Colors.white,
          title: Text(title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 15)),
        ),
        body: InteractiveViewer(
          maxScale: 8,
          child: Center(
            child: Image.network(url, fit: BoxFit.contain, errorBuilder: (_, _, _) => const _Broken('No se pudo cargar la imagen.')),
          ),
        ),
      );
}

class _Broken extends StatelessWidget {
  const _Broken(this.text);
  final String text;
  @override
  Widget build(BuildContext context) =>
      Padding(padding: const EdgeInsets.all(12), child: Text(text, style: const TextStyle(color: Orbita.dim, fontSize: 13)));
}

/// One piece of media under the text that names it: internet images (GIFs
/// move) drawn in place and decoded at the size they are shown; videos, PDFs
/// and SVGs as a card that opens them full screen.
class MediaPreview extends StatelessWidget {
  const MediaPreview(this.media, {super.key});
  final MediaRef media;

  @override
  Widget build(BuildContext context) {
    final MediaRef m = media;
    final String name = m.alt.isNotEmpty && m.alt != m.url ? m.alt : _fileName(m.url);
    if (m.kind == MediaKind.image && !m.url.toLowerCase().split(RegExp(r'[?#]')).first.endsWith('.svg')) {
      return _NetImage(url: m.url, name: name);
    }
    final (IconData icon, String label, String action) = switch (m.kind) {
      MediaKind.video => (Icons.play_circle_fill_rounded, 'Video', 'Reproducir'),
      MediaKind.pdf => (Icons.picture_as_pdf_rounded, 'PDF', 'Abrir'),
      MediaKind.image => (Icons.image_rounded, 'Imagen SVG', 'Ver'),
    };
    return _MediaCard(
      icon: icon,
      kind: label,
      name: name,
      action: action,
      onTap: () => MediaViewerPage.open(
        context,
        title: name,
        kind: m.kind,
        url: m.url,
      ),
    );
  }
}

class _NetImage extends StatelessWidget {
  const _NetImage({required this.url, required this.name});
  final String url;
  final String name;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (BuildContext context, BoxConstraints c) {
      final double w = c.maxWidth.isFinite ? c.maxWidth : 320;
      final double dpr = MediaQuery.devicePixelRatioOf(context);
      return Semantics(
        image: true,
        label: name,
        button: true,
        child: GestureDetector(
          onTap: () => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => ImageViewerPage(url: url, title: name))),
          child: Align(
            alignment: Alignment.centerLeft,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 360),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(12),
                child: Image.network(
                  url,
                  // Decoded at the size it is drawn, never larger than the file.
                  cacheWidth: (w * dpr).round(),
                  fit: BoxFit.contain,
                  gaplessPlayback: true,
                  semanticLabel: name,
                  loadingBuilder: (BuildContext _, Widget child, ImageChunkEvent? p) => p == null
                      ? child
                      : Container(
                          width: w,
                          height: 140,
                          color: Orbita.panel2,
                          alignment: Alignment.center,
                          child: const SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2, color: Orbita.faint)),
                        ),
                  errorBuilder: (_, _, _) => _MediaCard(
                    icon: Icons.broken_image_outlined,
                    kind: 'Imagen',
                    name: 'No se pudo cargar · $name',
                    action: '',
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    });
  }
}

class _MediaCard extends StatelessWidget {
  const _MediaCard({required this.icon, required this.kind, required this.name, required this.action, this.onTap});
  final IconData icon;
  final String kind;
  final String name;
  final String action;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Orbita.panel2,
      borderRadius: BorderRadius.circular(12),
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
          decoration: BoxDecoration(borderRadius: BorderRadius.circular(12), border: Border.all(color: Orbita.line)),
          child: Row(
            children: <Widget>[
              Icon(icon, size: 30, color: onTap == null ? Orbita.faint : Orbita.brand),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: <Widget>[
                    Text(kind, style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Orbita.faint, letterSpacing: .3)),
                    Text(name, maxLines: 2, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 14, color: Orbita.text)),
                  ],
                ),
              ),
              if (action.isNotEmpty) ...<Widget>[
                const SizedBox(width: 8),
                Text(action, style: const TextStyle(fontSize: 13.5, fontWeight: FontWeight.w600, color: Orbita.brand)),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// A Mermaid block: a card that opens the drawn diagram full screen (zoom and
/// pan); the code stays under it.
class MermaidCard extends StatelessWidget {
  const MermaidCard(this.code, {super.key});
  final String code;

  @override
  Widget build(BuildContext context) {
    final String first = code.trimLeft().split('\n').first.trim();
    final String type = first.split(RegExp(r'\s+')).first;
    return _MediaCard(
      icon: Icons.account_tree_rounded,
      kind: 'Diagrama Mermaid',
      name: type.isEmpty ? 'diagrama' : type,
      action: 'Ver diagrama',
      onTap: () => MediaViewerPage.open(context, title: 'Diagrama', mermaid: code),
    );
  }
}
