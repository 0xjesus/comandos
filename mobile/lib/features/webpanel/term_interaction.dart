// «Seleccionar» / «Interactuar» of the terminal: tmux mouse off lets the finger
// select text, on gives the touches back to tmux. On the web the dashboard (the
// terminal's parent) owns this (comandos-web/src/components/
// app_coordinator_interaction.rs); here the app is the parent. Pure Dart.

import 'dart:async';

import '../../core/api/api_response.dart';
import 'web_scripts.dart';

typedef TermRequest = Future<ApiResponse> Function(String method, String path,
    {Map<String, String>? query, Map<String, Object?>? body});

class TermInteraction {
  TermInteraction({required this.session, required TermRequest request, required void Function(Map<String, Object?>) post})
      : _request = request,
        _post = post;

  final String session;
  final TermRequest _request;
  final void Function(Map<String, Object?>) _post;

  /// tmux mouse: true = on (interact), false = off (selecting), null = unknown.
  bool? mouse;
  bool busy = false;

  /// Selection was turned on from here and must be undone when leaving.
  bool temporary = false;
  bool _restorePending = false;
  int _seq = 0;
  bool _closed = false;

  Map<String, Object?> get state => interactionMessage(known: mouse != null, busy: busy, selecting: mouse == false);

  void _send() {
    if (!_closed) _post(state);
  }

  /// Reads tmux's current mouse mode (`GET /tmux-mouse?session=`).
  Future<void> sync() => _begin(null);

  /// The page asked to toggle (`interaction-request`).
  Future<void> request(bool selecting) => _begin(selecting);

  /// Leaving the session or the screen: give tmux its mouse back if selection
  /// was turned on here (the web does it on hide and on session switch).
  Future<void> restore() async {
    if (busy) {
      _restorePending = true;
      return;
    }
    if (!temporary) return;
    await _begin(false);
  }

  /// No more messages to the page (it is going away); a pending restore still runs.
  void close() => _closed = true;

  Future<void> _begin(bool? selecting) async {
    if (busy) {
      if (selecting == false) _restorePending = true;
      return;
    }
    final int seq = ++_seq;
    busy = true;
    _send();
    try {
      final ApiResponse r = selecting == null
          ? await _request('GET', '/tmux-mouse', query: <String, String>{'session': session})
          : await _request('POST', '/tmux-mouse',
              body: <String, Object?>{'session': session, 'enabled': !selecting});
      if (seq != _seq) return;
      if (!r.ok) throw StateError('tmux-mouse ${r.status}');
      if (selecting != null) {
        mouse = !selecting;
        temporary = selecting;
        if (!selecting) _restorePending = false;
      } else {
        final Object? m = r.map['mouse'];
        if (m == 'on') {
          mouse = true;
          temporary = false;
        } else if (m == 'off') {
          mouse = false;
        }
      }
    } on Object {
      // Unknown state: the page keeps «Comprobando…» until the next sync.
    } finally {
      if (seq == _seq) {
        busy = false;
        _send();
        final bool again = _restorePending && temporary;
        _restorePending = false;
        if (again) unawaited(_begin(false));
      }
    }
  }
}
