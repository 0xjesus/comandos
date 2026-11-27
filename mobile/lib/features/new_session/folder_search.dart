import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/api/api_errors.dart';
import '../../core/models/json_read.dart';
import 'wizard_model.dart';

// The «Carpeta» section of «Nueva sesión» (same design as the web and the
// desktop): one field that searches every folder of the HOME (`GET /fs/find`),
// browses a typed path (`GET /fs/dirs`) or lists the recent folders, and a
// chosen folder whose subfolders can be drilled into.

/// `GET <path>?<query>` returning decoded JSON or throwing.
typedef JsonGet = Future<Object?> Function(String path, Map<String, String> query);

/// One row of the results list.
@immutable
class FolderEntry {
  const FolderEntry({
    required this.path,
    required this.name,
    required this.short,
    this.repo = false,
    this.recent = false,
  });

  /// Absolute path.
  final String path;

  /// Last segment, shown bold.
  final String name;

  /// `~/…` form, shown dimmed.
  final String short;

  /// A git repository (branch icon).
  final bool repo;

  /// The folder of a live session («reciente» badge).
  final bool recent;

  @override
  bool operator ==(Object other) =>
      other is FolderEntry &&
      other.path == path &&
      other.name == name &&
      other.short == short &&
      other.repo == repo &&
      other.recent == recent;

  @override
  int get hashCode => Object.hash(path, name, short, repo, recent);

  @override
  String toString() => 'FolderEntry($short${recent ? ', reciente' : ''}${repo ? ', repo' : ''})';
}

/// A `GET /fs/dirs?path=` answer.
@immutable
class DirsAnswer {
  const DirsAnswer({
    required this.path,
    required this.base,
    required this.exists,
    this.missing = '',
    this.home,
    this.dirs = const <(String, String)>[],
  });

  /// `{path, exists, base, parent, missing, home, dirs: [{name, path}]}`; null
  /// for anything else (an `error` answer included).
  static DirsAnswer? parse(Object? d) {
    final Map<String, Object?>? m = asMap(d);
    if (m == null) return null;
    final Object? err = m['error'];
    if (err != null && err != false && err != '') return null;
    return DirsAnswer(
      path: readString(m, 'path'),
      base: readString(m, 'base'),
      exists: m['exists'] == true,
      missing: readString(m, 'missing'),
      home: readStringOrNull(m, 'home'),
      dirs: <(String, String)>[
        for (final Map<String, Object?> x in readMapList(m, 'dirs')) (readString(x, 'name'), readString(x, 'path')),
      ],
    );
  }

  /// The resolved absolute path that was asked for.
  final String path;

  /// The deepest part of [path] that exists.
  final String base;
  final bool exists;

  /// What is missing below [base] («nonexist/xyz»).
  final String missing;
  final String? home;

  /// `(name, path)` of the subfolders of [base].
  final List<(String, String)> dirs;
}

/// What the list under the field shows.
enum FolderView {
  /// Empty field: «Recientes».
  recent,

  /// Text: `/fs/find` results.
  search,

  /// Text starting with `/` or `~`: crumbs and subfolders of that path.
  path,

  /// A chosen folder: the pill and its subfolders.
  picked,
}

/// State of the «Carpeta» section. Every request bumps one sequence number and
/// only the newest answer paints, so a slow reply never overwrites a newer one;
/// the previous rows stay visible while the next answer is on its way.
class FolderSearch extends ChangeNotifier {
  FolderSearch({
    required this.get,
    List<String> recent = const <String>[],
    this.home,
    this.debounce = const Duration(milliseconds: 120),
  }) : _recent = recent;

  final JsonGet get;
  final Duration debounce;

  /// The HOME as the server reports it (for `~/…`).
  String? home;

  List<String> _recent;
  List<String> get recent => _recent;

  String _query = '';
  String get query => _query;

  String? _picked;

  /// The chosen folder (absolute or as typed); null until one is chosen.
  String? get picked => _picked;

  bool _editing = true;

  /// True while the field is shown (no folder chosen, or «Cambiar»).
  bool get editing => _editing || _picked == null;

  /// A request is on its way.
  bool loading = false;

  /// `/fs/find` is missing on this server (older version): search falls back
  /// to the folders of `~` and `~/codebase`.
  bool findUnavailable = false;

  /// Rows of the search view.
  List<FolderEntry> results = const <FolderEntry>[];

  /// The `/fs/dirs` answer of the path view or of the chosen folder.
  DirsAnswer? dirs;

  /// The path [dirs] was asked for.
  String? dirsFor;

  int _seq = 0;
  Timer? _timer;
  bool _disposed = false;
  Future<List<FolderEntry>>? _fallback;

  FolderView get view {
    if (!editing) return FolderView.picked;
    final String q = _query.trim();
    if (q.isEmpty) return FolderView.recent;
    return WizardPaths.isPathQuery(q) ? FolderView.path : FolderView.search;
  }

  /// The chosen folder's last segment («ComandOS»).
  String get pickedName => _picked == null ? '' : WizardPaths.folderName(_picked!);

  /// `~/…` form of [path].
  String short(String path) => WizardPaths.shortHome(path, home);

  /// Whether the chosen folder is known to be missing.
  bool get pickedMissing => _picked != null && dirsFor == _picked && dirs != null && !dirs!.exists;

  /// Rows of the recent view.
  List<FolderEntry> get recentEntries => <FolderEntry>[
        for (final String p in _recent) _entry(p, recent: true),
      ];

  FolderEntry _entry(String path, {bool repo = false, bool recent = false, String? shortPath}) => FolderEntry(
        path: path,
        name: WizardPaths.folderName(path),
        short: shortPath ?? short(path),
        repo: repo,
        recent: recent,
      );

  void setRecent(List<String> folders) {
    _recent = folders;
    _notify();
  }

  void setHome(String? h) {
    if (h == null || h.isEmpty || h == home) return;
    home = h;
    _notify();
  }

  /// The field changed: switch view now, ask the server after [debounce].
  void setQuery(String q) {
    _query = q;
    _timer?.cancel();
    final String t = q.trim();
    if (t.isEmpty) {
      _seq++;
      loading = false;
      _notify();
      return;
    }
    _editing = true;
    _notify();
    _timer = Timer(debounce, () {
      if (WizardPaths.isPathQuery(t)) {
        unawaited(_runDirs(t));
      } else {
        unawaited(_runFind(t));
      }
    });
  }

  /// Chooses [path]: the field collapses into the pill and the list shows its
  /// subfolders.
  void pick(String path) {
    _timer?.cancel();
    _picked = path;
    _editing = false;
    _query = '';
    if (dirsFor != path) dirs = null;
    unawaited(_runDirs(path));
  }

  /// «Cambiar»: the field again, empty (recent folders).
  void change() {
    _timer?.cancel();
    _seq++;
    _editing = true;
    _query = '';
    loading = false;
    _notify();
  }

  /// Back to the chosen folder after «Cambiar» (clear button on an empty field).
  void keepPicked() {
    if (_picked == null) return;
    pick(_picked!);
  }

  /// Asks `/fs/dirs` again for the chosen folder (after a «no existe»).
  void refreshPicked() {
    if (_picked != null) unawaited(_runDirs(_picked!));
  }

  /// Fills the search view from the remote index, or from the fallback lists.
  Future<void> _runFind(String q) async {
    final int seq = ++_seq;
    loading = true;
    _notify();
    if (findUnavailable) return _runFallback(q, seq);
    Object? d;
    try {
      d = await get('/fs/find', <String, String>{'q': q, 'limit': '40'});
    } on Object catch (e) {
      // An older server answers 404: search the HOME and ~/codebase instead
      // for the rest of this opening. Other failures only skip this query.
      if (e is NotFoundException || e is BadResponseException || (e is ApiException && e.status == 405)) {
        findUnavailable = true;
      }
      if (_disposed || seq != _seq) return;
      return _runFallback(q, seq);
    }
    if (_disposed || seq != _seq) return;
    final Map<String, Object?>? m = asMap(d);
    if (m == null) {
      findUnavailable = true;
      return _runFallback(q, seq);
    }
    final String? h = readStringOrNull(m, 'home');
    if (h != null && h.isNotEmpty) home = h;
    final List<FolderEntry> found = <FolderEntry>[
      for (final Map<String, Object?> r in readMapList(m, 'results'))
        if (readString(r, 'path').isNotEmpty)
          FolderEntry(
            path: readString(r, 'path'),
            name: readStringOrNull(r, 'name') ?? WizardPaths.folderName(readString(r, 'path')),
            short: readStringOrNull(r, 'short') ?? short(readString(r, 'path')),
            repo: r['repo'] == true,
          ),
    ];
    results = _merge(q, found);
    loading = false;
    _notify();
  }

  Future<void> _runFallback(String q, int seq) async {
    _fallback ??= _loadFallback();
    final List<FolderEntry> all = await _fallback!;
    if (_disposed || seq != _seq) return;
    final List<(int, int, FolderEntry)> scored = <(int, int, FolderEntry)>[];
    for (int i = 0; i < all.length; i++) {
      final int? s = WizardPaths.matchScore(q, all[i].name);
      if (s != null) scored.add((s, i, all[i]));
    }
    scored.sort(((int, int, FolderEntry) a, (int, int, FolderEntry) b) =>
        a.$1 != b.$1 ? a.$1.compareTo(b.$1) : a.$2.compareTo(b.$2));
    results = _merge(q, <FolderEntry>[for (final (int, int, FolderEntry) s in scored.take(40)) s.$3]);
    loading = false;
    _notify();
  }

  /// The subfolders of `~/codebase` and `~` (what the old wizard browsed).
  Future<List<FolderEntry>> _loadFallback() async {
    final List<FolderEntry> out = <FolderEntry>[];
    final Set<String> seen = <String>{};
    for (final String p in <String>['~/codebase', '~']) {
      try {
        final DirsAnswer? a = DirsAnswer.parse(await get('/fs/dirs', <String, String>{'path': p}));
        if (a == null || !a.exists) continue;
        if (a.home != null && a.home!.isNotEmpty) home = a.home;
        for (final (String name, String path) in a.dirs) {
          if (seen.add(path)) out.add(FolderEntry(path: path, name: name, short: short(path)));
        }
      } on Object {
        continue;
      }
    }
    return out;
  }

  /// Recent folders whose name matches go first (with the badge); the rest
  /// keep the server's order (a recent one found there keeps its badge).
  List<FolderEntry> _merge(String q, List<FolderEntry> found) {
    final Set<String> recentSet = _recent.toSet();
    final List<(int, int, String)> hits = <(int, int, String)>[];
    for (int i = 0; i < _recent.length; i++) {
      final String p = _recent[i];
      // Only a name that contains the text: letters in order or a match deeper
      // in the path would push real results down.
      final int? s = WizardPaths.matchScore(q, WizardPaths.folderName(p));
      if (s != null && s <= 3) hits.add((s, i, p));
    }
    hits.sort(((int, int, String) a, (int, int, String) b) => a.$1 != b.$1 ? a.$1.compareTo(b.$1) : a.$2.compareTo(b.$2));
    final Map<String, FolderEntry> byPath = <String, FolderEntry>{for (final FolderEntry e in found) e.path: e};
    final List<FolderEntry> out = <FolderEntry>[
      for (final (int, int, String) h in hits)
        _entry(h.$3, recent: true, repo: byPath[h.$3]?.repo ?? false, shortPath: byPath[h.$3]?.short),
    ];
    final Set<String> listed = <String>{for (final FolderEntry e in out) e.path};
    for (final FolderEntry e in found) {
      if (listed.contains(e.path)) continue;
      out.add(recentSet.contains(e.path)
          ? FolderEntry(path: e.path, name: e.name, short: e.short, repo: e.repo, recent: true)
          : e);
    }
    return out;
  }

  Future<void> _runDirs(String path) async {
    final int seq = ++_seq;
    loading = true;
    _notify();
    Object? d;
    try {
      d = await get('/fs/dirs', <String, String>{'path': path});
    } on Object {
      if (_disposed || seq != _seq) return;
      loading = false;
      _notify();
      return;
    }
    if (_disposed || seq != _seq) return;
    final DirsAnswer? a = DirsAnswer.parse(d);
    loading = false;
    if (a != null) {
      dirs = a;
      dirsFor = path;
      if (a.home != null && a.home!.isNotEmpty) home = a.home;
    }
    _notify();
  }

  /// Subfolders to list under the path view: when the typed path does not
  /// exist yet, the ones of its deepest existing folder that start like the
  /// missing part («~/codebase/Com» → «ComandOS»).
  List<(String, String)> get pathDirs {
    final DirsAnswer? a = dirs;
    if (a == null) return const <(String, String)>[];
    if (a.exists) return a.dirs;
    if (a.missing.isEmpty || a.missing.contains('/')) return const <(String, String)>[];
    final String m = a.missing.toLowerCase();
    return <(String, String)>[
      for (final (String, String) d in a.dirs)
        if (d.$1.toLowerCase().startsWith(m)) d,
    ];
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}
