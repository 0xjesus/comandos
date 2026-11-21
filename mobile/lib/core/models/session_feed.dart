import 'ask_prompt.dart';
import 'json_read.dart';

/// Session state as reported by `/session-feed` (most urgent pane wins).
enum SessionState {
  /// A numbered menu / permission prompt is waiting for the person.
  ask,

  /// A usage-limit message is on screen.
  limit,

  /// The agent is working.
  work,

  /// Nothing to do.
  idle;

  static SessionState parse(String s) {
    switch (s) {
      case 'ask':
        return SessionState.ask;
      case 'limit':
        return SessionState.limit;
      case 'work':
        return SessionState.work;
      default:
        return SessionState.idle;
    }
  }

  bool get waitsForHuman => this == SessionState.ask || this == SessionState.limit;
}

/// One pane of a session (from `paneList`).
class PaneSummary {
  const PaneSummary({
    required this.pane,
    required this.state,
    required this.ts,
    this.agent = '',
    this.model = '',
    this.account = '',
    this.reply = '',
  });

  final String pane;
  final SessionState state;
  final double ts;
  final String agent;
  final String model;
  final String account;
  final String reply;

  static PaneSummary? tryParse(Map<String, Object?> m) {
    final String pane = readString(m, 'pane');
    if (pane.isEmpty) return null;
    return PaneSummary(
      pane: pane,
      state: SessionState.parse(readString(m, 'state')),
      ts: readDouble(m, 'ts'),
      agent: readString(m, 'agent'),
      model: readString(m, 'model'),
      account: readString(m, 'account'),
      reply: readString(m, 'reply'),
    );
  }
}

/// One card of the sessions list.
class SessionSummary {
  const SessionSummary({
    required this.session,
    required this.label,
    required this.state,
    required this.unread,
    required this.ts,
    this.seen = 0,
    this.pane,
    this.panes = 1,
    this.agent = '',
    this.model = '',
    this.prompt = '',
    this.reply = '',
    this.ask,
    this.paneList = const <PaneSummary>[],
  });

  final String session;
  final String label;
  final SessionState state;
  final bool unread;

  /// Last activity, seconds since epoch (server clock).
  final double ts;
  final double seen;
  final String? pane;
  final int panes;
  final String agent;
  final String model;
  final String prompt;
  final String reply;
  final AskPrompt? ask;
  final List<PaneSummary> paneList;

  /// "claude · opus-5", or whichever part is known.
  String get agentModel =>
      <String>[agent, model].where((String s) => s.isNotEmpty).join(' · ');

  /// Everything a row shows. Equal signatures ⇒ the row need not rebuild.
  String get signature =>
      '$label\u0001${state.name}\u0001$unread\u0001${ts.floor()}\u0001$agent\u0001$model'
      '\u0001$pane\u0001$reply\u0001${ask?.signature ?? ''}';

  static SessionSummary? tryParse(Map<String, Object?> m) {
    final String session = readString(m, 'session');
    if (session.isEmpty) return null;
    final String label = readString(m, 'label');
    return SessionSummary(
      session: session,
      label: label.isEmpty ? session : label,
      state: SessionState.parse(readString(m, 'state')),
      unread: readBool(m, 'unread'),
      ts: readDouble(m, 'ts'),
      seen: readDouble(m, 'seen'),
      pane: readStringOrNull(m, 'pane'),
      panes: readInt(m, 'panes', 1),
      agent: readString(m, 'agent'),
      model: readString(m, 'model'),
      prompt: readString(m, 'prompt'),
      reply: readString(m, 'reply'),
      ask: AskPrompt.tryParse(m['ask']),
      paneList: <PaneSummary>[
        for (final Map<String, Object?> p in readMapList(m, 'paneList'))
          if (PaneSummary.tryParse(p) case final PaneSummary ps) ps,
      ],
    );
  }
}

/// `GET /session-feed` answer.
class SessionFeed {
  const SessionFeed({required this.now, required this.sessions});

  /// Server clock, seconds since epoch. Relative times use it, not the phone's.
  final double now;
  final List<SessionSummary> sessions;

  static SessionFeed? tryParse(Object? json) {
    final Map<String, Object?>? m = asMap(json);
    if (m == null || m['sessions'] is! List) return null;
    return SessionFeed(
      now: readDouble(m, 'now', DateTime.now().millisecondsSinceEpoch / 1000),
      sessions: <SessionSummary>[
        for (final Map<String, Object?> s in readMapList(m, 'sessions'))
          if (SessionSummary.tryParse(s) case final SessionSummary ss) ss,
      ],
    );
  }
}
