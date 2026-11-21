import 'json_read.dart';

// Pure Dart reader for `GET /providers` (registry version 2): harnesses,
// motors with their models, and the harness × motor matrix the «Nueva sesión»
// wizard filters with. Lenient like the other models: wrong types become
// defaults, unknown fields are ignored.

/// One login of a provider (`harnesses.<id>.accounts[]`).
class ProviderAccount {
  const ProviderAccount({
    required this.alias,
    this.identity = '',
    this.state = '',
    this.selectable = false,
    this.motorSelectable = false,
  });

  factory ProviderAccount.fromJson(Map<String, Object?> m) => ProviderAccount(
        alias: readString(m, 'alias'),
        identity: readString(m, 'identity'),
        state: readString(m, 'state'),
        selectable: m['selectable'] == true,
        motorSelectable: m['motorSelectable'] == true,
      );

  final String alias;
  final String identity;
  final String state;
  final bool selectable;
  final bool motorSelectable;
}

/// A CLI that hosts the conversation (`harnesses.<id>`).
class HarnessInfo {
  const HarnessInfo({
    required this.id,
    this.label,
    this.icon = '',
    this.accountsCapable = false,
    this.accounts = const <ProviderAccount>[],
  });

  factory HarnessInfo.fromJson(String id, Map<String, Object?> m) => HarnessInfo(
        id: id,
        label: readStringOrNull(m, 'label'),
        icon: readString(m, 'icon'),
        accountsCapable: asMap(m['capabilities'])?['accounts'] == true,
        accounts: <ProviderAccount>[
          for (final Map<String, Object?> a in readMapList(m, 'accounts')) ProviderAccount.fromJson(a),
        ],
      );

  final String id;

  /// Null when the registry has no label (the web then shows the id).
  final String? label;
  final String icon;

  /// `capabilities.accounts`: the wizard shows «Cuenta del harness».
  final bool accountsCapable;
  final List<ProviderAccount> accounts;
}

/// One model a motor offers.
class MotorModel {
  const MotorModel({
    required this.id,
    this.name,
    this.efforts = const <String>[],
    this.defaultEffort = '',
  });

  factory MotorModel.fromJson(Map<String, Object?> m) {
    final Object? e = m['efforts'];
    return MotorModel(
      id: readString(m, 'id'),
      name: readStringOrNull(m, 'name'),
      efforts: <String>[if (e is List) for (final Object? x in e) if (x is String) x],
      defaultEffort: readString(m, 'defaultEffort'),
    );
  }

  final String id;
  final String? name;
  final List<String> efforts;
  final String defaultEffort;
}

/// The provider that runs inference (`motors.<id>`).
class MotorInfo {
  const MotorInfo({required this.id, this.label, this.icon = '', this.models = const <MotorModel>[]});

  factory MotorInfo.fromJson(String id, Map<String, Object?> m) => MotorInfo(
        id: id,
        label: readStringOrNull(m, 'label'),
        icon: readString(m, 'icon'),
        models: <MotorModel>[for (final Map<String, Object?> x in readMapList(m, 'models')) MotorModel.fromJson(x)],
      );

  final String id;
  final String? label;
  final String icon;
  final List<MotorModel> models;
}

/// One harness → motor combination (`matrix[]`).
class MatrixRoute {
  const MatrixRoute({
    required this.id,
    required this.harness,
    required this.motor,
    this.selectable = false,
    this.reasonCode,
    this.reasonMessage,
    this.productState = '',
    this.driver = '',
    this.accountScopes = const <String>[],
  });

  factory MatrixRoute.fromJson(Map<String, Object?> m) {
    final Map<String, Object?>? reason = asMap(m['reason']);
    final Object? scopes = m['accountScopes'];
    return MatrixRoute(
      id: readString(m, 'id'),
      harness: readString(m, 'harness'),
      motor: readString(m, 'motor'),
      selectable: m['selectable'] == true,
      reasonCode: reason == null ? null : readStringOrNull(reason, 'code'),
      reasonMessage: reason == null ? null : readStringOrNull(reason, 'message'),
      productState: readString(m, 'productState'),
      driver: readString(m, 'driver'),
      accountScopes: <String>[if (scopes is List) for (final Object? s in scopes) if (s is String) s],
    );
  }

  final String id;
  final String harness;
  final String motor;
  final bool selectable;
  final String? reasonCode;
  final String? reasonMessage;
  final String productState;
  final String driver;
  final List<String> accountScopes;
}

/// The whole `/providers` answer the wizard needs.
class ProviderRegistry {
  const ProviderRegistry({
    this.harnesses = const <String, HarnessInfo>{},
    this.motors = const <String, MotorInfo>{},
    this.matrix = const <MatrixRoute>[],
    this.matrixHarnesses,
  });

  /// Null when [json] is not an object.
  static ProviderRegistry? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null) return null;
    final Map<String, Object?> h = asMap(m['harnesses']) ?? const <String, Object?>{};
    final Map<String, Object?> mo = asMap(m['motors']) ?? const <String, Object?>{};
    final Object? mh = m['matrixHarnesses'];
    return ProviderRegistry(
      harnesses: <String, HarnessInfo>{
        for (final MapEntry<String, Object?> e in h.entries)
          if (asMap(e.value) != null) e.key: HarnessInfo.fromJson(e.key, asMap(e.value)!),
      },
      motors: <String, MotorInfo>{
        for (final MapEntry<String, Object?> e in mo.entries)
          if (asMap(e.value) != null) e.key: MotorInfo.fromJson(e.key, asMap(e.value)!),
      },
      matrix: <MatrixRoute>[for (final Map<String, Object?> r in readMapList(m, 'matrix')) MatrixRoute.fromJson(r)],
      matrixHarnesses: mh is List ? <String>[for (final Object? x in mh) if (x is String) x] : null,
    );
  }

  final Map<String, HarnessInfo> harnesses;
  final Map<String, MotorInfo> motors;
  final List<MatrixRoute> matrix;

  /// Order of the HARNESS chips; null when the server does not send it.
  final List<String>? matrixHarnesses;
}
