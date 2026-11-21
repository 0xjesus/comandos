// «Ficha viva»: the pure rules of the web's «Sesiones» feed, copied from
// crates/comandos-web-view/src/session_feed.rs (mark, verb, ago, groups, chips,
// initials, agent glyph, the «Te espera» pin and the time buckets).
import '../../core/models/session_feed.dart';

/// The K5 mark of a row or tab chip: one accent, more intense = more urgent.
enum FeedMark {
  /// ask / limit: solid violet chip, violet verb and quote bar.
  waiting,

  /// work: the faint violet ring with a gap (spinning on the web).
  working,

  /// unread: 6–7 px violet dot.
  unread,
  none,
}

FeedMark feedMark(SessionSummary e) {
  if (e.state.waitsForHuman) return FeedMark.waiting;
  if (e.state == SessionState.work) return FeedMark.working;
  if (e.unread) return FeedMark.unread;
  return FeedMark.none;
}

/// «pide permiso», «está trabajando», «respondió», …
String feedVerb(SessionSummary e) {
  switch (e.state) {
    case SessionState.ask:
      return 'pide permiso';
    case SessionState.limit:
      return 'llegó al límite de uso';
    case SessionState.work:
      return 'está trabajando';
    case SessionState.idle:
      return e.reply.trim().isEmpty ? 'sin actividad' : 'respondió';
  }
}

/// «hace cuánto»: `ahora`, `14 min`, `3 h`, `2 d`. Empty when unknown.
String feedAgo(double now, double ts) {
  if (ts <= 0 || now <= 0) return '';
  final double minutes = ((now - ts) < 0 ? 0 : (now - ts)) / 60;
  final int m = minutes.floor();
  if (m < 1) return 'ahora';
  if (m < 60) return '$m min';
  final int hours = (m / 60).round();
  if (hours < 24) return '$hours h';
  return '${(hours / 24).round()} d';
}

enum FeedGroup {
  now('Ahora'),
  lastHour('Última hora'),
  today('Hoy'),
  before('Antes');

  const FeedGroup(this.label);
  final String label;
}

/// [dayStart]: local midnight in epoch seconds.
FeedGroup feedGroup(double now, double ts, double dayStart) {
  final double age = now - ts;
  if (ts <= 0) return FeedGroup.before;
  if (age < 300) return FeedGroup.now;
  if (age < 3600) return FeedGroup.lastHour;
  if (ts >= dayStart) return FeedGroup.today;
  return FeedGroup.before;
}

/// Local midnight of [nowSec] (phone timezone, like the browser).
double localDayStart(double nowSec) {
  final DateTime t = DateTime.fromMillisecondsSinceEpoch((nowSec * 1000).round());
  return DateTime(t.year, t.month, t.day).millisecondsSinceEpoch / 1000;
}

/// Avatar letters: first letter of the first two words, else the first two characters.
String feedInitials(String label) {
  final List<String> words = label
      .split(RegExp(r'[^\p{L}\p{N}]+', unicode: true))
      .where((String w) => w.isNotEmpty)
      .toList();
  if (words.isEmpty) return '';
  String take(String w, int n) {
    final List<int> r = w.runes.toList();
    return String.fromCharCodes(r.take(n));
  }

  final String out = words.length == 1 ? take(words[0], 2) : take(words[0], 1) + take(words[1], 1);
  return out.toUpperCase();
}

/// Small corner glyph of the agent: ✻ Claude, ◎ Codex.
String? feedGlyph(String agent) => switch (agent) {
      'claude' => '✻',
      'codex' => '◎',
      _ => null,
    };

/// The quoted block: the menu's question while it waits on one, else the reply.
String feedQuote(SessionSummary e) {
  final String q = e.ask?.question ?? '';
  if (e.state == SessionState.ask && q.trim().isNotEmpty) return _oneLine(q);
  return e.reply.trim();
}

String _oneLine(String s) => s.split(RegExp(r'\s+')).where((String w) => w.isNotEmpty).join(' ');

/// «Tú: …» text (one line).
String feedPrompt(SessionSummary e) => _oneLine(e.prompt);

/// Filter chips «Todas / Sin leer / Permisos / Trabajando».
enum FeedChip {
  all('Todas'),
  unread('Sin leer'),
  perm('Permisos'),
  work('Trabajando');

  const FeedChip(this.label);
  final String label;

  bool matches(SessionSummary e) => switch (this) {
        FeedChip.all => true,
        FeedChip.unread => e.unread,
        FeedChip.perm => e.state == SessionState.ask,
        FeedChip.work => e.state == SessionState.work,
      };
}

class FeedCounts {
  const FeedCounts({this.all = 0, this.unread = 0, this.perm = 0, this.work = 0, this.waiting = 0});

  final int all;
  final int unread;
  final int perm;
  final int work;

  /// ask + limit: the violet badge of the «Sesiones» tab.
  final int waiting;

  int of(FeedChip c) => switch (c) {
        FeedChip.all => all,
        FeedChip.unread => unread,
        FeedChip.perm => perm,
        FeedChip.work => work,
      };

  @override
  bool operator ==(Object other) =>
      other is FeedCounts &&
      other.all == all &&
      other.unread == unread &&
      other.perm == perm &&
      other.work == work &&
      other.waiting == waiting;

  @override
  int get hashCode => Object.hash(all, unread, perm, work, waiting);
}

FeedCounts feedCounts(List<SessionSummary> list) {
  int n(FeedChip c) => list.where(c.matches).length;
  return FeedCounts(
    all: list.length,
    unread: n(FeedChip.unread),
    perm: n(FeedChip.perm),
    work: n(FeedChip.work),
    waiting: list.where((SessionSummary e) => e.state.waitsForHuman).length,
  );
}

class FeedLayout {
  const FeedLayout(this.pinned, this.groups);

  /// «TE ESPERA · N» (oldest first, outside the scroll on the web).
  final List<SessionSummary> pinned;

  /// The rest, most recent first, cut into time buckets.
  final List<(FeedGroup, List<SessionSummary>)> groups;

  bool get isEmpty => pinned.isEmpty && groups.isEmpty;
}

/// Same as the web's `layout`: both parts respect the chip.
FeedLayout feedLayout(List<SessionSummary> list, FeedChip chip, double now, double dayStart) {
  final List<SessionSummary> pinned =
      list.where((SessionSummary e) => chip.matches(e) && e.state.waitsForHuman).toList();
  _stableSort(pinned, (SessionSummary a, SessionSummary b) => a.ts.compareTo(b.ts));
  final List<SessionSummary> rest =
      list.where((SessionSummary e) => chip.matches(e) && !e.state.waitsForHuman).toList();
  _stableSort(rest, (SessionSummary a, SessionSummary b) => b.ts.compareTo(a.ts));
  final List<(FeedGroup, List<SessionSummary>)> groups = <(FeedGroup, List<SessionSummary>)>[];
  for (final SessionSummary e in rest) {
    final FeedGroup g = feedGroup(now, e.ts, dayStart);
    if (groups.isNotEmpty && groups.last.$1 == g) {
      groups.last.$2.add(e);
    } else {
      groups.add((g, <SessionSummary>[e]));
    }
  }
  return FeedLayout(pinned, groups);
}

/// Whether opening the session would move its «seen» mark.
bool feedNeedsSeen(SessionSummary e) => e.unread || e.ts > e.seen;

void _stableSort<T>(List<T> list, int Function(T, T) cmp) {
  final List<(int, T)> indexed = <(int, T)>[for (int i = 0; i < list.length; i++) (i, list[i])];
  indexed.sort(((int, T) a, (int, T) b) {
    final int c = cmp(a.$2, b.$2);
    return c != 0 ? c : a.$1.compareTo(b.$1);
  });
  for (int i = 0; i < list.length; i++) {
    list[i] = indexed[i].$2;
  }
}

/// The model line of a tab (cv-head subtitle): first model, «+N» for the other panes.
/// Same shortening as the web's `tab_model_short`.
String modelLine(SessionSummary? e) {
  if (e == null) return '';
  final List<String> models = <String>[
    for (final PaneSummary p in e.paneList)
      if (p.model.isNotEmpty) p.model,
  ];
  if (models.isEmpty && e.model.isNotEmpty) models.add(e.model);
  if (models.isEmpty) return '';
  final String first = tabModelShort(models.first);
  return models.length > 1 ? '$first +${models.length - 1}' : first;
}

String tabModelShort(String model) {
  String m = model;
  final int nl = m.lastIndexOf(RegExp('[\n\r  ]'));
  final int lineStart = nl < 0 ? 0 : nl + 1;
  final int bracket = m.indexOf('[', lineStart);
  if (bracket >= 0) m = m.substring(0, bracket);
  m = m.replaceFirst('claude-', '');
  final int dash = m.lastIndexOf('-');
  if (dash >= 0) {
    final String tail = m.substring(dash + 1);
    if (tail == '5' || (tail.length == 8 && tail.startsWith('202') && RegExp(r'^\d+$').hasMatch(tail))) {
      return m.substring(0, dash);
    }
  }
  return m;
}
