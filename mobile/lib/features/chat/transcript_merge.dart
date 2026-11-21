import '../../core/models/transcript.dart';

/// What the chat shows after a fresh answer of `POST /chat/transcript`.
class WindowUpdate {
  const WindowUpdate(this.messages, {required this.changed});

  /// Exactly the server's window, in its order.
  final List<ChatMessage> messages;

  /// Anything differs from what was shown before.
  final bool changed;
}

/// Takes the server's latest window as the whole chat, like the web page does
/// (`chat_view.rs`: `me.messages.replace(messages)`).
///
/// The result always has the content and order of [incoming], never more.
/// The server sends no message ids and its window is a byte tail, so its
/// start moves freely (a group of tool calls loses lines from the top, the
/// same tool line repeats many times): lining windows up by content to keep
/// older messages pasted the same window twice and left the chat anchored on
/// an old answer. Nothing is kept from [current] except object identity:
/// an unchanged message keeps its object, so its widget, its parsed markdown
/// and an edit made on its card survive the poll. Each old object is reused
/// at most once (two equal messages stay two objects).
WindowUpdate adoptWindow(List<ChatMessage> current, List<ChatMessage> incoming) {
  if (current.length == incoming.length) {
    bool same = true;
    for (int i = 0; i < current.length; i++) {
      if (!current[i].sameAs(incoming[i])) {
        same = false;
        break;
      }
    }
    if (same) return WindowUpdate(current, changed: false);
  }
  final Map<int, List<ChatMessage>> pool = <int, List<ChatMessage>>{};
  for (final ChatMessage m in current) {
    (pool[m.hash] ??= <ChatMessage>[]).add(m);
  }
  ChatMessage keep(ChatMessage next) {
    final List<ChatMessage>? old = pool[next.hash];
    if (old == null) return next;
    for (int i = 0; i < old.length; i++) {
      if (old[i].sameAs(next)) return old.removeAt(i);
    }
    return next;
  }

  return WindowUpdate(
    List<ChatMessage>.unmodifiable(<ChatMessage>[for (final ChatMessage m in incoming) keep(m)]),
    changed: true,
  );
}
