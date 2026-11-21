import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'paired_server.dart';

/// Where the pairing (host + token) lives. The token only ever goes to the
/// Android Keystore-backed secure storage, never to plain preferences.
abstract class CredentialStore {
  Future<PairedServer?> load();
  Future<void> save(PairedServer server);
  Future<void> clear();
}

class SecureCredentialStore implements CredentialStore {
  SecureCredentialStore([FlutterSecureStorage? storage])
      : _storage = storage ?? const FlutterSecureStorage();

  static const String _key = 'comandos.pairing.v1';
  final FlutterSecureStorage _storage;

  @override
  Future<PairedServer?> load() async {
    try {
      final String? raw = await _storage.read(key: _key);
      if (raw == null) return null;
      return PairedServer.fromJson(jsonDecode(raw));
    } catch (_) {
      // Unreadable keystore entry (e.g. restored from another device):
      // behave as unpaired; the person pairs again.
      return null;
    }
  }

  @override
  Future<void> save(PairedServer server) =>
      _storage.write(key: _key, value: jsonEncode(server.toJson()));

  @override
  Future<void> clear() => _storage.delete(key: _key);
}

/// In-memory store for tests.
class MemoryCredentialStore implements CredentialStore {
  PairedServer? value;

  @override
  Future<PairedServer?> load() async => value;

  @override
  Future<void> save(PairedServer server) async => value = server;

  @override
  Future<void> clear() async => value = null;
}
