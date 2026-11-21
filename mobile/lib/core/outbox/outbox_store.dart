import '../util/atomic_json_file.dart';
import 'outbox_item.dart';

/// Where the outbox lives between process restarts.
abstract class OutboxStore {
  Future<List<OutboxItem>> load();

  /// Replaces the whole persisted queue. Completes once durable.
  Future<void> save(List<OutboxItem> items);
}

/// `outbox.json` in the app support directory, written atomically
/// (temp file + fsync + rename) by [AtomicJsonFile].
class FileOutboxStore implements OutboxStore {
  FileOutboxStore(String path) : _file = AtomicJsonFile(path);

  final AtomicJsonFile _file;

  @override
  Future<List<OutboxItem>> load() async {
    final Object? json = await _file.read();
    final Object? list = json is Map ? json['items'] : null;
    if (list is! List) return <OutboxItem>[];
    return <OutboxItem>[
      for (final Object? e in list)
        if (OutboxItem.fromJson(e) case final OutboxItem item) item,
    ];
  }

  @override
  Future<void> save(List<OutboxItem> items) => _file.write(<String, Object?>{
        'v': 1,
        'items': <Object?>[for (final OutboxItem i in items) i.toJson()],
      });
}

/// In-memory store for tests; [saves] counts durable writes.
class MemoryOutboxStore implements OutboxStore {
  List<OutboxItem> saved = <OutboxItem>[];
  int saves = 0;
  bool failNext = false;

  @override
  Future<List<OutboxItem>> load() async => List<OutboxItem>.of(saved);

  @override
  Future<void> save(List<OutboxItem> items) async {
    if (failNext) {
      failNext = false;
      throw StateError('disk full');
    }
    saved = List<OutboxItem>.of(items);
    saves++;
  }
}
