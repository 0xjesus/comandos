/// Typed failures of the ComandOS API. Each carries Spanish copy that says
/// what happened and what to do; screens show [userMessage] as is.
sealed class ApiException implements Exception {
  const ApiException(this.userMessage, {this.status});

  final String userMessage;
  final int? status;

  /// True when retrying later may succeed without the person doing anything.
  bool get isTransient => false;

  @override
  String toString() => '$runtimeType(${status ?? '-'}): $userMessage';
}

/// 401: the token was rejected (rotated, or the pairing is stale).
class UnauthorizedException extends ApiException {
  const UnauthorizedException()
      : super('Tu equipo rechazó el token. Vuelve a vincular el teléfono en Ajustes.',
            status: 401);
}

/// 403: the server does not accept this Host (IP instead of MagicDNS name).
class HostForbiddenException extends ApiException {
  const HostForbiddenException()
      : super('Tu equipo no acepta esta dirección. Vincula con el nombre '
            'MagicDNS (…ts.net), no con una IP.',
            status: 403);
}

/// 404 with an optional machine [code] (`pane_gone`, `session_gone`…).
class NotFoundException extends ApiException {
  const NotFoundException({this.code, String? message})
      : super(message ?? 'Esa sesión o panel ya no existe en tu equipo.', status: 404);

  final String? code;
}

/// 409 / 429: the server is busy with this target; retry shortly.
class BusyException extends ApiException {
  const BusyException({required int status})
      : super('Tu equipo está ocupado con esa sesión. Reintento en un momento.',
            status: status);

  @override
  bool get isTransient => true;
}

/// 5xx.
class ServerException extends ApiException {
  const ServerException({required int status, String? message})
      : super(message ?? 'Tu equipo tuvo un error ($status). Reintento en un momento.',
            status: status);

  @override
  bool get isTransient => true;
}

/// Other 4xx, or a body that is not the JSON we expect.
class BadResponseException extends ApiException {
  const BadResponseException({int? status, String? message})
      : super(message ?? 'Respuesta inesperada de tu equipo.', status: status);
}

/// DNS, TCP, TLS failures: the phone could not reach the desktop.
class NetworkException extends ApiException {
  const NetworkException({bool dns = false})
      : super(
            dns
                ? 'No encuentro tu equipo. ¿Tailscale está encendido en este teléfono?'
                : 'No llego a tu equipo. Revisa que Tailscale esté encendido aquí '
                    'y que el equipo esté prendido.',
          );

  @override
  bool get isTransient => true;
}

/// No answer in time.
class RequestTimeoutException extends ApiException {
  const RequestTimeoutException()
      : super('Tu equipo tarda en responder. Reintento en un momento.');

  @override
  bool get isTransient => true;
}
