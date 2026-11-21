import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// What the UI may know about the key: never the key itself, only whether
/// there is one and its last four characters.
@immutable
class KeyStatus {
  const KeyStatus._(this.loaded, this.suffix);

  static const KeyStatus unknown = KeyStatus._(false, null);
  static const KeyStatus none = KeyStatus._(true, null);
  const KeyStatus.present(String suffix) : this._(true, suffix);

  final bool loaded;
  final String? suffix;
  bool get present => suffix != null;

  @override
  bool operator ==(Object other) => other is KeyStatus && other.loaded == loaded && other.suffix == suffix;
  @override
  int get hashCode => Object.hash(loaded, suffix);
}

/// Finds an OpenAI key in pasted / scanned text: trims whitespace and
/// newlines, tolerates `OPENAI_API_KEY=...`, quotes, etc. Null when there is
/// not exactly one plausible key.
String? extractOpenAiKey(String raw) {
  final String compact = raw.replaceAll(RegExp(r'\s+'), ' ').trim();
  final List<String> found = RegExp(r'sk-[A-Za-z0-9_\-]{16,}')
      .allMatches(compact)
      .map((Match m) => m[0]!)
      .toSet()
      .toList();
  return found.length == 1 ? found.single : null;
}

/// Minimal key/value interface so tests do not need the platform plugin.
abstract class SecretStore {
  Future<String?> read(String key);
  Future<void> write(String key, String value);
  Future<void> delete(String key);
}

class PlatformSecretStore implements SecretStore {
  const PlatformSecretStore([this._s = const FlutterSecureStorage()]);
  final FlutterSecureStorage _s;
  @override
  Future<String?> read(String key) => _s.read(key: key);
  @override
  Future<void> write(String key, String value) => _s.write(key: key, value: value);
  @override
  Future<void> delete(String key) => _s.delete(key: key);
}

class MemorySecretStore implements SecretStore {
  final Map<String, String> values = <String, String>{};
  @override
  Future<String?> read(String key) async => values[key];
  @override
  Future<void> write(String key, String value) async => values[key] = value;
  @override
  Future<void> delete(String key) async => values.remove(key);
}

/// The OpenAI key, kept only in the Android Keystore-backed secure storage
/// (and in memory while the app runs). Never logged, never shown.
class OpenAiKeyStore {
  OpenAiKeyStore([SecretStore? store]) : _store = store ?? const PlatformSecretStore();

  static const String _name = 'comandos.openai.v1';
  final SecretStore _store;
  String? _key;
  Future<void>? _loading;

  final ValueNotifier<KeyStatus> status = ValueNotifier<KeyStatus>(KeyStatus.unknown);

  Future<void> load() => _loading ??= _load();

  Future<void> _load() async {
    try {
      final String? k = await _store.read(_name);
      _key = (k == null || k.isEmpty) ? null : k;
    } catch (_) {
      _key = null; // no plugin (tests) or unreadable keystore entry
    }
    status.value = _statusOf(_key);
  }

  static KeyStatus _statusOf(String? k) =>
      k == null ? KeyStatus.none : KeyStatus.present(k.substring(k.length - 4));

  /// The key for a request to api.openai.com. Null when there is none.
  Future<String?> read() async {
    await load();
    return _key;
  }

  /// Stores a key already normalised by [extractOpenAiKey].
  Future<void> save(String key) async {
    await load();
    await _store.write(_name, key);
    _key = key;
    status.value = _statusOf(key);
  }

  Future<void> clear() async {
    await load();
    await _store.delete(_name);
    _key = null;
    status.value = KeyStatus.none;
  }
}
