import '../../core/models/transcript.dart';
import '../../core/util/clock.dart';
import 'text_prep.dart';

/// A finished assistant reply: everything the assistant wrote after the
/// last user message.
class ReplyRef {
  const ReplyRef(this.id, this.text);

  /// Stable while the reply does not change (hash of question + answer).
  final String id;
  final String text;

  @override
  String toString() => 'ReplyRef($id)';
}

/// The current reply of a transcript, or null when the last word is not
/// the assistant's (a tool is running, or the person just wrote).
ReplyRef? currentReply(List<ChatMessage> messages) {
  if (messages.isEmpty || messages.last.role != ChatRole.assistant) return null;
  int lastUser = -1;
  for (int i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role == ChatRole.user) {
      lastUser = i;
      break;
    }
  }
  final List<String> parts = <String>[
    for (int i = lastUser + 1; i < messages.length; i++)
      if (messages[i].role == ChatRole.assistant && messages[i].text.trim().isNotEmpty) messages[i].text,
  ];
  if (parts.isEmpty) return null;
  final String text = parts.join('\n\n');
  final String question = lastUser >= 0 ? messages[lastUser].text : '';
  return ReplyRef(fnv64('$question\u0001$text'), text);
}

/// Detects a reply that has COMPLETED, the way the chat detects "working →
/// done", and reports each one once. Partial text is never reported: the
/// reply must be the last message, the agent must not be working, and it
/// must stay unchanged for [settle] (Claude often writes a sentence, then
/// a tool call a moment later).
class ReplyWatcher {
  ReplyWatcher({NowMs now = systemNowMs, this.settle = const Duration(seconds: 4), Set<String>? alreadyRead})
      : _now = now,
        _read = alreadyRead ?? <String>{};

  final NowMs _now;
  final Duration settle;
  final Set<String> _read;
  String? _candidate;
  int _since = 0;
  bool _baselined = false;

  bool get baselined => _baselined;

  /// Marks whatever reply is on screen now as already known (not new).
  void baseline(List<ChatMessage> messages) {
    _baselined = true;
    final ReplyRef? r = currentReply(messages);
    if (r != null) _read.add(r.id);
  }

  void markRead(String id) => _read.add(id);
  bool wasRead(String id) => _read.contains(id);

  /// Feed on every transcript update. Returns a completed, unread reply
  /// once; null otherwise. When it returns null with [pending] true, call
  /// again after [settle] (nothing else may trigger a call).
  ReplyRef? observe(List<ChatMessage> messages, {required bool working, bool feedWorking = false}) {
    if (!_baselined) {
      baseline(messages);
      return null;
    }
    final ReplyRef? r = working || feedWorking ? null : currentReply(messages);
    if (r == null || _read.contains(r.id)) {
      _candidate = null;
      return null;
    }
    final int now = _now();
    if (_candidate != r.id) {
      _candidate = r.id;
      _since = now;
    }
    if (now - _since < settle.inMilliseconds) return null;
    _candidate = null;
    _read.add(r.id);
    return r;
  }

  /// A reply is waiting for [settle] to pass.
  bool get pending => _candidate != null;
}
