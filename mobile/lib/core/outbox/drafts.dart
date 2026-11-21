import 'dart:async';

import '../util/atomic_json_file.dart';

/// Per-session composer drafts, persisted as the person types (debounced)
/// so a half-written message survives process death.
class DraftStore {
  DraftStore(String path, {this.debounce = const Duration(milliseconds: 400)})
      : _file = AtomicJsonFile(path);

  final AtomicJsonFile _file;
  final Duration debounce;
  final Map<String, String> _drafts = <String, String>{};
  Timer? _timer;

  /// Reads drafts from disk. Call once at boot.
  Future<void> load() async {
    final Object? json = await _file.read();
    if (json is Map) {
      json.forEach((Object? k, Object? v) {
        if (k is String && v is String && v.isNotEmpty) _drafts[k] = v;
      });
    }
  }

  String get(String session) => _drafts[session] ?? '';

  /// Updates the draft and schedules a write.
  void set(String session, String text) {
    if (text.isEmpty) {
      if (_drafts.remove(session) == null) return;
    } else {
      if (_drafts[session] == text) return;
      _drafts[session] = text;
    }
    _timer?.cancel();
    _timer = Timer(debounce, () {
      _timer = null;
      unawaited(_write());
    });
  }

  /// Sets and writes immediately (used before discarding an outbox item for
  /// "Editar", so the text is durable in one of the two places).
  Future<void> setNow(String session, String text) {
    _timer?.cancel();
    _timer = null;
    if (text.isEmpty) {
      _drafts.remove(session);
    } else {
      _drafts[session] = text;
    }
    return _write();
  }

  /// Writes any pending change now (app going to background).
  Future<void> flush() async {
    if (_timer == null) return;
    _timer!.cancel();
    _timer = null;
    await _write();
  }

  Future<void> _write() async {
    try {
      await _file.write(Map<String, String>.of(_drafts));
    } catch (_) {
      // Drafts are best effort; the outbox is the durable path.
    }
  }

  void dispose() => _timer?.cancel();
}
