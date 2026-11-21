import '../api/api_response.dart';
import 'outbox_item.dart';

/// What one delivery step produced. The outbox state machine switches on it.
sealed class SendResult {
  const SendResult();
}

/// 200 `{status:"delivered"}` (also a replayed duplicate).
class Delivered extends SendResult {
  const Delivered({this.duplicate = false, this.deliveredAtMs, this.pane});
  final bool duplicate;
  final int? deliveredAtMs;
  final String? pane;
}

/// 202 `{status:"delivering"}`: the server is still typing this id.
class InProgress extends SendResult {
  const InProgress();
}

/// 409 / 429: per-pane lock held or server busy. Retry soon.
class Busy extends SendResult {
  const Busy();
}

/// Definitive rejection; nothing was typed. Never auto-retried.
class Rejected extends SendResult {
  const Rejected(this.code);
  final String code;
}

/// The server may have typed part of it (500 `{status:"uncertain"}`).
class Uncertain extends SendResult {
  const Uncertain();
}

/// The server does not have `/session/message` (old ComandOS).
class RouteMissing extends SendResult {
  const RouteMissing();
}

/// `GET /session/message` → 404 `{status:"unknown"}`: never seen this id.
class UnknownReceipt extends SendResult {
  const UnknownReceipt();
}

/// Network failure, timeout or 5xx: the request may or may not have arrived.
class TransientFailure extends SendResult {
  const TransientFailure(this.message);
  final String message;
}

/// Classifies an answer of `POST /session/message` or
/// `GET /session/message?clientMessageId=`.
SendResult classifyMessageResponse(ApiResponse r) {
  final int s = r.status;
  final String? status = r.map['status'] is String ? r.map['status'] as String : null;
  final String? code = r.code;
  if (s == 200) {
    // GET returns the stored receipt with 200 whatever its state, so the
    // `status` field decides, never `ok` alone.
    switch (status) {
      case 'delivered':
        final Object? at = r.map['deliveredAtMs'];
        final Object? pane = r.map['pane'];
        return Delivered(
          duplicate: r.map['duplicate'] == true,
          deliveredAtMs: at is num ? at.toInt() : null,
          pane: pane is String ? pane : null,
        );
      case 'delivering':
      case 'accepted':
        return const InProgress();
      case 'uncertain':
        return const Uncertain();
      case 'rejected':
        // Rejections leave no receipt server-side; treat as definitive.
        return Rejected(code ?? FailCode.invalid);
    }
    // A 200 we cannot read: safe to ask again, the route is idempotent.
    return const TransientFailure('Respuesta ilegible; reintentando.');
  }
  if (s == 202) return const InProgress();
  if (s == 404) {
    if (code == FailCode.paneGone || code == FailCode.sessionGone) return Rejected(code!);
    if (status == 'unknown') return const UnknownReceipt();
    if (code != null) return Rejected(code);
    return const RouteMissing();
  }
  if (s == 405 || s == 501) return const RouteMissing();
  if (s == 409 || s == 429) return const Busy();
  if (s == 401) return const Rejected(FailCode.unauthorized);
  if (s == 403) return const Rejected(FailCode.forbidden);
  if (s >= 500) {
    // A stored uncertain receipt is replayed as 500: never auto-resend.
    if (status == 'uncertain') return const Uncertain();
    // 500 tmux_failed / 503 unavailable are rejections that leave no
    // receipt (nothing was pasted), and 502/504 are transport trouble:
    // retrying the same id later is safe.
    return TransientFailure('Tu equipo respondió $s; reintentando.');
  }
  if (s >= 400) return Rejected(code ?? FailCode.invalid);
  return TransientFailure('Respuesta inesperada ($s); reintentando.');
}

/// Classifies an answer of the legacy `POST /send` (no request id). A
/// network error or ambiguous 5xx there is [Uncertain], never retried.
SendResult classifyLegacyResponse(ApiResponse r) {
  final int s = r.status;
  if (s >= 200 && s < 300) return const Delivered();
  if (s == 401) return const Rejected(FailCode.unauthorized);
  if (s == 403) return const Rejected(FailCode.forbidden);
  if (s == 404) return const Rejected(FailCode.sessionGone);
  // 502: the native route declined and the legacy server is down, or
  // Tailscale could not reach the backend. Nothing was typed.
  if (s == 502) return const Rejected(FailCode.serverUnavailable);
  if (s >= 400 && s < 500) return const Rejected(FailCode.invalid);
  return const Uncertain();
}
