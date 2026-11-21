import '../../core/api/api_client.dart';
import '../../core/api/api_errors.dart';

/// The routes the «Nueva sesión» wizard and the «Consumos» summary use, on top
/// of the shared client (kept here, not in core/api, by agreement).
extension WizardApi on ComandosApi {
  /// GET returning the decoded JSON, or throwing a typed [ApiException].
  Future<Object?> getJson(String path, {Map<String, String>? query}) async {
    final ApiResponse r = await requestOk('GET', path, query: query);
    if (r.malformed) throw const BadResponseException();
    return r.json;
  }

  /// POST whose failures carry the server's own Spanish `error` text, as the
  /// web shows it in its toast. 401/403 keep the app's copy (re-pair).
  Future<Map<String, Object?>> postForm(String path, Map<String, Object?> body) async {
    final ApiResponse r = await request('POST', path, body: body, timeout: ApiTimeouts.send);
    if (r.ok) return r.map;
    if (r.status == 401 || r.status == 403) throw exceptionFor(r);
    final Object? e = r.map['error'];
    if (e is String && e.isNotEmpty) {
      throw ServerMessage(e, status: r.status, code: r.code);
    }
    throw exceptionFor(r);
  }
}

/// A rejection whose text comes from the server (`{"error": …}`), e.g.
/// «Esa carpeta no existe todavía.» (409 `cwd_missing`).
class ServerMessage implements Exception {
  const ServerMessage(this.message, {this.status, this.code});

  final String message;
  final int? status;
  final String? code;

  @override
  String toString() => 'ServerMessage(${status ?? '-'}, ${code ?? '-'}): $message';
}

/// Text to show for any failure of these calls.
String errorText(Object e) => switch (e) {
      ServerMessage(:final String message) => message,
      ApiException(:final String userMessage) => userMessage,
      _ => 'Respuesta inesperada de tu equipo.',
    };
