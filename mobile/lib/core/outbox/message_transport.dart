import '../api/api_client.dart';
import '../api/api_errors.dart';
import '../models/transcript.dart';
import 'outbox_item.dart';
import 'send_result.dart';

/// Network side of the outbox. Pure interface so the state machine is tested
/// without HTTP. Implementations must not throw: failures are [SendResult]s.
abstract class MessageTransport {
  /// `POST /session/message` with the item's clientMessageId.
  Future<SendResult> send(OutboxItem item);

  /// `GET /session/message?clientMessageId=`.
  Future<SendResult> lookup(OutboxItem item);

  /// Legacy `POST /send` (no idempotency). Called at most once per item.
  Future<SendResult> legacySend(OutboxItem item);

  /// How many user messages in the current transcript tail equal [text]
  /// (normalised). Null when it cannot be known right now.
  Future<int?> countUserMessages(String session, String? pane, String text);
}

/// Normalises text for transcript matching: trims and collapses whitespace.
String normalizeForMatch(String text) => text.trim().replaceAll(RegExp(r'\s+'), ' ');

/// True when transcript text [shown] is the message [sent]. The server clips
/// `t` at 6000 chars with "…", so long texts compare by prefix.
bool transcriptTextMatches(String shown, String sent) {
  final String a = normalizeForMatch(shown);
  final String b = normalizeForMatch(sent);
  if (a == b) return true;
  if (a.length >= 1000 && a.endsWith('…')) {
    final String prefix = a.substring(0, a.length - 1).trimRight();
    return b.startsWith(prefix);
  }
  return false;
}

/// Counts user messages in [messages] that match [text].
int countMatchingUserMessages(Iterable<ChatMessage> messages, String text) {
  int n = 0;
  for (final ChatMessage m in messages) {
    if (m.role == ChatRole.user && transcriptTextMatches(m.text, text)) n++;
  }
  return n;
}

/// [MessageTransport] over the real API. [api] returns the current client
/// (it changes when the person re-pairs), or null when unpaired.
class ApiMessageTransport implements MessageTransport {
  ApiMessageTransport(this.api);

  final ComandosApi? Function() api;

  static const String _route = '/session/message';

  Future<SendResult> _guard(Future<SendResult> Function(ComandosApi api) run) async {
    final ComandosApi? client = api();
    if (client == null) return const TransientFailure('Sin vincular.');
    try {
      return await run(client);
    } on ApiException catch (e) {
      return TransientFailure(e.userMessage);
    } catch (_) {
      return const TransientFailure('Error inesperado; reintentando.');
    }
  }

  @override
  Future<SendResult> send(OutboxItem item) => _guard((ComandosApi c) async {
        final ApiResponse r = await c.request('POST', _route,
            body: <String, Object?>{
              'clientMessageId': item.id,
              'session': item.session,
              'pane': ?item.pane,
              'text': item.text,
              'submit': true,
            },
            timeout: ApiTimeouts.send);
        return classifyMessageResponse(r);
      });

  @override
  Future<SendResult> lookup(OutboxItem item) => _guard((ComandosApi c) async {
        final ApiResponse r = await c.request('GET', _route,
            query: <String, String>{'clientMessageId': item.id}, timeout: ApiTimeouts.get);
        return classifyMessageResponse(r);
      });

  @override
  Future<SendResult> legacySend(OutboxItem item) async {
    final ComandosApi? client = api();
    if (client == null) return const Rejected(FailCode.unauthorized);
    try {
      final ApiResponse r = await client.request('POST', '/send',
          body: <String, Object?>{
            'session': item.session,
            'pane': ?item.pane,
            'text': item.text,
          },
          timeout: ApiTimeouts.send);
      return classifyLegacyResponse(r);
    } catch (_) {
      // No idempotency on /send: a failure here may still have typed.
      return const Uncertain();
    }
  }

  @override
  Future<int?> countUserMessages(String session, String? pane, String text) async {
    final ComandosApi? client = api();
    if (client == null) return null;
    try {
      final TranscriptResponse t = await client.transcript(session: session, pane: pane);
      if (t.agent == null && t.messages.isEmpty) return null;
      return countMatchingUserMessages(t.messages, text);
    } catch (_) {
      return null;
    }
  }
}
