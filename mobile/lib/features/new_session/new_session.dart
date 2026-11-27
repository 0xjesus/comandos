import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../core/api/api_client.dart';
import '../../core/connection.dart';
import '../../core/locator.dart';
import '../../core/models/json_read.dart';
import '../../core/models/provider_registry.dart';
import '../../ui/theme.dart';
import '../../ui/web_kit.dart';
import 'folder_search.dart';
import 'folder_widgets.dart';
import 'profile_picker.dart';
import 'provider_mark.dart';
import 'wizard_api.dart';
import 'wizard_model.dart';
import 'wizard_widgets.dart';

/// Opens «Nueva sesión»; completes with the new session name or null.
///
/// Same design as the web and the desktop: «Carpeta» (search every folder of
/// the HOME, browse a path or pick a recent one), «Agente», «Modelo» and «Más
/// opciones». On a phone it is a full-screen page with a fixed header and a
/// fixed footer above the keyboard; on a wide screen a centred card.
Future<String?> showNewSession(BuildContext context) {
  return showGeneralDialog<String>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Cerrar',
    barrierColor: const Color(0x88000000), // #0008
    transitionDuration: Duration.zero, // the web toggles .hidden, no animation
    pageBuilder: (BuildContext context, Animation<double> a, Animation<double> b) => const NewSessionPanel(),
  );
}

/// What survives between openings, like the web's `NS` global, the chosen
/// folder and the `#ns-danger` switch.
class _Memory {
  static ProviderRegistry? registry;
  static bool tiersLoaded = false;
  static WizardSelection sel = WizardSelection.defaults();
  static String cwd = '';
  static String? home;
  static bool danger = false;
}

/// Test hook: forget what earlier openings left behind.
@visibleForTesting
void resetNewSessionMemory() {
  _Memory.registry = null;
  _Memory.tiersLoaded = false;
  _Memory.sel = WizardSelection.defaults();
  _Memory.cwd = '';
  _Memory.home = null;
  _Memory.danger = false;
  ProviderMarks.reset();
}

/// Width from which the wizard is a centred card instead of a full page.
const double _cardBreakpoint = 700;

/// Rows of «Buscar modelo» drawn at once; the rest wait for a narrower query.
const int _modelRowsShown = 60;

/// Subfolders of the chosen folder listed before «Ver las N subcarpetas».
const int _subsShown = 6;

/// The «Nueva sesión» page. Public for widget tests; use [showNewSession].
class NewSessionPanel extends StatefulWidget {
  const NewSessionPanel({super.key});

  @override
  State<NewSessionPanel> createState() => _NewSessionPanelState();
}

class _NewSessionPanelState extends State<NewSessionPanel> {
  final TextEditingController _q = TextEditingController();
  final FocusNode _qFocus = FocusNode();
  late final WizardModel _m = WizardModel(_Memory.registry ?? const ProviderRegistry(), _Memory.sel);
  late final FolderSearch _folders = FolderSearch(get: _get, home: _Memory.home);

  bool _loaded = false;
  String? _loadError;

  bool _modelSearchOpen = false;
  final TextEditingController _modelQ = TextEditingController();
  final GlobalKey _modelSearchKey = GlobalKey();

  /// Every subfolder of the chosen folder, not only the first [_subsShown].
  bool _allSubs = false;

  bool _moreOpen = false;
  bool _unavailableOpen = false;
  bool _creating = false;
  String _profileNote = 'Sin perfil';

  String? _toast;
  bool _toastErr = false;
  Timer? _toastTimer;

  ComandosApi? get _api => sl.maybe<Connection>()?.api;

  Future<Object?> _get(String path, Map<String, String> query) {
    final ComandosApi? api = _api;
    if (api == null) return Future<Object?>.error(StateError('sin conexión'));
    return api.getJson(path, query: query);
  }

  @override
  void initState() {
    super.initState();
    if (_Memory.cwd.isNotEmpty) _folders.pick(_Memory.cwd);
    _folders.addListener(_onFolders);
    _qFocus.addListener(() {
      if (mounted) setState(() {});
    });
    if (_Memory.registry != null) {
      _m.selectDefaults();
      _m.normalize();
      _loaded = true;
    }
    unawaited(_open());
  }

  void _onFolders() {
    if (_folders.home != null) _Memory.home = _folders.home;
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    _toastTimer?.cancel();
    _folders
      ..removeListener(_onFolders)
      ..dispose();
    _q.dispose();
    _qFocus.dispose();
    _modelQ.dispose();
    super.dispose();
  }

  // ---- Data ----------------------------------------------------------------

  /// `open()`: fresh `/providers` on every opening (an account added with the
  /// dashboard open must show), recent folders, the active plan.
  Future<void> _open() async {
    final ComandosApi? api = _api;
    if (api == null) {
      setState(() => _loadError = 'Vincula el teléfono con tu equipo para crear sesiones.');
      return;
    }
    final Future<Object?> providers = api.getJson('/providers');
    final Future<Object?> tiers =
        _Memory.tiersLoaded ? Future<Object?>.value(null) : api.getJson('/model-tiers').catchError((Object _) => null);
    final Future<Object?> live = api.getJson('/state', query: const <String, String>{'live': '1'})
        .catchError((Object _) => null);
    final Future<Object?> active = api.getJson('/active-tab').catchError((Object _) => null);
    final Future<Object?> plans = api.getJson('/optimization/plans').catchError((Object _) => null);
    // The HOME, so recent folders read `~/…` before anything is typed.
    if (_folders.home == null && _folders.picked == null) {
      unawaited(api.getJson('/fs/dirs', query: const <String, String>{'path': '~'}).then((Object? d) {
        _folders.setHome(DirsAnswer.parse(d)?.home);
      }, onError: (Object _) {}));
    }

    try {
      final ProviderRegistry? reg = ProviderRegistry.tryParse(await providers);
      if (reg != null) {
        _Memory.registry = reg;
        _m.registry = reg;
      }
    } on Object catch (e) {
      if (_Memory.registry == null && mounted) {
        setState(() => _loadError = errorText(e));
        return;
      }
    }
    final Object? t = await tiers;
    if (t != null) {
      ProviderMarks.setRules(t);
      _Memory.tiersLoaded = true;
    }

    // «Recientes»: the folders of the live sessions; the current tab's folder
    // is chosen when nothing was chosen before.
    final Object? list = await live;
    final List<Map<String, Object?>> items = <Map<String, Object?>>[
      if (list is List)
        for (final Object? it in list)
          if (asMap(it) != null) asMap(it)!,
    ];
    _folders.setRecent(WizardPaths.recentFolders(items));
    final Map<String, Object?>? tab = asMap(await active);
    if (_folders.picked == null && _folders.query.trim().isEmpty && tab != null) {
      final String s = readString(tab, 'session');
      final String p = readString(tab, 'pane');
      String? cwd;
      for (final Map<String, Object?> it in items) {
        if (readString(it, 'session') == s && (p.isEmpty || readString(it, 'pane') == p)) {
          cwd = readStringOrNull(it, 'cwd');
          if (cwd != null && cwd.isNotEmpty) break;
        }
      }
      if (cwd != null && cwd.isNotEmpty) _pick(cwd);
    }

    final Map<String, Object?>? opts = asMap(await plans);
    if (opts != null) {
      final Object? activeId = opts['active'];
      for (final Map<String, Object?> plan in readMapList(opts, 'plans')) {
        if (plan['id'] == activeId) {
          _m.applyPlanVariant(asMap(asMap(plan['variants'])?[_m.sel.harness]));
        }
      }
    }
    if (!mounted) return;
    _m.selectDefaults();
    _m.normalize();
    setState(() {
      _loaded = true;
      _loadError = null;
    });
  }

  // ---- Folder --------------------------------------------------------------

  void _pick(String path) {
    _allSubs = false;
    _Memory.cwd = path;
    _q.clear();
    _qFocus.unfocus();
    _folders.pick(path);
  }

  void _change() {
    _q.clear();
    _folders.change();
    _qFocus.requestFocus();
  }

  void _clearQuery() {
    if (_q.text.isEmpty && _folders.picked != null) {
      _qFocus.unfocus();
      _folders.keepPicked();
      return;
    }
    _q.clear();
    _folders.setQuery('');
  }

  /// Enter: a typed path is chosen as it is; a search takes its first row.
  void _submitted(String v) {
    final String t = v.trim();
    if (t.isEmpty) return;
    if (WizardPaths.isPathQuery(t)) {
      final DirsAnswer? d = _folders.dirs;
      _pick(d != null && d.exists && _folders.dirsFor == t ? d.path : t);
      return;
    }
    if (_folders.results.isNotEmpty) _pick(_folders.results.first.path);
  }

  /// Breadcrumb of the path view: browse that folder.
  void _browseTo(String path) {
    final String short = _folders.short(path);
    final String text = short.endsWith('/') ? short : '$short/';
    _q
      ..text = text
      ..selection = TextSelection.collapsed(offset: text.length);
    _folders.setQuery(text);
  }

  Future<void> _mkdir(String path) async {
    final ComandosApi? api = _api;
    if (api == null) return;
    try {
      final Map<String, Object?> r = await api.postForm('/fs/mkdir', <String, Object?>{'path': path});
      final String made = readString(r, 'path');
      _showToast('Carpeta creada: ${_folders.short(made)}');
      _pick(made.isEmpty ? path : made);
    } on Object catch (e) {
      _showToast(errorText(e), err: true);
    }
  }

  // ---- Create --------------------------------------------------------------

  /// `go()`; [shell] is «Solo terminal» (plain shell, the agent choice kept).
  Future<void> _create({bool shell = false}) async {
    if (_creating) return;
    final String cwd = _folders.picked?.trim() ?? '';
    if (cwd.isEmpty) {
      _showToast('Falta la carpeta', err: true);
      return;
    }
    final ComandosApi? api = _api;
    if (api == null) return;
    setState(() => _creating = true);
    try {
      final Map<String, Object?> r = await api.postForm(
        '/session-new',
        _m.createBody(cwd: cwd, danger: _Memory.danger, shell: shell),
      );
      final String session = readString(r, 'session');
      if (!mounted) return;
      Navigator.of(context).pop(session.isEmpty ? null : session);
    } on Object catch (e) {
      if (!mounted) return;
      setState(() => _creating = false);
      final String message = errorText(e);
      if (message.contains('no existe')) _folders.refreshPicked();
      _showToast(WizardPaths.createErrorText(message), err: true);
    }
  }

  Future<void> _chooseProfile() async {
    final ComandosApi? api = _api;
    if (api == null) return;
    final String cwd = _folders.picked ?? '';
    final String? id = await showProfilePicker(
      context,
      harness: _m.sel.harness,
      account: _m.sel.harnessAccount,
      cwd: cwd,
    );
    if (id == null || !mounted) return;
    try {
      final Map<String, Object?> r = await api.postForm(
        '/session-profile-apply',
        <String, Object?>{'profileId': id, 'cwd': cwd},
      );
      if (!mounted) return;
      setState(() {
        _m.applyProfileDraft(id, asMap(r['launchDraft']) ?? const <String, Object?>{});
        _profileNote = 'Perfil seleccionado. Skills y MCPs se aplicarán al iniciar.';
      });
    } on Object catch (e) {
      _showToast(errorText(e), err: true);
    }
  }

  void _clearProfile() => setState(() {
        _m.sel.profileId = null;
        _profileNote = 'Sin perfil';
      });

  void _showToast(String text, {bool err = false}) {
    if (!mounted) return;
    _toastTimer?.cancel();
    setState(() {
      _toast = text;
      _toastErr = err;
    });
    _toastTimer = Timer(const Duration(seconds: 4), () {
      if (mounted) setState(() => _toast = null);
    });
  }

  // ---- Layout --------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final Size screen = MediaQuery.sizeOf(context);
    final EdgeInsets insets = MediaQuery.viewInsetsOf(context);
    final EdgeInsets safe = MediaQuery.paddingOf(context);
    final bool card = screen.width >= _cardBreakpoint;
    final Widget page = Material(
      type: MaterialType.transparency,
      child: DefaultTextStyle(
        style: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, color: Orbita.text),
        child: Column(
          children: <Widget>[
            _header(card),
            Expanded(child: _body()),
            _footer(card ? 0 : safe.bottom),
          ],
        ),
      ),
    );
    final Widget toast = _toast == null
        ? const SizedBox.shrink()
        : Positioned(
            right: 12,
            top: safe.top + 50,
            child: WizardToast(text: _toast!, error: _toastErr),
          );

    if (!card) {
      return Stack(
        children: <Widget>[
          Positioned.fill(
            child: ColoredBox(
              color: Orbita.bg,
              child: Padding(
                padding: EdgeInsets.only(top: safe.top, bottom: insets.bottom),
                child: page,
              ),
            ),
          ),
          toast,
        ],
      );
    }
    final double width = math.min(640, screen.width * .92);
    final double maxHeight = (screen.height - insets.bottom - safe.vertical) * .92;
    return Stack(
      children: <Widget>[
        // backdrop-filter: blur(3px); taps fall through to the barrier.
        Positioned.fill(
          child: IgnorePointer(
            child: BackdropFilter(filter: ui.ImageFilter.blur(sigmaX: 3, sigmaY: 3), child: const SizedBox.expand()),
          ),
        ),
        Padding(
          padding: EdgeInsets.only(top: safe.top, bottom: insets.bottom + safe.bottom),
          child: Center(
            child: Container(
              width: width,
              height: maxHeight,
              decoration: BoxDecoration(
                color: Orbita.bg,
                border: Border.all(color: Orbita.line2),
                borderRadius: BorderRadius.circular(16),
                boxShadow: const <BoxShadow>[BoxShadow(color: Color(0xAA000000), offset: Offset(0, 18), blurRadius: 50)],
              ),
              child: ClipRRect(borderRadius: BorderRadius.circular(15), child: page),
            ),
          ),
        ),
        toast,
      ],
    );
  }

  Widget _header(bool card) => Container(
        decoration: const BoxDecoration(
          color: Orbita.bar,
          border: Border(bottom: BorderSide(color: Orbita.line)),
        ),
        padding: const EdgeInsets.fromLTRB(16, 4, 4, 4),
        child: Row(
          children: <Widget>[
            const WebIcon(WebIcons.plus, size: 18, color: Orbita.brand),
            const SizedBox(width: 10),
            const Expanded(
              child: Text(
                'Nueva sesión',
                style: TextStyle(fontFamily: Orbita.sans, fontSize: 17, fontWeight: FontWeight.w700, color: Orbita.text),
              ),
            ),
            Semantics(
              button: true,
              label: 'Cerrar',
              child: InkResponse(
                key: const ValueKey<String>('ns-close'),
                onTap: () => Navigator.of(context).pop(),
                radius: 24,
                child: const SizedBox(
                  width: 48,
                  height: 48,
                  child: Center(child: WebIcon(WebIcons.close, size: 20, color: Orbita.dim)),
                ),
              ),
            ),
          ],
        ),
      );

  Widget _body() {
    final List<Widget> sections = <Widget>[_folderSection()];
    if (_loadError != null && !_loaded) {
      sections.add(NsHint(_loadError!, color: Orbita.waiting));
    } else if (!_loaded) {
      sections.add(const NsHint('Cargando…'));
    } else {
      sections
        ..addAll(_choices())
        ..add(_moreOptions());
    }
    return SingleChildScrollView(
      key: const ValueKey<String>('ns-body'),
      keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
      padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          for (int i = 0; i < sections.length; i++) ...<Widget>[if (i > 0) const SizedBox(height: 22), sections[i]],
        ],
      ),
    );
  }

  Widget _footer(double bottomSafe) {
    final String? picked = _folders.picked;
    final bool hasFolder = picked != null && picked.isNotEmpty;
    final bool canCreate = _loaded && hasFolder && _m.canCreate && !_creating;
    final String name = _folders.pickedName;
    final String note = _loaded ? _m.note : '';
    return Container(
      decoration: const BoxDecoration(
        color: Orbita.bar,
        border: Border(top: BorderSide(color: Orbita.line)),
      ),
      padding: EdgeInsets.fromLTRB(16, 10, 16, 12 + bottomSafe),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          if (note.isNotEmpty || !hasFolder)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                !hasFolder ? 'Elige una carpeta para crear la sesión' : note,
                key: const ValueKey<String>('ns-note'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint),
              ),
            ),
          Row(
            children: <Widget>[
              NsActionButton(
                key: const ValueKey<String>('ns-term'),
                label: 'Solo terminal',
                icon: WebIcons.terminal,
                tooltip: 'Abrir una terminal simple en esta carpeta, sin agente',
                onTap: _creating || !hasFolder ? null : () => _create(shell: true),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: NsActionButton(
                  key: const ValueKey<String>('ns-go'),
                  label: _creating
                      ? 'Creando…'
                      : hasFolder
                          ? 'Crear en $name'
                          : 'Crear sesión',
                  primary: true,
                  tooltip: _loaded ? _m.createTitle : null,
                  onTap: canCreate ? _create : null,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  // ---- «Carpeta» -----------------------------------------------------------

  Widget _folderSection() {
    final FolderSearch f = _folders;
    final List<Widget> out = <Widget>[const NsLabel('Carpeta')];
    if (f.view == FolderView.picked) {
      out
        ..add(FolderPill(
          name: f.pickedName,
          short: f.short(f.picked!),
          missing: f.pickedMissing,
          onChange: _change,
        ))
        ..addAll(_pickedList());
    } else {
      out.add(FolderField(
        controller: _q,
        focusNode: _qFocus,
        loading: f.loading,
        showClear: f.picked != null,
        onChanged: f.setQuery,
        onSubmitted: _submitted,
        onClear: _clearQuery,
      ));
      out.add(const SizedBox(height: 6));
      out.addAll(switch (f.view) {
        FolderView.recent => _recentList(),
        FolderView.search => _searchList(),
        FolderView.path => _pathList(),
        FolderView.picked => const <Widget>[],
      });
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: out);
  }

  List<Widget> _recentList() {
    final List<FolderEntry> rows = _folders.recentEntries;
    if (rows.isEmpty) {
      return const <Widget>[NsHint('Escribe el nombre de un proyecto para buscarlo en tu equipo, o una ruta con / o ~.')];
    }
    return <Widget>[
      const Padding(
        padding: EdgeInsets.fromLTRB(4, 6, 4, 2),
        child: Text('Recientes', style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Orbita.dim)),
      ),
      for (final FolderEntry e in rows.take(12))
        FolderRow(entry: e, showRecent: false, onTap: () => _pick(e.path)),
    ];
  }

  List<Widget> _searchList() {
    final FolderSearch f = _folders;
    final String q = f.query.trim();
    final List<Widget> out = <Widget>[
      if (f.findUnavailable)
        const NsHint('Tu equipo aún no tiene la búsqueda completa: busco en ~ y ~/codebase. '
            'Para otra ruta escribe / o ~.'),
      for (final FolderEntry e in f.results) FolderRow(entry: e, query: q, onTap: () => _pick(e.path)),
    ];
    if (f.results.isEmpty && !f.loading) out.add(NsHint('Sin carpetas que coincidan con «$q».'));
    return out;
  }

  List<Widget> _pathList() {
    final FolderSearch f = _folders;
    final DirsAnswer? d = f.dirs;
    if (d == null) return <Widget>[if (!f.loading) const NsHint('Escribe una ruta, por ejemplo ~/codebase/')];
    final List<(String, String)> subs = f.pathDirs;
    return <Widget>[
      NsCrumbs(base: d.base, home: f.home, onTap: _browseTo),
      const SizedBox(height: 4),
      if (d.exists)
        FolderRow(
          key: const ValueKey<String>('ns-use-path'),
          entry: FolderEntry(path: d.path, name: 'Usar ${WizardPaths.folderName(d.path)}', short: f.short(d.path)),
          showRecent: false,
          onTap: () => _pick(d.path),
        )
      else
        MissingFolder(short: f.short(d.path), onCreate: () => _mkdir(d.path)),
      if (subs.isNotEmpty) ...<Widget>[
        const SizedBox(height: 6),
        for (final (String name, String path) in subs)
          FolderRow(
            entry: FolderEntry(path: path, name: name, short: f.short(path)),
            query: d.exists ? '' : d.missing,
            onTap: () => _pick(path),
          ),
      ] else if (d.exists)
        const NsHint('Sin subcarpetas.'),
    ];
  }

  List<Widget> _pickedList() {
    final FolderSearch f = _folders;
    final String picked = f.picked!;
    final DirsAnswer? d = f.dirsFor == picked ? f.dirs : null;
    if (d == null) {
      return <Widget>[
        if (f.loading)
          const Padding(
            padding: EdgeInsets.only(top: 12),
            child: Center(
              child: SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 1.8)),
            ),
          ),
      ];
    }
    if (!d.exists) {
      return <Widget>[
        const SizedBox(height: 8),
        MissingFolder(short: f.short(d.path), onCreate: () => _mkdir(d.path)),
      ];
    }
    return <Widget>[
      const SizedBox(height: 6),
      NsCrumbs(base: d.base, home: f.home, onTap: _pick),
      if (d.dirs.isNotEmpty) ...<Widget>[
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 2, 4, 2),
          child: Text(
            'Subcarpetas de ${f.pickedName}',
            style: const TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Orbita.dim),
          ),
        ),
        for (final (String name, String path) in _allSubs ? d.dirs : d.dirs.take(_subsShown))
          FolderRow(
            entry: FolderEntry(path: path, name: name, short: f.short(path)),
            compact: true,
            onTap: () => _pick(path),
          ),
        if (!_allSubs && d.dirs.length > _subsShown)
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(
              key: const ValueKey<String>('ns-subs-all'),
              onPressed: () => setState(() => _allSubs = true),
              style: TextButton.styleFrom(
                minimumSize: const Size(44, 44),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                foregroundColor: Orbita.brand,
                textStyle: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600),
              ),
              child: Text('Ver las ${d.dirs.length} subcarpetas'),
            ),
          ),
      ],
    ];
  }

  // ---- «Agente», «Modelo» ----------------------------------------------------

  List<Widget> _choices() {
    final WizardModel m = _m;
    // «Atajos»: one tap sets harness, motor, model and effort; nothing at all
    // when the server sends no usable preset.
    final List<WizardChip> presets = m.presetChips;
    final List<Widget> out = <Widget>[
      if (presets.isNotEmpty)
        NsWrap(
          label: 'Atajos',
          child: NsChips(
            key: const ValueKey<String>('ns-presets'),
            items: presets,
            current: m.currentPresetId,
            onPick: (String id) => setState(() {
              m.applyPresetId(id);
              _closeModelSearch();
            }),
          ),
        ),
      NsWrap(
        label: 'Agente',
        child: NsChips(
          key: const ValueKey<String>('ns-harness'),
          items: m.harnessChips,
          current: m.sel.harness,
          onPick: (String id) => setState(() {
            m.pickHarness(id);
            _closeModelSearch();
          }),
        ),
      ),
    ];
    if (!m.isShell) {
      out
        ..add(NsWrap(
          label: 'Motor',
          child: NsChips(
            key: const ValueKey<String>('ns-motor'),
            items: m.motorChips,
            current: m.sel.motor,
            onPick: (String id) => setState(() {
              m.pickMotor(id);
              _closeModelSearch();
            }),
          ),
        ))
        ..add(_modelSection());
      if (m.effortChips.isNotEmpty) {
        out.add(NsWrap(
          label: 'Pensamiento',
          child: NsChips(
            key: const ValueKey<String>('ns-effort'),
            items: m.effortChips,
            current: m.sel.effort,
            onPick: (String id) => setState(() => m.pickEffort(id)),
          ),
        ));
      }
    }
    if (m.showHarnessAccounts) {
      out.add(NsWrap(
        label: 'Cuenta del harness',
        child: NsChips(
          key: const ValueKey<String>('ns-acct'),
          items: m.harnessAccountChips,
          current: m.sel.harnessAccount,
          onPick: (String id) => setState(() => m.pickHarnessAccount(id)),
        ),
      ));
    }
    if (m.showMotorAccounts) {
      out.add(NsWrap(
        label: 'Cuenta del motor',
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            NsChips(
              key: const ValueKey<String>('ns-motor-acct'),
              items: m.motorAccountChips,
              current: m.sel.motorAccount,
              onPick: (String id) => setState(() => m.pickMotorAccount(id)),
            ),
            const SizedBox(height: 6),
            const Text(WizardModel.motorAccountNote, style: nsNoteStyle),
          ],
        ),
      ));
    }
    return out;
  }

  void _closeModelSearch() {
    _modelSearchOpen = false;
    _modelQ.clear();
  }

  /// Opens «Buscar modelo» and brings its field to the top of the page, so
  /// the list shows above the keyboard.
  void _openModelSearch() {
    setState(() => _modelSearchOpen = true);
    void reveal() {
      final BuildContext? c = _modelSearchKey.currentContext;
      if (c != null && c.mounted) {
        unawaited(Scrollable.ensureVisible(c, alignment: .02, duration: const Duration(milliseconds: 180)));
      }
    }

    WidgetsBinding.instance.addPostFrameCallback((_) => reveal());
    // Again once the keyboard has shrunk the page.
    Timer(const Duration(milliseconds: 450), () {
      if (mounted && _modelSearchOpen) reveal();
    });
  }

  void _pickModel(String id) {
    if (_modelSearchOpen) FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      _m.pickModel(id);
      _closeModelSearch();
    });
  }

  Widget _modelSection() {
    final WizardModel m = _m;
    final List<Widget> rows = <Widget>[
      NsChips(
        key: const ValueKey<String>('ns-model'),
        items: m.visibleModelChips,
        current: m.sel.model,
        onPick: _pickModel,
      ),
    ];
    final String tag = m.model.tag;
    if (tag.isNotEmpty && !NsChip.shortTag(tag)) {
      rows.add(Padding(
        padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
        child: Text(
          '${m.model.name ?? m.model.id} · $tag',
          key: const ValueKey<String>('ns-model-tag'),
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(fontSize: 12, color: Orbita.faint),
        ),
      ));
    }
    if (m.hasModelSearch) {
      if (!_modelSearchOpen) {
        rows.add(const SizedBox(height: 10));
        rows.add(Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: const ValueKey<String>('ns-model-search-open'),
            onPressed: _openModelSearch,
            icon: const WebIcon(WebIcons.search, size: 15, color: Orbita.brand),
            label: Text('Buscar modelo (${m.modelList.length})'),
            style: TextButton.styleFrom(
              minimumSize: const Size(44, 44),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              foregroundColor: Orbita.brand,
              textStyle: const TextStyle(fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600),
            ),
          ),
        ));
      } else {
        final List<WizardChip> found = m.filterModels(_modelQ.text);
        rows
          ..add(KeyedSubtree(
            key: _modelSearchKey,
            // The room above keeps the field clear of the header once revealed.
            child: Padding(padding: const EdgeInsets.only(top: 12), child: _modelSearchField(m.modelList.length)),
          ))
          ..add(const SizedBox(height: 6));
        rows.add(Container(
          key: const ValueKey<String>('ns-model-results'),
          decoration: BoxDecoration(
            color: Orbita.panel,
            border: Border.all(color: Orbita.line),
            borderRadius: BorderRadius.circular(12),
          ),
          padding: const EdgeInsets.all(4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: <Widget>[
              for (final WizardChip c in found.take(_modelRowsShown))
                NsModelRow(
                  key: ValueKey<String>('ns-model-row:${c.id}'),
                  item: c,
                  on: c.id == m.sel.model,
                  onTap: () => _pickModel(c.id),
                ),
              if (found.isEmpty) NsHint('Ningún modelo coincide con «${_modelQ.text.trim()}».'),
              if (found.length > _modelRowsShown)
                NsHint('y ${found.length - _modelRowsShown} más: escribe para afinar.'),
            ],
          ),
        ));
      }
    }
    return NsWrap(
      label: 'Modelo',
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: rows),
    );
  }

  Widget _modelSearchField(int total) => TextField(
        key: const ValueKey<String>('ns-model-q'),
        scrollPadding: const EdgeInsets.only(bottom: 320),
        controller: _modelQ,
        autofocus: true,
        autocorrect: false,
        enableSuggestions: false,
        onChanged: (_) => setState(() {}),
        style: const TextStyle(fontFamily: Orbita.sans, fontSize: 15, color: Orbita.text),
        cursorColor: Orbita.brand,
        decoration: InputDecoration(
          hintText: 'Buscar entre $total modelos',
          isDense: true,
          contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 13),
          prefixIcon: const SizedBox(
            width: 42,
            child: Center(child: WebIcon(WebIcons.search, size: 16, color: Orbita.dim)),
          ),
          prefixIconConstraints: const BoxConstraints(minWidth: 42, minHeight: 44),
          suffixIcon: IconButton(
            key: const ValueKey<String>('ns-model-search-close'),
            tooltip: 'Cerrar búsqueda',
            onPressed: () => setState(_closeModelSearch),
            icon: const WebIcon(WebIcons.close, size: 16, color: Orbita.dim),
          ),
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(12),
            borderSide: const BorderSide(color: Orbita.line2),
          ),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(12),
            borderSide: const BorderSide(color: Orbita.line2),
          ),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(12),
            borderSide: const BorderSide(color: Orbita.brand),
          ),
        ),
      );

  // ---- «Más opciones» --------------------------------------------------------

  Widget _moreOptions() {
    final WizardModel m = _m;
    final List<Widget> flags = <Widget>[
      if (_Memory.danger) const NsBadge('sin aprobaciones', color: Orbita.waiting),
      if (m.sel.hasProfile) const NsBadge('con perfil'),
    ];
    final Widget head = Semantics(
      button: true,
      expanded: _moreOpen,
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          key: const ValueKey<String>('ns-more'),
          onTap: () => setState(() => _moreOpen = !_moreOpen),
          borderRadius: BorderRadius.circular(10),
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 48),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: Row(
                children: <Widget>[
                  WebIcon(_moreOpen ? WebIcons.chevronDown : WebIcons.chevronRight, size: 14, color: Orbita.dim),
                  const SizedBox(width: 10),
                  const Text(
                    'Más opciones',
                    style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600, color: Orbita.text),
                  ),
                  const SizedBox(width: 10),
                  Expanded(child: Wrap(spacing: 6, runSpacing: 4, children: flags)),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    if (!_moreOpen) return _box(head);
    return _box(Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        head,
        const Divider(height: 1),
        const SizedBox(height: 6),
        _dangerRow(),
        const SizedBox(height: 10),
        const NsLabel('Perfil'),
        Text(_profileNote, style: const TextStyle(fontSize: 13, height: 1.45, color: Orbita.dim)),
        const SizedBox(height: 8),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: <Widget>[
            NsActionButton(
              key: const ValueKey<String>('ns-profile-open'),
              label: 'Elegir perfil',
              onTap: _loaded ? _chooseProfile : null,
            ),
            NsActionButton(
              key: const ValueKey<String>('ns-profile-clear'),
              label: 'Quitar perfil',
              onTap: m.sel.hasProfile ? _clearProfile : null,
            ),
          ],
        ),
        if (m.showUnavailable) ...<Widget>[
          const SizedBox(height: 10),
          BlockedRoutes(
            summary: m.unavailableSummary,
            rows: m.blockedRoutes,
            open: _unavailableOpen,
            onToggle: () => setState(() => _unavailableOpen = !_unavailableOpen),
          ),
        ],
        const SizedBox(height: 6),
      ],
    ));
  }

  Widget _box(Widget child) => Container(
        decoration: BoxDecoration(
          color: Orbita.panel,
          border: Border.all(color: Orbita.line),
          borderRadius: BorderRadius.circular(12),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 2),
        child: child,
      );

  Widget _dangerRow() => Semantics(
        toggled: _Memory.danger,
        label: 'sin aprobaciones (dangerously)',
        child: GestureDetector(
          key: const ValueKey<String>('ns-danger'),
          behavior: HitTestBehavior.opaque,
          onTap: () => setState(() => _Memory.danger = !_Memory.danger),
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 52),
            child: Row(
              children: <Widget>[
                NsSwitch(on: _Memory.danger),
                const SizedBox(width: 12),
                const Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: <Widget>[
                      Text(
                        'sin aprobaciones (dangerously)',
                        style: TextStyle(fontFamily: Orbita.mono, fontSize: 12.5, color: Orbita.text),
                      ),
                      SizedBox(height: 2),
                      Text(
                        'Apagado: el agente pide permiso antes de acciones riesgosas. '
                        'Enciéndelo solo si confías en la tarea.',
                        style: TextStyle(fontSize: 12, height: 1.35, color: Orbita.faint),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      );
}
