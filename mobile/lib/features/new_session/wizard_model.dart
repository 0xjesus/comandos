import '../../core/models/provider_registry.dart';

// Pure Dart port of the «Nueva sesión» wizard rules of the web page:
// crates/comandos-web/src/components/foundation_helpers_wizard.rs (route,
// model_list, model, account_items, ensure_account, select_defaults,
// selectable, render, go) and SessionConfig.validate (session_config.rs) for
// the profile check. Comments name the Rust function each piece copies.

/// One chip of a wizard row.
class WizardChip {
  const WizardChip({
    required this.id,
    required this.label,
    this.icon = '',
    this.model = '',
    this.disabled = false,
    this.title = '',
    this.tag = '',
  });

  final String id;
  final String label;

  /// The model's catalog note («nuevo»), shown as a small badge.
  final String tag;

  /// Agent/icon id for the provider mark (empty: no mark).
  final String icon;

  /// Model id for the provider mark (model chips).
  final String model;
  final bool disabled;
  final String title;

  @override
  bool operator ==(Object other) =>
      other is WizardChip &&
      other.id == id &&
      other.label == label &&
      other.icon == icon &&
      other.model == model &&
      other.disabled == disabled &&
      other.title == title &&
      other.tag == tag;

  @override
  int get hashCode => Object.hash(id, label, icon, model, disabled, title, tag);

  @override
  String toString() => 'WizardChip($id${disabled ? ', disabled' : ''})';
}

/// One row of the «N combinaciones listas · M bloqueadas» list.
class BlockedRoute {
  const BlockedRoute(this.title, this.reason);

  /// «Codex CLI → Claude».
  final String title;
  final String reason;
}

/// The wizard selection: the web's `NS` global. It survives between openings
/// (a static in the widget) exactly like the web keeps `NS`.
class WizardSelection {
  WizardSelection({
    this.harness = 'codex',
    this.motor = 'codex',
    this.routeId = 'codex:codex',
    this.model = 'gpt-6.1-sol',
    this.effort = 'high',
    this.harnessAccount = 'main',
    this.motorAccount = 'main',
    this.profileId,
  });

  /// `ns_defaults()` of foundation_helpers_view.rs.
  factory WizardSelection.defaults() => WizardSelection();

  String harness;
  String motor;
  String routeId;
  String model;
  String effort;
  String harnessAccount;
  String motorAccount;
  String? profileId;

  bool get hasProfile => profileId != null && profileId!.isNotEmpty;

  WizardSelection copy() => WizardSelection(
        harness: harness,
        motor: motor,
        routeId: routeId,
        model: model,
        effort: effort,
        harnessAccount: harnessAccount,
        motorAccount: motorAccount,
        profileId: profileId,
      );
}

/// The wizard rules over one [ProviderRegistry] and one [WizardSelection].
/// Every `pick…` method mutates the selection exactly like the web's chip
/// callbacks, then [normalize] applies what the web's `render()` writes back.
class WizardModel {
  WizardModel(this.registry, this.sel);

  ProviderRegistry registry;
  final WizardSelection sel;

  bool get isShell => sel.harness == 'shell';

  HarnessInfo? harnessInfo(String id) => registry.harnesses[id];
  MotorInfo? motorInfo(String id) => registry.motors[id];

  /// `route(h, m)`.
  MatrixRoute? route([String? h, String? m]) {
    final String hh = h ?? sel.harness;
    final String mm = m ?? sel.motor;
    for (final MatrixRoute r in registry.matrix) {
      if (r.harness == hh && r.motor == mm) return r;
    }
    return null;
  }

  /// `model_list()`.
  List<MotorModel> get modelList => motorInfo(sel.motor)?.models ?? const <MotorModel>[];

  /// `model()`: the chosen model, else the first, else an empty one.
  MotorModel get model {
    final List<MotorModel> list = modelList;
    for (final MotorModel m in list) {
      if (m.id == sel.model) return m;
    }
    return list.isNotEmpty ? list.first : const MotorModel(id: '');
  }

  /// `account_items(provider, role)`.
  List<WizardChip> accountItems(String provider, String role) {
    final bool acp = role == 'motor' && route()?.driver == 'acp';
    final List<ProviderAccount> accounts = harnessInfo(provider)?.accounts ?? const <ProviderAccount>[];
    return <WizardChip>[
      for (final ProviderAccount a in accounts)
        () {
          final bool enabled = (role == 'motor' && !acp) ? a.motorSelectable : a.selectable;
          final String title;
          if (a.identity.isNotEmpty) {
            title = a.identity;
          } else if (!enabled && a.selectable) {
            title = 'cuenta de motor nombrada: gateway aislado aún no disponible';
          } else {
            title = switch (a.state) {
              'ready' => 'suscripción lista',
              'login_required' => 'requiere login',
              'unsupported_auth' => 'auth no compatible con suscripción',
              _ => a.state,
            };
          }
          return WizardChip(id: a.alias, label: a.alias, title: title, disabled: !enabled, icon: provider);
        }(),
    ];
  }

  /// `ensure_account(provider, field, role)`: keeps the chosen account when it
  /// is usable, otherwise picks the first usable one (or «main»). A profile
  /// keeps whatever it brought.
  void ensureAccount(String provider, String field, String role) {
    if (sel.hasProfile) return;
    final List<WizardChip> ready = <WizardChip>[
      for (final WizardChip a in accountItems(provider, role))
        if (!a.disabled) a,
    ];
    final String current = field == 'motorAccount' ? sel.motorAccount : sel.harnessAccount;
    if (!ready.any((WizardChip a) => a.id == current)) {
      final String next = ready.isNotEmpty && ready.first.id.isNotEmpty ? ready.first.id : 'main';
      if (field == 'motorAccount') {
        sel.motorAccount = next;
      } else {
        sel.harnessAccount = next;
      }
    }
  }

  /// `select_defaults()`: route id, model and effort follow the motor.
  void selectDefaults() {
    final MatrixRoute? r = route();
    sel.routeId = r != null && r.selectable ? r.id : '';
    final List<MotorModel> list = modelList;
    if ((!sel.hasProfile || sel.model.isEmpty) && !list.any((MotorModel m) => m.id == sel.model)) {
      sel.model = list.isNotEmpty ? list.first.id : '';
    }
    final MotorModel m = model;
    if ((!sel.hasProfile || sel.effort.isEmpty) && !m.efforts.contains(sel.effort)) {
      sel.effort = m.defaultEffort.isNotEmpty ? m.defaultEffort : (m.efforts.isNotEmpty ? m.efforts.first : '');
    }
  }

  /// `selectable(h)`: motors shown for a harness. Routes the harness can never
  /// host (`not_routed`) are hidden; blocked ones show disabled.
  List<MatrixRoute> selectableRoutes(String h) => <MatrixRoute>[
        for (final MatrixRoute r in registry.matrix)
          if (r.harness == h && (r.selectable || r.reasonCode != 'not_routed')) r,
      ];

  // ---- Rows ----------------------------------------------------------------

  /// HARNESS chips: `matrixHarnesses` (or claude, codex, grok) plus Shell.
  List<WizardChip> get harnessChips {
    final List<String> ids = registry.matrixHarnesses ?? const <String>['claude', 'codex', 'grok'];
    return <WizardChip>[
      for (final String id in ids)
        WizardChip(id: id, label: harnessInfo(id)?.label ?? id, icon: harnessInfo(id)?.icon ?? ''),
      const WizardChip(id: 'shell', label: 'Shell', icon: 'terminal'),
    ];
  }

  /// MOTOR chips (hidden for Shell).
  List<WizardChip> get motorChips => <WizardChip>[
        for (final MatrixRoute r in selectableRoutes(sel.harness))
          WizardChip(
            id: r.motor,
            label: motorInfo(r.motor)?.label ?? r.motor,
            icon: motorInfo(r.motor)?.icon ?? '',
            disabled: !r.selectable,
            title: r.selectable ? '' : (r.reasonMessage ?? r.productState),
          ),
      ];

  /// MODELO chips.
  List<WizardChip> get modelChips => <WizardChip>[for (final MotorModel m in modelList) _modelChip(m)];

  WizardChip _modelChip(MotorModel m) =>
      WizardChip(id: m.id, label: m.name ?? m.id, model: m.id, icon: sel.motor, tag: m.tag, title: m.tag);

  /// Chips shown before «Buscar modelo»: a motor with a long catalog (OpenCode
  /// lists hundreds, newest first) shows its first [modelChipLimit].
  static const int modelChipLimit = 8;

  /// True when the motor's list is long enough for «Buscar modelo (N)».
  bool get hasModelSearch => modelList.length > modelChipLimit;

  /// The first [modelChipLimit] models, plus the chosen one when it is
  /// further down the list (the selection is always visible).
  List<WizardChip> get visibleModelChips {
    final List<WizardChip> all = modelChips;
    if (all.length <= modelChipLimit) return all;
    final List<WizardChip> out = all.sublist(0, modelChipLimit);
    if (!out.any((WizardChip c) => c.id == sel.model)) {
      for (final WizardChip c in all) {
        if (c.id == sel.model) out.add(c);
      }
    }
    return out;
  }

  /// «Buscar modelo»: every model whose id or name contains each word of
  /// [query] (case-insensitive), in catalog order.
  List<WizardChip> filterModels(String query) {
    final List<String> words = query.toLowerCase().split(RegExp(r'\s+')).where((String w) => w.isNotEmpty).toList();
    return <WizardChip>[
      for (final MotorModel m in modelList)
        if (() {
          final String hay = '${m.id} ${m.name ?? ''}'.toLowerCase();
          return words.every(hay.contains);
        }())
          _modelChip(m),
    ];
  }

  /// PENSAMIENTO chips.
  List<WizardChip> get effortChips => <WizardChip>[
        for (final String e in model.efforts) WizardChip(id: e, label: e),
      ];

  /// «Cuenta del harness» shows for harnesses with `capabilities.accounts`.
  bool get showHarnessAccounts => !isShell && (harnessInfo(sel.harness)?.accountsCapable ?? false);

  List<WizardChip> get harnessAccountChips => accountItems(sel.harness, 'harness');

  /// «Cuenta del motor»: the route bills a separate motor subscription.
  bool get showMotorAccounts {
    final MatrixRoute? r = route();
    return !isShell && r != null && r.accountScopes.contains('motor') && sel.motor != sel.harness;
  }

  List<WizardChip> get motorAccountChips => accountItems(sel.motor, 'motor');

  static const String motorAccountNote =
      'Esta suscripción paga la inferencia; la cuenta del harness conserva transcript y herramientas.';

  /// Routes that are blocked (login / gateway), excluding `not_routed`.
  List<MatrixRoute> get unavailable => <MatrixRoute>[
        for (final MatrixRoute r in registry.matrix)
          if (!r.selectable && r.reasonCode != 'not_routed') r,
      ];

  bool get showUnavailable => !isShell && unavailable.isNotEmpty;

  /// `N combinaciones listas · M bloqueadas (login / gateway)`.
  String get unavailableSummary {
    final int ready = registry.matrix.where((MatrixRoute r) => r.selectable).length;
    return '$ready combinaciones listas · ${unavailable.length} bloqueadas (login / gateway)';
  }

  List<BlockedRoute> get blockedRoutes => <BlockedRoute>[
        for (final MatrixRoute r in unavailable)
          BlockedRoute(
            '${harnessInfo(r.harness)?.label ?? r.harness} → ${motorInfo(r.motor)?.label ?? r.motor}',
            r.reasonMessage ?? r.productState,
          ),
      ];

  // ---- Choices (chip callbacks) --------------------------------------------

  void pickHarness(String h) {
    sel
      ..harness = h
      ..motor = h == 'shell' ? '' : h
      ..harnessAccount = 'main'
      ..motorAccount = 'main';
    selectDefaults();
    normalize();
  }

  void pickMotor(String id) {
    sel
      ..motor = id
      ..motorAccount = 'main';
    selectDefaults();
    normalize();
  }

  void pickModel(String id) {
    sel.model = id;
    selectDefaults();
    normalize();
  }

  /// The web does not re-render after an effort tap.
  void pickEffort(String id) => sel.effort = id;

  void pickHarnessAccount(String id) {
    sel.harnessAccount = id;
    if (sel.motor == sel.harness) sel.motorAccount = id;
    normalize();
  }

  void pickMotorAccount(String id) => sel.motorAccount = id;

  /// What the web's `render()` writes back into `NS` while drawing.
  void normalize() {
    if (showHarnessAccounts) ensureAccount(sel.harness, 'harnessAccount', 'harness');
    if (showMotorAccounts) {
      ensureAccount(sel.motor, 'motorAccount', 'motor');
    } else if (!isShell) {
      sel.motorAccount = sel.harnessAccount;
    }
    final MatrixRoute? r = route();
    sel.routeId = r != null && r.selectable ? r.id : '';
  }

  // ---- Footer --------------------------------------------------------------

  /// `SessionConfig.validate(PROVIDERS, NS + toHarness)` when a profile is
  /// chosen; empty when fine.
  String get profileError {
    if (!sel.hasProfile || isShell) return '';
    final MatrixRoute? r = route();
    if (r == null || !r.selectable) {
      final String? msg = r?.reasonMessage;
      return msg == null || msg.isEmpty ? 'Esta combinación no está disponible' : msg;
    }
    final List<MotorModel> models = modelList;
    MotorModel? chosen;
    for (final MotorModel m in models) {
      if (m.id == sel.model) chosen = m;
    }
    if (models.isNotEmpty && chosen == null) return 'Selecciona un modelo disponible';
    if (sel.effort.isNotEmpty && !(chosen?.efforts ?? const <String>[]).contains(sel.effort)) {
      return 'Selecciona un esfuerzo compatible';
    }
    final bool acp = sel.harness == 'acp';
    final String provider = acp ? sel.motor : sel.harness;
    final String account = acp ? sel.motorAccount : sel.harnessAccount;
    final List<ProviderAccount> accounts = harnessInfo(provider)?.accounts ?? const <ProviderAccount>[];
    if (accounts.isNotEmpty && !accounts.any((ProviderAccount a) => a.alias == account && a.selectable)) {
      return 'Selecciona una cuenta con sesión iniciada';
    }
    final List<ProviderAccount> motorAccounts = harnessInfo(sel.motor)?.accounts ?? const <ProviderAccount>[];
    if (sel.motor != sel.harness &&
        !acp &&
        motorAccounts.isNotEmpty &&
        !motorAccounts.any((ProviderAccount a) => a.alias == sel.motorAccount && a.motorSelectable)) {
      return 'Selecciona una cuenta compatible para el motor';
    }
    return '';
  }

  /// «Crear» is disabled on a profile error or a blocked combination.
  bool get canCreate => profileError.isEmpty && (isShell || sel.routeId.isNotEmpty);

  /// Tooltip of «Crear».
  String get createTitle => canCreate
      ? 'Crear la sesión con este setup'
      : 'Esta combinación no está disponible — elige un motor habilitado';

  /// `#ns-note`: «Codex CLI → Codex · gpt-6.1-sol · high».
  String get note {
    final String pe = profileError;
    if (pe.isNotEmpty) return pe;
    if (isShell) return '';
    if (sel.routeId.isEmpty) return 'combinación no disponible';
    final String h = harnessInfo(sel.harness)?.label ?? '';
    final String m = motorInfo(sel.motor)?.label ?? '';
    return '$h → $m · ${sel.model}${sel.effort.isEmpty ? '' : ' · ${sel.effort}'}';
  }

  /// Body of `POST /session-new` (`go()`). [shell] is «Solo terminal».
  Map<String, Object?> createBody({required String cwd, required bool danger, bool shell = false}) {
    final Map<String, Object?> body = shell || isShell
        ? <String, Object?>{'cwd': cwd, 'agent': 'shell'}
        : <String, Object?>{
            'cwd': cwd,
            'routeId': sel.routeId,
            'model': sel.model,
            'effort': sel.effort,
            'harnessAccount': sel.harnessAccount,
            'motorAccount': sel.motorAccount,
          };
    if (sel.hasProfile) body['profileId'] = sel.profileId;
    body['danger'] = danger;
    return body;
  }

  /// The active optimisation plan's variant for this harness (`open()`):
  /// motor from its route id, model and effort.
  void applyPlanVariant(Map<String, Object?>? variant) {
    if (variant == null || variant['selectable'] != true) return;
    final Object? routeId = variant['routeId'];
    if (routeId is String) {
      final List<String> parts = routeId.split(':');
      if (parts.length > 1 && parts[1].isNotEmpty) sel.motor = parts[1];
    }
    final Object? model = variant['model'];
    final Object? effort = variant['effort'];
    sel.model = model is String ? model : '';
    sel.effort = effort is String ? effort : '';
  }

  /// `use_profile`: `launchDraft` of `/session-profile-apply` plus its id.
  void applyProfileDraft(String profileId, Map<String, Object?> draft) {
    String? s(String k) {
      final Object? v = draft[k];
      return v is String ? v : null;
    }

    sel
      ..motor = s('motor') ?? sel.motor
      ..routeId = s('routeId') ?? sel.routeId
      ..model = s('model') ?? sel.model
      ..effort = s('effort') ?? sel.effort
      ..harnessAccount = s('harnessAccount') ?? sel.harnessAccount
      ..motorAccount = s('motorAccount') ?? sel.motorAccount
      ..profileId = profileId;
    for (final String k in <String>['agent', 'toHarness']) {
      final String? v = s(k);
      if (v != null && v.isNotEmpty) sel.harness = v;
    }
    selectDefaults();
    normalize();
  }
}

/// Folder helpers of the wizard (`short_home`, crumbs of `browse()`).
abstract final class WizardPaths {
  /// `~` for the home prefix (the web's `FS_HOME_RE` is `^<home>`).
  static String shortHome(String path, String? home) {
    if (home == null || home.isEmpty || !path.startsWith(home)) return path;
    return '~${path.substring(home.length)}';
  }

  /// `(label, absolute path)` for every segment of [base].
  static List<(String, String)> crumbs(String base) {
    final List<(String, String)> out = <(String, String)>[];
    String acc = '';
    for (final String p in base.split('/')) {
      if (p.isEmpty) continue;
      acc = '$acc/$p';
      out.add((p, acc));
    }
    return out;
  }

  /// «Recientes»: the folders of the live sessions (`/state?live=1` rows),
  /// once each, the most recently active (`ts`) first.
  static List<String> recentFolders(Iterable<Map<String, Object?>> rows) {
    final Map<String, double> last = <String, double>{};
    for (final Map<String, Object?> r in rows) {
      final Object? c = r['cwd'];
      if (c is! String || c.isEmpty) continue;
      final Object? t = r['ts'];
      final double ts = t is num ? t.toDouble() : 0;
      final double? seen = last[c];
      if (seen == null || ts > seen) last[c] = ts;
    }
    final List<String> order = last.keys.toList();
    final Map<String, int> first = <String, int>{for (int i = 0; i < order.length; i++) order[i]: i};
    order.sort((String a, String b) {
      final int byTs = last[b]!.compareTo(last[a]!);
      return byTs != 0 ? byTs : first[a]!.compareTo(first[b]!);
    });
    return order;
  }

  /// The last segment of [path] («~/codebase/x» → «x»; «/» stays).
  static String folderName(String path) {
    final String p = path.endsWith('/') && path.length > 1 ? path.substring(0, path.length - 1) : path;
    final int i = p.lastIndexOf('/');
    return i < 0 || i == p.length - 1 ? p : p.substring(i + 1);
  }

  /// Text that browses a path instead of searching: starts with `/` or `~`.
  static bool isPathQuery(String q) {
    final String t = q.trimLeft();
    return t.startsWith('/') || t.startsWith('~');
  }

  /// How well [q] matches the folder [name] (lower is better), or null:
  /// exact 0, prefix 1, word start 2, substring 3, letters in order 4.
  static int? matchScore(String q, String name) {
    final String qq = q.trim().toLowerCase();
    if (qq.isEmpty) return 0;
    final String n = name.toLowerCase();
    if (n == qq) return 0;
    if (n.startsWith(qq)) return 1;
    final int i = n.indexOf(qq);
    if (i > 0) return _wordChar(n.codeUnitAt(i - 1)) ? 3 : 2;
    return _subsequence(qq, n) == null ? null : 4;
  }

  static bool _wordChar(int c) =>
      (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) || c > 0x7F;

  /// Indexes of [q]'s letters found in order in [n], or null.
  static List<int>? _subsequence(String q, String n) {
    final List<int> at = <int>[];
    int j = 0;
    for (int i = 0; i < n.length && j < q.length; i++) {
      if (n.codeUnitAt(i) == q.codeUnitAt(j)) {
        at.add(i);
        j++;
      }
    }
    return j == q.length ? at : null;
  }

  /// [name] cut into `(text, matched)` runs for the highlight: the substring
  /// [q] when present, else its letters in order. Only the text after the last
  /// `/` of [q] counts («codebase/coma» highlights «coma»).
  static List<(String, bool)> highlight(String name, String q) {
    String qq = q.trim().toLowerCase();
    final int slash = qq.lastIndexOf('/');
    if (slash >= 0) qq = qq.substring(slash + 1);
    if (qq.isEmpty || name.isEmpty) return <(String, bool)>[(name, false)];
    final String n = name.toLowerCase();
    final Set<int> marked = <int>{};
    final int i = n.indexOf(qq);
    if (i >= 0) {
      for (int k = i; k < i + qq.length; k++) {
        marked.add(k);
      }
    } else {
      marked.addAll(_subsequence(qq, n) ?? const <int>[]);
    }
    if (marked.isEmpty || n.length != name.length) return <(String, bool)>[(name, false)];
    final List<(String, bool)> out = <(String, bool)>[];
    int start = 0;
    for (int k = 1; k <= name.length; k++) {
      if (k == name.length || marked.contains(k) != marked.contains(start)) {
        out.add((name.substring(start, k), marked.contains(start)));
        start = k;
      }
    }
    return out;
  }

  /// The toast text for an error of `POST /session-new` (`go()`): a missing
  /// folder points at the «Crear carpeta» button.
  static String createErrorText(String message) =>
      message.contains('no existe') ? 'Esa carpeta no existe — usa el botón Crear carpeta.' : message;
}
