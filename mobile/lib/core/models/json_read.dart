/// Lenient readers for server JSON: wrong types become defaults instead of
/// crashing. The server is ours, but versions drift.
String readString(Map<String, Object?> m, String k, [String fallback = '']) {
  final Object? v = m[k];
  return v is String ? v : fallback;
}

String? readStringOrNull(Map<String, Object?> m, String k) {
  final Object? v = m[k];
  return v is String && v.isNotEmpty ? v : null;
}

double readDouble(Map<String, Object?> m, String k, [double fallback = 0]) {
  final Object? v = m[k];
  return v is num ? v.toDouble() : fallback;
}

int readInt(Map<String, Object?> m, String k, [int fallback = 0]) {
  final Object? v = m[k];
  return v is num ? v.toInt() : fallback;
}

bool readBool(Map<String, Object?> m, String k, [bool fallback = false]) {
  final Object? v = m[k];
  return v is bool ? v : fallback;
}

Map<String, Object?>? asMap(Object? v) => v is Map ? v.cast<String, Object?>() : null;

List<Map<String, Object?>> readMapList(Map<String, Object?> m, String k) {
  final Object? v = m[k];
  if (v is! List) return const <Map<String, Object?>>[];
  return <Map<String, Object?>>[
    for (final Object? e in v)
      if (e is Map) e.cast<String, Object?>(),
  ];
}
