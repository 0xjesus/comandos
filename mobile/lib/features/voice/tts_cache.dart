import 'dart:io';

import '../../core/util/clock.dart';

/// Small on-disk LRU of synthesised speech chunks, so replaying a reply is
/// instant and works offline. Bounded by [maxBytes]; the least recently
/// used files go first. Lives in the cache dir (Android may clear it; it is
/// only a cache).
class TtsCache {
  TtsCache(this._dir, {this.maxBytes = 20 * 1024 * 1024, NowMs now = systemNowMs}) : _now = now;

  final Future<String> Function() _dir;
  final int maxBytes;
  final NowMs _now;

  /// key → (bytes, last use ms). Built from the directory on first use.
  Map<String, (int, int)>? _index;
  String? _path;

  static const String ext = '.mp3';

  Future<Map<String, (int, int)>> _load() async {
    final Map<String, (int, int)>? ready = _index;
    if (ready != null) return ready;
    final String dir = _path = await _dir();
    final Directory d = Directory(dir);
    await d.create(recursive: true);
    final Map<String, (int, int)> index = <String, (int, int)>{};
    await for (final FileSystemEntity e in d.list()) {
      if (e is! File) continue;
      final String name = e.uri.pathSegments.last;
      if (name.endsWith('.tmp')) {
        await _delete(e);
        continue;
      }
      if (!name.endsWith(ext)) continue;
      try {
        final FileStat st = await e.stat();
        index[name.substring(0, name.length - ext.length)] =
            (st.size, st.modified.millisecondsSinceEpoch);
      } catch (_) {}
    }
    return _index = index;
  }

  String _file(String key) => '$_path/$key$ext';

  /// The cached file for [key], or null. Marks it as recently used.
  Future<File?> get(String key) async {
    final Map<String, (int, int)> index = await _load();
    final (int, int)? hit = index[key];
    if (hit == null) return null;
    final File f = File(_file(key));
    if (!await f.exists()) {
      index.remove(key);
      return null;
    }
    final int now = _now();
    index[key] = (hit.$1, now);
    try {
      await f.setLastModified(DateTime.fromMillisecondsSinceEpoch(now));
    } catch (_) {}
    return f;
  }

  /// Stores [bytes] under [key] (atomic rename) and evicts down to the cap.
  Future<File> put(String key, List<int> bytes) async {
    final Map<String, (int, int)> index = await _load();
    final File f = File(_file(key));
    final File tmp = File('${f.path}.tmp');
    await tmp.writeAsBytes(bytes, flush: true);
    await tmp.rename(f.path);
    final int now = _now();
    try {
      await f.setLastModified(DateTime.fromMillisecondsSinceEpoch(now));
    } catch (_) {}
    index[key] = (bytes.length, now);
    await _evict(keep: key);
    return f;
  }

  int get totalBytes => (_index?.values ?? const <(int, int)>[]).fold(0, (int a, (int, int) e) => a + e.$1);

  Future<void> _evict({String? keep}) async {
    final Map<String, (int, int)> index = _index!;
    int total = totalBytes;
    if (total <= maxBytes) return;
    final List<MapEntry<String, (int, int)>> byAge = index.entries.toList()
      ..sort((MapEntry<String, (int, int)> a, MapEntry<String, (int, int)> b) => a.value.$2.compareTo(b.value.$2));
    for (final MapEntry<String, (int, int)> e in byAge) {
      if (total <= maxBytes) break;
      if (e.key == keep) continue;
      index.remove(e.key);
      total -= e.value.$1;
      await _delete(File(_file(e.key)));
    }
  }

  /// Drops one entry (e.g. a file the player could not decode).
  Future<void> remove(String key) async {
    await _load();
    _index!.remove(key);
    await _delete(File(_file(key)));
  }

  static Future<void> _delete(File f) async {
    try {
      await f.delete();
    } catch (_) {}
  }
}
