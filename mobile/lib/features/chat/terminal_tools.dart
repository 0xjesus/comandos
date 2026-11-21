import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

// The terminal tools of the dock that the web forwards to the session's
// terminal frame, done natively over the same HTTP routes the frame uses:
// «Seleccionar», «Historial» and «Copiar» open the stable, selectable copy of
// the pane (`POST /terminal-history`, `#terminal-history` in term_page_body),
// «Paneles» opens «Paneles de esta sesión» (`POST /terminal-panes`,
// `#pane-dialog`).

Future<T?> _sheet<T>(BuildContext context, Widget child) {
  return showModalBottomSheet<T>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    backgroundColor: Orbita.panel,
    barrierColor: Colors.black54,
    shape: const RoundedRectangleBorder(
      borderRadius: BorderRadius.vertical(top: Radius.circular(14)),
      side: BorderSide(color: Orbita.line2),
    ),
    builder: (BuildContext c) => child,
  );
}

Widget _btn(String label, VoidCallback? onTap, {bool on = false, bool danger = false, bool primary = false}) => WebButton(
      onTap: onTap,
      on: on,
      face: primary ? Orbita.brand : Orbita.bg,
      ink: danger ? Orbita.err : (primary ? Colors.white : Orbita.text),
      radius: 10,
      lip: 2,
      minHeight: 40,
      child: Text(label, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13.5, fontWeight: FontWeight.w600)),
    );

/// Opens the selectable history of [pane]. [status]: a first line under the
/// controls (e.g. «Selecciona texto para copiar, o pulsa Todo.»).
Future<void> showTerminalHistory(BuildContext context, {required String session, String? pane, String? status}) {
  return _sheet<void>(context, _History(session: session, pane: pane, status: status));
}

class _History extends StatefulWidget {
  const _History({required this.session, this.pane, this.status});
  final String session;
  final String? pane;
  final String? status;

  @override
  State<_History> createState() => _HistoryState();
}

class _HistoryState extends State<_History> {
  final ScrollController _scroll = ScrollController();
  String _text = '';
  String? _pane;
  List<Map<String, Object?>> _panes = const <Map<String, Object?>>[];
  String _status = 'Cargando historial…';
  bool _busy = false;
  bool _copied = false;

  @override
  void initState() {
    super.initState();
    _pane = widget.pane;
    unawaited(_load(toEnd: true));
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _load({bool toEnd = false}) async {
    final ComandosApi? api = sl.get<Connection>().api;
    if (api == null || _busy) return;
    setState(() => _busy = true);
    try {
      final ApiResponse r = await api.request('POST', '/terminal-history', body: <String, Object?>{
        'session': widget.session,
        'pane': ?_pane,
        'lines': 2000,
      });
      if (!mounted) return;
      final Map<String, Object?> m = r.map;
      if (!r.ok || m['ok'] != true) {
        setState(() => _status = (m['error'] as String?) ?? 'Historial temporalmente no disponible');
        return;
      }
      final Object? panes = m['panes'];
      setState(() {
        _text = (m['text'] as String?) ?? '';
        _pane = (m['pane'] as String?) ?? _pane;
        _panes = panes is List ? <Map<String, Object?>>[for (final Object? p in panes) if (p is Map) p.cast<String, Object?>()] : const <Map<String, Object?>>[];
        _status = widget.status ??
            (m['truncated'] == true
                ? 'Se muestran las últimas ${m['lines']} líneas. Mantén presionado para seleccionar.'
                : 'Mantén presionado el texto para seleccionarlo y copiarlo.');
      });
      if (toEnd) _toEnd();
    } on ApiException catch (e) {
      if (mounted) setState(() => _status = e.userMessage);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  void _toEnd() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
  }

  Future<void> _copyAll() async {
    await Clipboard.setData(ClipboardData(text: _text));
    if (!mounted) return;
    setState(() => _copied = true);
    await Future<void>.delayed(const Duration(milliseconds: 1200));
    if (mounted) setState(() => _copied = false);
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * .86,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 10, 8, 4),
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: <Widget>[
                  _btn('← Volver', () => Navigator.of(context).pop()),
                  const SizedBox(width: 6),
                  _btn(_copied ? 'Copiado' : 'Copiar todo', _text.isEmpty ? null : _copyAll, on: _copied),
                  const SizedBox(width: 6),
                  _btn('Actualizar', _busy ? null : () => unawaited(_load())),
                  const SizedBox(width: 6),
                  _btn('Final', _toEnd),
                  if (_panes.length > 1)
                    for (final (int i, Map<String, Object?> p) in _panes.indexed) ...<Widget>[
                      const SizedBox(width: 6),
                      _btn('Panel ${i + 1}', () {
                        _pane = p['id'] as String?;
                        unawaited(_load(toEnd: true));
                      }, on: p['id'] == _pane),
                    ],
                ],
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 4, 12, 6),
            child: Text(_status, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 12.5, color: Orbita.dim)),
          ),
          Expanded(
            child: Container(
              margin: const EdgeInsets.fromLTRB(8, 0, 8, 8),
              decoration: BoxDecoration(
                color: Orbita.bg,
                borderRadius: BorderRadius.circular(10),
                border: Border.all(color: Orbita.line),
              ),
              child: SingleChildScrollView(
                controller: _scroll,
                padding: const EdgeInsets.all(10),
                child: SelectableText(
                  _text,
                  style: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, height: 1.35, color: Orbita.text),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// «Paneles de esta sesión»: activate a pane (it becomes the dock's target),
/// split one, or close a split after confirming that exact pane.
/// Completes with the pane chosen to activate, or null.
Future<String?> showPanes(BuildContext context, {required String session}) {
  return _sheet<String>(context, _Panes(session: session));
}

class _Panes extends StatefulWidget {
  const _Panes({required this.session});
  final String session;

  @override
  State<_Panes> createState() => _PanesState();
}

class _PanesState extends State<_Panes> {
  List<Map<String, Object?>> _panes = const <Map<String, Object?>>[];
  String _status = 'Cargando paneles…';
  bool _busy = false;
  Map<String, Object?>? _closing;

  @override
  void initState() {
    super.initState();
    unawaited(_run('list'));
  }

  static String _label(Map<String, Object?> p, int i) {
    final String title = (p['title'] as String?) ?? '';
    final String kind = title.isEmpty || const <String>{'zsh', 'bash', 'sh', 'fish', 'dash'}.contains(title) ? 'terminal' : title;
    return 'Panel ${i + 1} · $kind';
  }

  Future<void> _run(String action, {Map<String, Object?>? pane, String? direction}) async {
    final ComandosApi? api = sl.get<Connection>().api;
    if (api == null || _busy) return;
    setState(() {
      _busy = true;
      if (action != 'list') {
        _status = switch (action) {
          'close' => 'Guardando copia y cerrando el split…',
          'split' => 'Abriendo split…',
          _ => 'Consultando…',
        };
      }
    });
    try {
      final ApiResponse r = await api.request('POST', '/terminal-panes', body: <String, Object?>{
        'session': widget.session,
        'action': action,
        if (pane != null) 'pane': pane['id'],
        if (pane != null) 'identity': pane['identity'],
        'direction': ?direction,
      }, timeout: const Duration(seconds: 30));
      if (!mounted) return;
      final Map<String, Object?> m = r.map;
      final Object? list = m['panes'];
      if (!r.ok || m['ok'] != true || list is! List) {
        setState(() {
          _closing = null;
          _status = '${(m['error'] as String?) ?? 'No se pudo consultar el estado de los paneles'}'
              '${action == 'list' ? '' : '. Vuelve a abrir Paneles para comprobar el estado.'}';
        });
        return;
      }
      final List<Map<String, Object?>> panes = <Map<String, Object?>>[for (final Object? p in list) if (p is Map) p.cast<String, Object?>()];
      if (action == 'split') {
        final Object? opened = m['opened'];
        Navigator.of(context).pop(opened is String ? opened : null);
        return;
      }
      setState(() {
        _closing = null;
        _panes = panes;
        _status = action == 'close'
            ? 'Split cerrado. Los demás paneles siguen abiertos.'
            : panes.length == 1
                ? 'Un solo panel. Divídelo para abrir un split.'
                : 'Elige un panel para activarlo, dividirlo o cerrar su split.';
      });
    } on ApiException catch (e) {
      if (mounted) setState(() => _status = e.userMessage);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final Map<String, Object?>? closing = _closing;
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 14, 14, 10),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            const Text('Paneles de esta sesión',
                style: TextStyle(fontFamily: Orbita.sans, fontSize: 16, fontWeight: FontWeight.w700, color: Orbita.text)),
            const SizedBox(height: 6),
            Text(_status, style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.dim)),
            const SizedBox(height: 10),
            if (closing != null) ...<Widget>[
              Text('¿Cerrar ${_label(closing, _panes.indexOf(closing))}?',
                  style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600, color: Orbita.text)),
              const SizedBox(height: 4),
              const Text('Se cerrará el proceso de este panel.',
                  style: TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.dim)),
              const SizedBox(height: 10),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: <Widget>[
                  _btn('Cancelar', () => setState(() => _closing = null)),
                  const SizedBox(width: 6),
                  _btn('Cerrar split', _busy ? null : () => unawaited(_run('close', pane: closing)), danger: true),
                ],
              ),
            ] else
              Flexible(
                child: ListView(
                  shrinkWrap: true,
                  children: <Widget>[
                    for (final (int i, Map<String, Object?> p) in _panes.indexed)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 8),
                        child: Wrap(
                          spacing: 6,
                          runSpacing: 6,
                          crossAxisAlignment: WrapCrossAlignment.center,
                          children: <Widget>[
                            WebButton(
                              // Activating is per device (`scope: client` on the web): the
                              // pane becomes this phone's target; the desktop keeps its focus.
                              onTap: _busy ? null : () => Navigator.of(context).pop(p['id'] as String?),
                              on: p['active'] == true,
                              face: Orbita.bg,
                              ink: Orbita.text,
                              radius: 10,
                              lip: 2,
                              minHeight: 44,
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: <Widget>[
                                  Text('${_label(p, i)}${p['active'] == true ? ' · Activo' : ''}',
                                      style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13.5, fontWeight: FontWeight.w700)),
                                  if ((p['path'] as String?)?.isNotEmpty ?? false)
                                    Text(p['path']! as String,
                                        style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint, fontWeight: FontWeight.w400)),
                                ],
                              ),
                            ),
                            _btn('Dividir →', _busy ? null : () => unawaited(_run('split', pane: p, direction: 'right'))),
                            _btn('Dividir ↓', _busy ? null : () => unawaited(_run('split', pane: p, direction: 'down'))),
                            _btn('Cerrar', _busy || _panes.length <= 1 ? null : () => setState(() => _closing = p), danger: true),
                          ],
                        ),
                      ),
                  ],
                ),
              ),
            const SizedBox(height: 6),
            Align(alignment: Alignment.centerRight, child: _btn('Volver', () => Navigator.of(context).pop())),
          ],
        ),
      ),
    );
  }
}
