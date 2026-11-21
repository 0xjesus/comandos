import 'api_errors.dart';

// Pure Dart (no Flutter imports) so `tool/live_check.dart` can use it.

/// A decoded HTTP answer. [json] is null for an empty body; [malformed] is
/// true when the body was present but not JSON.
class ApiResponse {
  const ApiResponse(this.status, this.json, {this.malformed = false});

  final int status;
  final Object? json;
  final bool malformed;

  bool get ok => status >= 200 && status < 300;

  Map<String, Object?> get map =>
      json is Map ? (json! as Map).cast<String, Object?>() : const <String, Object?>{};

  /// Machine code in `{"code": …}`, if any.
  String? get code {
    final Object? c = map['code'];
    return c is String ? c : null;
  }
}

/// Maps a non-2xx answer to a typed [ApiException].
ApiException exceptionFor(ApiResponse r) {
  final int s = r.status;
  if (s == 401) return const UnauthorizedException();
  if (s == 403) return const HostForbiddenException();
  if (s == 404) return NotFoundException(code: r.code);
  if (s == 409 || s == 429) return BusyException(status: s);
  if (s >= 500) return ServerException(status: s);
  return BadResponseException(status: s);
}
