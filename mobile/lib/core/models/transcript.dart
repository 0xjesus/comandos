import 'ask_prompt.dart';
import 'json_read.dart';

enum ChatRole {
  user,
  assistant,
  tool;

  static ChatRole? parse(String r) {
    switch (r) {
      case 'u':
        return ChatRole.user;
      case 'a':
        return ChatRole.assistant;
      case 't':
        return ChatRole.tool;
    }
    return null;
  }
}

/// One transcript entry. Only `r` and `t` are used; the server's HTML (`h`)
/// is ignored on purpose.
class ChatMessage {
  ChatMessage(this.role, this.text) : hash = Object.hash(role, text);

  final ChatRole role;

  /// Raw markdown, ≤ 6000 chars (server clips with "…").
  final String text;
  final int hash;

  /// Parsed markdown, filled lazily by the renderer and reused while this
  /// object lives (objects are reused across polls when content is equal).
  Object? renderCache;

  bool sameAs(ChatMessage o) => hash == o.hash && role == o.role && text == o.text;

  static ChatMessage? tryParse(Map<String, Object?> m) {
    final ChatRole? role = ChatRole.parse(readString(m, 'r'));
    if (role == null) return null;
    return ChatMessage(role, readString(m, 't'));
  }
}

/// `POST /chat/transcript` answer.
class TranscriptResponse {
  const TranscriptResponse({
    required this.unchanged,
    required this.token,
    required this.messages,
    this.pane,
    this.agent,
    this.age,
    this.ask,
  });

  /// True when `since` equalled the current token: nothing moved.
  final bool unchanged;
  final String token;
  final List<ChatMessage> messages;
  final String? pane;
  final String? agent;

  /// Seconds since the transcript file changed.
  final int? age;
  final AskPrompt? ask;

  static TranscriptResponse? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null) return null;
    if (m['ok'] == false) return null;
    final Object? age = m['age'];
    return TranscriptResponse(
      unchanged: readBool(m, 'unchanged'),
      token: readString(m, 'token'),
      pane: readStringOrNull(m, 'pane'),
      agent: readStringOrNull(m, 'agent'),
      age: age is num ? age.toInt() : null,
      ask: AskPrompt.tryParse(m['ask']),
      messages: <ChatMessage>[
        for (final Map<String, Object?> e in readMapList(m, 'messages'))
          if (ChatMessage.tryParse(e) case final ChatMessage msg) msg,
      ],
    );
  }
}
