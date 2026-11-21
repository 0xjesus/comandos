import 'package:flutter/foundation.dart';

import 'api/api_client.dart';
import 'config/credential_store.dart';
import 'config/paired_server.dart';

/// The current pairing and its API client. Listen to it to react to
/// pair / unpair. There is at most one live [ComandosApi] at a time.
class Connection extends ChangeNotifier {
  Connection(this._store, {ComandosApi Function(PairedServer server)? apiFactory})
      : _apiFactory = apiFactory ?? ComandosApi.new;

  final CredentialStore _store;
  final ComandosApi Function(PairedServer server) _apiFactory;

  PairedServer? _server;
  ComandosApi? _api;

  PairedServer? get server => _server;
  ComandosApi? get api => _api;
  bool get isPaired => _server != null;

  /// Set when any request got 401: the token was rotated. Cleared on pair.
  final ValueNotifier<bool> tokenRejected = ValueNotifier<bool>(false);

  Future<void> load() async {
    _server = await _store.load();
    if (_server != null) _api = _apiFactory(_server!);
  }

  /// Saves the pairing (token to secure storage) and swaps the client.
  Future<void> pair(PairedServer server) async {
    await _store.save(server);
    _api?.close();
    _server = server;
    _api = _apiFactory(server);
    tokenRejected.value = false;
    notifyListeners();
  }

  /// Forgets the token. Outbox items are kept (held until paired again).
  Future<void> unpair() async {
    await _store.clear();
    _api?.close();
    _api = null;
    _server = null;
    notifyListeners();
  }
}
