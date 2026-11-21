import 'dart:async';
import 'dart:convert';
import 'dart:io';

/// A small JSON document on disk that is always replaced atomically.
///
/// Every write goes to `<path>.tmp`, is flushed to storage, and is then
/// renamed over `<path>`. A crash or power loss leaves either the old or the
/// new document, never a torn one. Writes are serialised; while one is in
/// progress, later writes coalesce into a single follow-up write of the most
/// recent value, and every caller's future completes only once a write that
/// includes its value has reached disk.
class AtomicJsonFile {
  AtomicJsonFile(this.path);

  final String path;

  Future<void>? _running;
  Object? _pendingValue;
  bool _hasPending = false;
  Completer<void>? _pendingDone;

  /// Reads the document. Returns null when it does not exist. A corrupt file
  /// is moved aside to `<path>.corrupt-<ms>` (never deleted) and null returned.
  Future<Object?> read() async {
    final File file = File(path);
    if (!await file.exists()) return null;
    final String text = await file.readAsString();
    if (text.trim().isEmpty) return null;
    try {
      return jsonDecode(text);
    } on FormatException {
      final int ms = DateTime.now().millisecondsSinceEpoch;
      await file.rename('$path.corrupt-$ms');
      return null;
    }
  }

  /// Writes [value] (any JSON-encodable object). Completes after it is durable.
  Future<void> write(Object? value) {
    _pendingValue = value;
    _hasPending = true;
    final Completer<void> done = _pendingDone ??= Completer<void>();
    _running ??= _drain();
    return done.future;
  }

  Future<void> _drain() async {
    try {
      while (_hasPending) {
        final Object? value = _pendingValue;
        final Completer<void> done = _pendingDone!;
        _hasPending = false;
        _pendingValue = null;
        _pendingDone = null;
        try {
          await _writeNow(value);
          done.complete();
        } catch (error, stack) {
          done.completeError(error, stack);
        }
      }
    } finally {
      _running = null;
    }
  }

  Future<void> _writeNow(Object? value) async {
    final File target = File(path);
    await target.parent.create(recursive: true);
    final File tmp = File('$path.tmp');
    final RandomAccessFile raf = await tmp.open(mode: FileMode.write);
    try {
      await raf.writeString(jsonEncode(value));
      await raf.flush();
    } finally {
      await raf.close();
    }
    await tmp.rename(path);
  }
}
