import '../../core/platform/native_channel.dart';
import 'notice.dart';

// Notice → local notification mapping. Pure Dart; unit-tested.

/// Notification tag for a session: one notification per session, a newer
/// event replaces the previous one instead of stacking.
String tagForSession(String session) => 's:$session';

/// Tag of the one-off "pair again" notification.
const String tokenAlertTag = 'comandos:token';

/// What the app remembers about a notification it showed.
class ShownAlert {
  const ShownAlert({required this.eventId, required this.waitsOnRequest});

  /// Remembers [alert] (built by [alertFor]).
  ShownAlert.of(LocalAlert alert)
      : eventId = alert.eventId ?? '',
        waitsOnRequest = alert.urgent && alert.pane != null;

  final String eventId;

  /// A "te espera" alert for a request the server tracks per pane: it goes
  /// away once the request is no longer pending.
  final bool waitsOnRequest;
}

const int _maxBody = 240;

String _clean(String s) {
  final String t = s.replaceAll(RegExp(r'\s+'), ' ').trim();
  return t.length <= _maxBody ? t : '${t.substring(0, _maxBody - 1).trimRight()}…';
}

/// Local notifications for a batch of [notices] (oldest first).
///
/// * only "needs a human", done and error events notify; news, usage,
///   focus and info never do;
/// * events already read on the desktop are skipped (seen at the desk);
/// * a "needs a human" request that is no longer in [pending] was already
///   answered: skipped (only when it names a pane, as the server tracks
///   pending requests per pane);
/// * nothing for [openChat] (that chat is on screen right now);
/// * done (not error) only when [includeDone];
/// * one alert per session: the newest event of the batch wins.
List<LocalAlert> alertsFor(
  Iterable<Notice> notices, {
  String? openChat,
  bool includeDone = true,
  Set<String>? pending,
}) {
  final Map<String, (int, LocalAlert)> byTag = <String, (int, LocalAlert)>{};
  for (final Notice n in notices) {
    final NoticeKind kind = n.phoneKind;
    if (kind == NoticeKind.other || n.read) continue;
    if (kind == NoticeKind.needsHuman &&
        pending != null &&
        n.paneId != null &&
        !pending.contains(n.eventId)) {
      continue;
    }
    if (kind == NoticeKind.done && !includeDone) continue;
    final String? session = n.sessionKey;
    if (session != null && session == openChat) continue;
    final LocalAlert a = alertFor(n);
    final (int, LocalAlert)? prev = byTag[a.tag];
    if (prev == null || prev.$1 < n.sequence) byTag[a.tag] = (n.sequence, a);
  }
  final List<(int, LocalAlert)> out = byTag.values.toList()
    ..sort(((int, LocalAlert) a, (int, LocalAlert) b) => a.$1.compareTo(b.$1));
  return <LocalAlert>[for (final (int, LocalAlert) e in out) e.$2];
}

/// The notification for one notice (no filtering).
LocalAlert alertFor(Notice n) {
  final String who = _clean(n.project ?? n.sessionKey ?? 'Una sesión');
  final NoticeKind kind = n.phoneKind;
  final String title;
  final String fallback;
  switch (kind) {
    case NoticeKind.needsHuman:
      title = '$who te espera';
      fallback = 'Necesita tu respuesta para seguir.';
    case NoticeKind.error:
      title = '$who falló';
      fallback = 'Revisa la sesión.';
    case NoticeKind.done:
    case NoticeKind.other:
      title = n.kind == 'turn_cancelled' ? '$who se detuvo' : '$who terminó';
      fallback = 'Terminó su turno.';
  }
  final String excerpt = _clean(n.excerpt);
  final String head = _clean(n.title);
  return LocalAlert(
    tag: n.sessionKey != null ? tagForSession(n.sessionKey!) : 'e:${n.eventId}',
    urgent: kind == NoticeKind.needsHuman,
    title: title,
    body: excerpt.isNotEmpty ? excerpt : (head.isNotEmpty ? head : fallback),
    session: n.sessionKey,
    pane: n.paneId,
    whenMs: n.occurredAtMs,
    eventId: n.eventId,
  );
}

/// Tags of "te espera" notifications whose request is no longer pending
/// (answered on the desktop or elsewhere): they should be removed.
List<String> resolvedTags(Map<String, ShownAlert> shown, Set<String> pending) => <String>[
      for (final MapEntry<String, ShownAlert> e in shown.entries)
        if (e.value.waitsOnRequest && !pending.contains(e.value.eventId)) e.key,
    ];
