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
import 'profile_picker.dart';
import 'provider_mark.dart';
import 'wizard_api.dart';
import 'wizard_model.dart';
import 'wizard_widgets.dart';

/// Opens the "Nueva sesión" wizard; completes with the new session name or null.
///
/// A native copy of the web's `#newsess` dialog (index_page_body.html,
/// foundation_helpers_wizard.rs): a panel over a dimmed, blurred backdrop.
/// Tapping the backdrop or «Cancelar» closes it with null.
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

/// What survives between openings, like the web's `NS` global, the `#ns-cwd`
/// value and the `#ns-danger` switch.
class _Memory {
  static ProviderRegistry? registry;
  static bool tiersLoaded = false;
  static WizardSelection sel = WizardSelection.defaults();
  static String cwd = '';
  static bool danger = false;
}

/// Test hook: forget what earlier openings left behind.
@visibleForTesting
void resetNewSessionMemory() {
  _Memory.registry = null;
  _Memory.tiersLoaded = false;
  _Memory.sel = WizardSelection.defaults();
  _Memory.cwd = '';
  _Memory.danger = false;
  ProviderMarks.reset();
}

/// The `.ns-card` panel. Public for widget tests; use [showNewSession].
class NewSessionPanel extends StatefulWidget {
  const NewSessionPanel({super.key});

  @override
  State<NewSessionPanel> createState() => _NewSessionPanelState();
}

class _NewSessionPanelState extends State<NewSessionPanel> {
  late final TextEditingController _cwd = TextEditingController(text: _Memory.cwd);
  final FocusNode _cwdFocus = FocusNode();
  late final WizardModel _m = WizardModel(_Memory.registry ?? const ProviderRegistry(), _Memory.sel);

  bool _loaded = false;
  String? _loadError;
  List<String> _recent = const <String>[];
  bool _dirsOpen = false;

  // `/fs/dirs` answer.
  int _fsSeq = 0;
  bool _fsShown = false;
  String _fsBase = '';
  String _fsPath = '';
  String? _fsHome;
  bool _fsExists = true;
  List<(String, String)> _fsDirs = const <(String, String)>[];
  Timer? _typing;

  bool _unavailableOpen = false;
  bool _creating = false;
  String _profileNote = 'Sin perfil';

  String? _toast;
  bool _toastErr = false;
  Timer? _toastTimer;

  ComandosApi? get _api => sl.maybe<Connection>()?.api;

  @override
  void initState() {
    super.initState();
    _cwd.addListener(() => _Memory.cwd = _cwd.text);
    _cwdFocus.addListener(() {
      if (!_cwdFocus.hasFocus && _dirsOpen) setState(() => _dirsOpen = false);
      if (mounted) setState(() {});
    });
    if (_Memory.registry != null) {
      _m.selectDefaults();
      _m.normalize();
      _loaded = true;
    }
    unawaited(_open());
  }

  @override
  void dispose() {
    _typing?.cancel();
    _toastTimer?.cancel();
    _cwd.dispose();
    _cwdFocus.dispose();
    super.dispose();
  }

  // ---- Data ----------------------------------------------------------------

  /// `open()`: fresh `/providers` on every opening (an account added with the
  /// dashboard open must show), recent folders, the active plan, then browse.
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

    // `#ns-dirs`: the folders of the live sessions; the selected one fills an
    // empty field.
    final Object? list = await live;
    final List<Map<String, Object?>> items = <Map<String, Object?>>[
      if (list is List)
        for (final Object? it in list)
          if (asMap(it) != null) asMap(it)!,
    ];
    _recent = WizardPaths.recentDirs(<String>[for (final Map<String, Object?> it in items) readString(it, 'cwd')]);
    final Map<String, Object?>? tab = asMap(await active);
    if (_cwd.text.trim().isEmpty && tab != null) {
      final String s = readString(tab, 'session');
      final String p = readString(tab, 'pane');
      String? cwd;
      for (final Map<String, Object?> it in items) {
        if (readString(it, 'session') == s && (p.isEmpty || readString(it, 'pane') == p)) {
          cwd = readStringOrNull(it, 'cwd');
          if (cwd != null) break;
        }
      }
      if (cwd != null) _cwd.text = cwd;
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
    final String path = _cwd.text.trim();
    unawaited(_browse(path.isEmpty ? '~/codebase' : path));
  }

  /// `browse(path)`: crumbs, child folders and the exists / missing state.
  /// Only the newest request paints.
  Future<void> _browse(String path) async {
    final ComandosApi? api = _api;
    if (api == null) return;
    final int seq = ++_fsSeq;
    Object? d;
    try {
      d = await api.getJson('/fs/dirs', query: <String, String>{'path': path.isEmpty ? '~' : path});
    } on Object {
      return;
    }
    final Map<String, Object?>? m = asMap(d);
    if (!mounted || seq != _fsSeq || m == null || (m['error'] != null && m['error'] != false && m['error'] != '')) {
      return;
    }
    setState(() {
      _fsShown = true;
      _fsHome = readStringOrNull(m, 'home') ?? _fsHome;
      _fsBase = readString(m, 'base');
      _fsPath = readString(m, 'path');
      _fsExists = m['exists'] == true;
      _fsDirs = <(String, String)>[
        for (final Map<String, Object?> x in readMapList(m, 'dirs')) (readString(x, 'name'), readString(x, 'path')),
      ];
    });
  }

  void _onTyped(String _) {
    _typing?.cancel();
    _typing = Timer(const Duration(milliseconds: 220), () {
      final String p = _cwd.text.trim();
      unawaited(_browse(p.isEmpty ? '~' : p));
    });
    final bool any = _filteredRecent.isNotEmpty;
    if (_dirsOpen != any) setState(() => _dirsOpen = any);
  }

  void _goTo(String path) {
    _cwd.text = path;
    _cwd.selection = TextSelection.collapsed(offset: path.length);
    setState(() => _dirsOpen = false);
    unawaited(_browse(path));
  }

  List<String> get _filteredRecent {
    final String q = _cwd.text.trim().toLowerCase();
    if (q.isEmpty) return _recent;
    final List<String> hit = <String>[
      for (final String d in _recent)
        if (d.toLowerCase().contains(q) && d != _cwd.text.trim()) d,
    ];
    return hit;
  }

  Future<void> _mkdir() async {
    final ComandosApi? api = _api;
    if (api == null) return;
    try {
      final Map<String, Object?> r = await api.postForm('/fs/mkdir', <String, Object?>{'path': _fsPath});
      final String path = readString(r, 'path');
      _showToast('Carpeta creada: ${WizardPaths.shortHome(path, _fsHome)}');
      _cwd.text = path;
      unawaited(_browse(path));
    } on Object catch (e) {
      _showToast(errorText(e), err: true);
    }
  }

  /// `go()`; [shell] is «Solo terminal» (plain shell, the agent choice kept).
  Future<void> _create({bool shell = false}) async {
    if (_creating) return;
    final String cwd = _cwd.text.trim();
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
      if (message.contains('no existe')) unawaited(_browse(cwd));
      _showToast(WizardPaths.createErrorText(message), err: true);
    }
  }

  Future<void> _chooseProfile() async {
    final ComandosApi? api = _api;
    if (api == null) return;
    final String? id = await showProfilePicker(
      context,
      harness: _m.sel.harness,
      account: _m.sel.harnessAccount,
      cwd: _cwd.text.trim(),
    );
    if (id == null || !mounted) return;
    try {
      final Map<String, Object?> r = await api.postForm(
        '/session-profile-apply',
        <String, Object?>{'profileId': id, 'cwd': _cwd.text.trim()},
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

  // ---- UI ------------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final Size screen = MediaQuery.sizeOf(context);
    final EdgeInsets insets = MediaQuery.viewInsetsOf(context);
    final double width = math.min(520, screen.width * .94);
    final double maxHeight = (screen.height - insets.bottom) * .9;
    return Stack(
      children: <Widget>[
        // backdrop-filter: blur(3px); taps fall through to the barrier.
        Positioned.fill(
          child: IgnorePointer(
            child: BackdropFilter(filter: ui.ImageFilter.blur(sigmaX: 3, sigmaY: 3), child: const SizedBox.expand()),
          ),
        ),
        Padding(
          padding: EdgeInsets.only(bottom: insets.bottom),
          child: Center(
            child: Material(
              type: MaterialType.transparency,
              child: Container(
                width: width,
                constraints: BoxConstraints(maxHeight: maxHeight),
                decoration: BoxDecoration(
                  color: Orbita.panel,
                  border: Border.all(color: Orbita.line2),
                  borderRadius: BorderRadius.circular(16),
                  boxShadow: const <BoxShadow>[BoxShadow(color: Color(0xAA000000), offset: Offset(0, 18), blurRadius: 50)],
                ),
                child: ClipRRect(
                  borderRadius: BorderRadius.circular(15),
                  child: SingleChildScrollView(
                    padding: const EdgeInsets.fromLTRB(24, 22, 24, 22),
                    child: DefaultTextStyle(
                      style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.text),
                      child: _card(),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
        if (_toast != null)
          Positioned(
            right: 12,
            top: MediaQuery.paddingOf(context).top + 50,
            child: WizardToast(text: _toast!, error: _toastErr),
          ),
      ],
    );
  }

  Widget _card() {
    final List<Widget> rows = <Widget>[
      const Padding(
        padding: EdgeInsets.only(bottom: 4),
        child: Text('Nueva sesión', style: TextStyle(fontWeight: FontWeight.w700, fontSize: 15)),
      ),
      const NsLabel('Carpeta del proyecto'),
      _cwdField(),
      _fs(),
    ];
    if (_loadError != null && !_loaded) {
      rows.add(Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Text(_loadError!, style: const TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.waiting)),
      ));
    } else if (!_loaded) {
      rows.add(const Padding(
        padding: EdgeInsets.only(top: 6),
        child: Text('Cargando…', style: TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.faint)),
      ));
    } else {
      rows.addAll(_choices());
    }
    rows.addAll(<Widget>[
      _dangerRow(),
      Container(
        constraints: const BoxConstraints(minHeight: 14),
        child: Text(_loaded ? _m.note : '', style: nsNoteStyle),
      ),
      _actions(),
    ]);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        for (int i = 0; i < rows.length; i++) ...<Widget>[if (i > 0) const SizedBox(height: 8), rows[i]],
      ],
    );
  }

  Widget _cwdField() {
    final bool focused = _cwdFocus.hasFocus;
    final List<String> options = _filteredRecent;
    final Widget field = Container(
      decoration: BoxDecoration(
        color: Orbita.bg,
        border: Border.all(color: focused ? Orbita.brand : Orbita.line),
        borderRadius: BorderRadius.circular(10),
      ),
      padding: const EdgeInsets.only(left: 12),
      child: Row(
        children: <Widget>[
          Expanded(
            child: TextField(
              key: const ValueKey<String>('ns-cwd'),
              controller: _cwd,
              focusNode: _cwdFocus,
              autocorrect: false,
              enableSuggestions: false,
              keyboardType: TextInputType.url,
              onChanged: _onTyped,
              onSubmitted: (String v) => _goTo(v.trim()),
              style: const TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.text),
              cursorColor: Orbita.brand,
              decoration: const InputDecoration(
                isCollapsed: true,
                filled: false,
                border: InputBorder.none,
                enabledBorder: InputBorder.none,
                focusedBorder: InputBorder.none,
                contentPadding: EdgeInsets.symmetric(vertical: 10),
                hintText: '~/codebase/...',
                hintStyle: TextStyle(fontFamily: Orbita.mono, fontSize: 13, color: Orbita.faint),
              ),
            ),
          ),
          // The datalist's drop-down arrow (Chrome on Android).
          if (_recent.isNotEmpty)
            GestureDetector(
              key: const ValueKey<String>('ns-dirs-toggle'),
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(() => _dirsOpen = !_dirsOpen),
              child: const SizedBox(
                width: 28,
                height: 40,
                child: Center(child: Text('▼', style: TextStyle(fontSize: 11, color: Orbita.text))),
              ),
            )
          else
            const SizedBox(width: 12),
        ],
      ),
    );
    if (!_dirsOpen || options.isEmpty) return field;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        field,
        const SizedBox(height: 4),
        RecentDirsList(dirs: options, onPick: _goTo),
      ],
    );
  }

  Widget _fs() {
    if (!_fsShown) return const SizedBox(height: 6);
    final List<(String, String)> crumbs = WizardPaths.crumbs(_fsBase);
    return Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Wrap(
            crossAxisAlignment: WrapCrossAlignment.center,
            spacing: 1,
            runSpacing: 2,
            children: <Widget>[
              for (int i = 0; i < crumbs.length; i++) ...<Widget>[
                if (i > 0) const Text('/', style: TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.line2)),
                NsCrumb(label: crumbs[i].$1, last: i == crumbs.length - 1, onTap: () => _goTo(crumbs[i].$2)),
              ],
            ],
          ),
          const SizedBox(height: 6),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 132),
            child: SingleChildScrollView(
              child: _fsDirs.isEmpty
                  ? const Text('sin subcarpetas', style: TextStyle(fontSize: 11, color: Orbita.faint))
                  : Wrap(
                      spacing: 5,
                      runSpacing: 5,
                      children: <Widget>[
                        for (final (String name, String path) in _fsDirs)
                          NsDir(name: name, onTap: () => _goTo(path)),
                      ],
                    ),
            ),
          ),
          const SizedBox(height: 6),
          ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 24),
            child: _fsExists ? const FsOk() : _fsMissing(),
          ),
        ],
      ),
    );
  }

  Widget _fsMissing() => Wrap(
        spacing: 8,
        runSpacing: 6,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: <Widget>[
          Row(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              const WebIcon(WebIcons.sparkles, size: 11, color: Orbita.waiting),
              const SizedBox(width: 4),
              Flexible(
                child: Text(
                  '${WizardPaths.shortHome(_fsPath, _fsHome)} no existe',
                  style: const TextStyle(fontSize: 11, color: Orbita.waiting),
                ),
              ),
            ],
          ),
          NsTestButton(
            key: const ValueKey<String>('ns-mkdir'),
            tooltip: 'Crear la carpeta (y las intermedias) dentro de tu HOME',
            onTap: _mkdir,
            child: const Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                WebIcon(WebIcons.plus, size: 11),
                SizedBox(width: 4),
                Text('Crear carpeta'),
              ],
            ),
          ),
        ],
      );

  List<Widget> _choices() {
    final WizardModel m = _m;
    final List<Widget> out = <Widget>[
      const NsLabel('Harness'),
      NsChips(
        key: const ValueKey<String>('ns-harness'),
        items: m.harnessChips,
        current: m.sel.harness,
        onPick: (String id) => setState(() => m.pickHarness(id)),
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
            onPick: (String id) => setState(() => m.pickMotor(id)),
          ),
        ))
        ..add(NsWrap(
          label: 'Modelo',
          child: NsChips(
            key: const ValueKey<String>('ns-model'),
            items: m.modelChips,
            current: m.sel.model,
            onPick: (String id) => setState(() => m.pickModel(id)),
          ),
        ))
        ..add(NsWrap(
          label: 'Pensamiento',
          child: NsChips(
            key: const ValueKey<String>('ns-effort'),
            items: m.effortChips,
            current: m.sel.effort,
            onPick: (String id) => setState(() => m.pickEffort(id)),
          ),
        ));
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
            const Text(WizardModel.motorAccountNote, style: nsNoteStyle),
          ],
        ),
      ));
    }
    if (m.showUnavailable) {
      out.add(BlockedRoutes(
        summary: m.unavailableSummary,
        rows: m.blockedRoutes,
        open: _unavailableOpen,
        onToggle: () => setState(() => _unavailableOpen = !_unavailableOpen),
      ));
    }
    return out;
  }

  Widget _dangerRow() => Padding(
        padding: const EdgeInsets.only(top: 8),
        child: Semantics(
          toggled: _Memory.danger,
          label: 'sin aprobaciones (dangerously)',
          child: GestureDetector(
            key: const ValueKey<String>('ns-danger'),
            behavior: HitTestBehavior.opaque,
            onTap: () => setState(() => _Memory.danger = !_Memory.danger),
            child: ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 44),
              child: Row(
                children: <Widget>[
                  Tooltip(
                    message: 'Apagado: el agente pide permiso antes de acciones riesgosas. '
                        'Enciéndelo solo si confías en la tarea.',
                    excludeFromSemantics: true,
                    child: NsSwitch(on: _Memory.danger),
                  ),
                  const SizedBox(width: 8),
                  const Flexible(
                    child: Text(
                      'sin aprobaciones (dangerously)',
                      style: TextStyle(fontFamily: Orbita.mono, fontSize: 11, color: Orbita.dim),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      );

  Widget _actions() {
    final bool canCreate = _loaded && _m.canCreate && !_creating;
    return Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Wrap(
        alignment: WrapAlignment.end,
        spacing: 8,
        runSpacing: 8,
        children: <Widget>[
          NsActionButton(
            key: const ValueKey<String>('ns-cancel'),
            label: 'Cancelar',
            onTap: () => Navigator.of(context).pop(),
          ),
          SizedBox(
            width: double.infinity,
            child: Text(
              _profileNote,
              style: const TextStyle(fontSize: 13, height: 1.5, color: Orbita.dim),
            ),
          ),
          NsActionButton(
            key: const ValueKey<String>('ns-profile-open'),
            label: 'Elegir perfil',
            onTap: _loaded ? _chooseProfile : null,
          ),
          NsActionButton(
            key: const ValueKey<String>('ns-profile-clear'),
            label: 'Quitar perfil',
            onTap: _clearProfile,
          ),
          NsActionButton(
            key: const ValueKey<String>('ns-term'),
            label: 'Solo terminal',
            tooltip: 'Abrir una terminal simple en esta carpeta, sin agente',
            onTap: _creating ? null : () => _create(shell: true),
          ),
          NsActionButton(
            key: const ValueKey<String>('ns-go'),
            label: 'Crear',
            primary: true,
            tooltip: _loaded ? _m.createTitle : null,
            onTap: canCreate ? _create : null,
          ),
        ],
      ),
    );
  }
}
