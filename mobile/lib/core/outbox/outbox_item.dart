/// Lifecycle of one outgoing message.
///
/// ```
/// queued ──▶ sending ──▶ delivered
///   ▲           │
///   └─(retry)───┤──▶ failed(reason)    (definitive: nothing was typed)
///               └──▶ uncertain         (may or may not have been typed)
/// ```
enum OutboxState { queued, sending, delivered, failed, uncertain }

/// Machine codes for [OutboxItem.failCode]. Spanish copy lives in
/// [outboxFailureMessage].
abstract final class FailCode {
  static const String paneGone = 'pane_gone';
  static const String sessionGone = 'session_gone';
  static const String invalid = 'invalid';
  static const String unauthorized = 'unauthorized';
  static const String forbidden = 'forbidden';
  static const String legacyTooLong = 'legacy_too_long';
  static const String serverUnavailable = 'server_unavailable';
  static const String otherHost = 'other_host';
  static const String uncertain = 'uncertain';
}

/// Spanish explanation of why an item failed or is uncertain.
String outboxFailureMessage(String? code) {
  switch (code) {
    case FailCode.paneGone:
      return 'Ese panel ya no existe en tu equipo. Reintenta para enviarlo al panel actual de la sesión.';
    case FailCode.sessionGone:
      return 'La sesión ya no existe en tu equipo. No se envió.';
    case FailCode.invalid:
      return 'Tu equipo rechazó el mensaje por su formato. No se envió.';
    case FailCode.unauthorized:
      return 'Tu equipo rechazó el token. Vuelve a vincular en Ajustes y reintenta.';
    case FailCode.forbidden:
      return 'Tu equipo no acepta esta dirección. Vincula con el nombre MagicDNS y reintenta.';
    case FailCode.legacyTooLong:
      return 'Tu ComandOS es una versión anterior que acepta hasta 4000 caracteres. '
          'Actualízalo o divide el mensaje.';
    case FailCode.serverUnavailable:
      return 'Tu equipo no pudo atender el envío. No se escribió nada; puedes reintentar.';
    case FailCode.otherHost:
      return 'Este mensaje era para otro equipo. Reintenta para enviarlo al equipo vinculado.';
    case FailCode.uncertain:
      return 'No sé si llegó. Revisa el chat antes de reenviar.';
  }
  return 'No se envió.';
}

/// One message in the persistent outbox.
///
/// [id] is the `clientMessageId`: generated once at enqueue and reused on
/// every automatic retry, so the server can deduplicate.
class OutboxItem {
  const OutboxItem({
    required this.id,
    required this.host,
    required this.session,
    required this.text,
    required this.createdAtMs,
    this.pane,
    this.state = OutboxState.queued,
    this.attempts = 0,
    this.retryStreak = 0,
    this.nextAttemptAtMs = 0,
    this.lastError,
    this.failCode,
    this.ambiguous = false,
    this.legacyTried = false,
    this.deliveredAtMs,
    this.transcriptBaseline,
    this.checksLeft = 0,
  });

  final String id;

  /// The desktop host it was written for. Never sent to another host.
  final String host;
  final String session;
  final String? pane;
  final String text;
  final int createdAtMs;
  final OutboxState state;

  /// Total attempts (including lookups).
  final int attempts;

  /// Consecutive retryable failures; drives the backoff.
  final int retryStreak;

  /// Not before this time (ms since epoch).
  final int nextAttemptAtMs;

  /// Last transient problem, Spanish, for the UI ("Sin conexión…").
  final String? lastError;

  /// Why it failed / is uncertain (see [FailCode]).
  final String? failCode;

  /// A previous attempt may have reached the server (timeout, 5xx, 202):
  /// the next attempt asks for the receipt before POSTing again.
  final bool ambiguous;

  /// The legacy `/send` route has been used (at most once per id).
  final bool legacyTried;
  final int? deliveredAtMs;

  /// How many user messages with this same text the transcript already had
  /// when this item was created (plus same-text items queued ahead of it).
  /// The message is "seen" once the transcript has more than that.
  final int? transcriptBaseline;

  /// Remaining automatic transcript checks for an uncertain item.
  final int checksLeft;

  bool get isTerminalOk => state == OutboxState.delivered;
  bool get needsHuman => state == OutboxState.failed || state == OutboxState.uncertain;
  bool get isPending => state == OutboxState.queued || state == OutboxState.sending;

  OutboxItem copyWith({
    String? id,
    String? host,
    String? pane,
    bool clearPane = false,
    OutboxState? state,
    int? attempts,
    int? retryStreak,
    int? nextAttemptAtMs,
    String? lastError,
    bool clearLastError = false,
    String? failCode,
    bool clearFailCode = false,
    bool? ambiguous,
    bool? legacyTried,
    int? deliveredAtMs,
    int? transcriptBaseline,
    int? checksLeft,
  }) =>
      OutboxItem(
        id: id ?? this.id,
        host: host ?? this.host,
        session: session,
        pane: clearPane ? null : (pane ?? this.pane),
        text: text,
        createdAtMs: createdAtMs,
        state: state ?? this.state,
        attempts: attempts ?? this.attempts,
        retryStreak: retryStreak ?? this.retryStreak,
        nextAttemptAtMs: nextAttemptAtMs ?? this.nextAttemptAtMs,
        lastError: clearLastError ? null : (lastError ?? this.lastError),
        failCode: clearFailCode ? null : (failCode ?? this.failCode),
        ambiguous: ambiguous ?? this.ambiguous,
        legacyTried: legacyTried ?? this.legacyTried,
        deliveredAtMs: deliveredAtMs ?? this.deliveredAtMs,
        transcriptBaseline: transcriptBaseline ?? this.transcriptBaseline,
        checksLeft: checksLeft ?? this.checksLeft,
      );

  Map<String, Object?> toJson() => <String, Object?>{
        'id': id,
        'host': host,
        'session': session,
        'pane': pane,
        'text': text,
        'createdAtMs': createdAtMs,
        'state': state.name,
        'attempts': attempts,
        'retryStreak': retryStreak,
        'nextAttemptAtMs': nextAttemptAtMs,
        'lastError': lastError,
        'failCode': failCode,
        'ambiguous': ambiguous,
        'legacyTried': legacyTried,
        'deliveredAtMs': deliveredAtMs,
        'transcriptBaseline': transcriptBaseline,
        'checksLeft': checksLeft,
      };

  static OutboxItem? fromJson(Object? json) {
    if (json is! Map) return null;
    final Object? id = json['id'];
    final Object? session = json['session'];
    final Object? text = json['text'];
    if (id is! String || session is! String || text is! String) return null;
    int i(String k) => (json[k] is num) ? (json[k] as num).toInt() : 0;
    int? ni(String k) => (json[k] is num) ? (json[k] as num).toInt() : null;
    String? s(String k) => json[k] is String ? json[k] as String : null;
    return OutboxItem(
      id: id,
      host: s('host') ?? '',
      session: session,
      pane: s('pane'),
      text: text,
      createdAtMs: i('createdAtMs'),
      state: OutboxState.values.firstWhere((OutboxState e) => e.name == json['state'],
          orElse: () => OutboxState.queued),
      attempts: i('attempts'),
      retryStreak: i('retryStreak'),
      nextAttemptAtMs: i('nextAttemptAtMs'),
      lastError: s('lastError'),
      failCode: s('failCode'),
      ambiguous: json['ambiguous'] == true,
      legacyTried: json['legacyTried'] == true,
      deliveredAtMs: ni('deliveredAtMs'),
      transcriptBaseline: ni('transcriptBaseline'),
      checksLeft: i('checksLeft'),
    );
  }
}
