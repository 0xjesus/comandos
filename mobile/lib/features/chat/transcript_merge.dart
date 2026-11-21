import '../../core/models/transcript.dart';

/// Result of merging a fresh transcript window into what the chat shows.
class MergeOutcome {
  const MergeOutcome({
    required this.messages,
    required this.changed,
    required this.reset,
    required this.dropped,
  });

  final List<ChatMessage> messages;

  /// Anything differs from before.
  final bool changed;

  /// No overlap was found: the list was replaced wholesale.
  final bool reset;

  /// How many messages were removed from the front (cap).
  final int dropped;
}

/// True when [next] is [prev] grown in place (consecutive tool calls are
/// merged into one entry that gains lines).
bool _grewFrom(ChatMessage prev, ChatMessage next) {
  if (prev.role != next.role) return false;
  final String p = prev.text.endsWith('…') ? prev.text.substring(0, prev.text.length - 1) : prev.text;
  return p.isNotEmpty && next.text.startsWith(p);
}

/// Merges [incoming] (the server's latest ≤80-message tail) into [current].
///
/// The server has no message ids, so alignment is by content: find the
/// longest suffix of [current] that equals a prefix of [incoming] (the last
/// overlapping entry may have grown in place). Equal messages keep their
/// existing objects, so their widgets and render caches survive. Older
/// messages that slid out of the server window are kept, up to [cap].
MergeOutcome mergeTranscript(List<ChatMessage> current, List<ChatMessage> incoming, {int cap = 300}) {
  if (current.isEmpty || incoming.isEmpty) {
    final bool changed = current.length != incoming.length;
    final List<ChatMessage> list =
        incoming.length > cap ? incoming.sublist(incoming.length - cap) : incoming;
    return MergeOutcome(messages: list, changed: changed, reset: true, dropped: 0);
  }
  final int maxOverlap = current.length < incoming.length ? current.length : incoming.length;
  for (int j = maxOverlap; j >= 1; j--) {
    final int s = current.length - j;
    bool ok = true;
    bool grew = false;
    for (int i = 0; i < j; i++) {
      final ChatMessage a = current[s + i];
      final ChatMessage b = incoming[i];
      if (a.sameAs(b)) continue;
      if (i == j - 1 && _grewFrom(a, b)) {
        grew = true;
        continue;
      }
      ok = false;
      break;
    }
    if (!ok) continue;
    if (!grew && j == incoming.length) {
      return MergeOutcome(messages: current, changed: false, reset: false, dropped: 0);
    }
    final List<ChatMessage> merged = <ChatMessage>[
      ...current.sublist(0, current.length - (grew ? 1 : 0)),
      if (grew) incoming[j - 1],
      ...incoming.sublist(j),
    ];
    final int dropped = merged.length > cap ? merged.length - cap : 0;
    return MergeOutcome(
      messages: dropped > 0 ? merged.sublist(dropped) : merged,
      changed: true,
      reset: false,
      dropped: dropped,
    );
  }
  final List<ChatMessage> list =
      incoming.length > cap ? incoming.sublist(incoming.length - cap) : incoming;
  return MergeOutcome(messages: list, changed: true, reset: true, dropped: 0);
}
