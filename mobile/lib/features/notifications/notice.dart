import '../../core/models/json_read.dart';

// Pure Dart (no Flutter imports): `tool/live_check.dart` parses the real
// server's answers with these same classes.

/// What a notice means for the phone.
enum NoticeKind {
  /// A permission / input request: the agent waits for the person.
  needsHuman,

  /// The turn finished (completed or cancelled).
  done,

  /// The turn failed.
  error,

  /// Anything else (news, usage, focus, info): never a phone notification.
  other,
}

/// One entry of `GET /notices` (`notices[]`).
class Notice {
  const Notice({
    required this.eventId,
    required this.sequence,
    required this.kind,
    required this.category,
    required this.needsHuman,
    required this.occurredAtMs,
    this.project,
    this.sessionKey,
    this.paneId,
    this.title = '',
    this.excerpt = '',
    this.read = false,
  });

  final String eventId;
  final int sequence;

  /// Server event kind (`turn_completed`, `permission_requested`, …).
  final String kind;

  /// Server category (`attention`, `done`, `error`, `news`, …).
  final String category;
  final bool needsHuman;

  /// Project label; null for news.
  final String? project;

  /// tmux session the event came from (null for news / global events).
  final String? sessionKey;

  /// tmux pane id (`%51`), when known.
  final String? paneId;
  final String title;
  final String excerpt;
  final int occurredAtMs;

  /// Already marked read on the desktop.
  final bool read;

  NoticeKind get phoneKind {
    if (needsHuman) return NoticeKind.needsHuman;
    switch (category) {
      case 'attention':
        return NoticeKind.needsHuman;
      case 'done':
        return NoticeKind.done;
      case 'error':
        return NoticeKind.error;
    }
    return NoticeKind.other;
  }

  static Notice? tryParse(Map<String, Object?> m) {
    final String id = readString(m, 'eventId');
    final Object? seq = m['sequence'];
    if (id.isEmpty || seq is! num) return null;
    return Notice(
      eventId: id,
      sequence: seq.toInt(),
      kind: readString(m, 'kind'),
      category: readString(m, 'category'),
      needsHuman: readBool(m, 'needsHuman'),
      project: readStringOrNull(m, 'project') ?? readStringOrNull(m, 'projectKey'),
      sessionKey: readStringOrNull(m, 'sessionKey'),
      paneId: readStringOrNull(m, 'paneId'),
      title: readString(m, 'title'),
      excerpt: readString(m, 'excerpt'),
      occurredAtMs: readInt(m, 'occurredAtMs'),
      read: readBool(m, 'read'),
    );
  }
}

List<String> _stringList(Map<String, Object?> m, String k) {
  final Object? v = m[k];
  if (v is! List) return const <String>[];
  return <String>[for (final Object? e in v) if (e is String) e];
}

/// `GET /notices?after=&limit=` answer.
class NoticePage {
  const NoticePage({required this.notices, required this.nextAfter, this.pending = const <String>[]});

  final List<Notice> notices;

  /// Cursor for the next page: the last *event* sequence of this page
  /// (notices are a filtered subset of events), or the `after` sent when
  /// the page was empty.
  final int nextAfter;

  /// Event ids of live "needs a human" requests.
  final List<String> pending;

  static NoticePage? tryParse(Object? json, {required int after}) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null || m['notices'] is! List) return null;
    final Object? next = m['nextAfter'];
    return NoticePage(
      notices: <Notice>[
        for (final Map<String, Object?> n in readMapList(m, 'notices'))
          if (Notice.tryParse(n) case final Notice notice) notice,
      ],
      nextAfter: next is num ? next.toInt() : after,
      pending: _stringList(m, 'pending'),
    );
  }
}

/// `GET /notices/watch?rev=&wait=` answer.
class NoticeWatch {
  const NoticeWatch({required this.rev, required this.latest, this.pending = const <String>[]});

  /// Opaque revision; send it back as `rev` to wait for the next change.
  final String rev;

  /// Highest event sequence on the server.
  final int latest;

  /// Event ids of live "needs a human" requests.
  final List<String> pending;

  static NoticeWatch? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null) return null;
    final Object? rev = m['rev'];
    final Object? latest = m['latest'];
    if (rev is! String || latest is! num) return null;
    return NoticeWatch(rev: rev, latest: latest.toInt(), pending: _stringList(m, 'pending'));
  }
}
