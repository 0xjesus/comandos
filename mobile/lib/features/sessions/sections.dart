import '../../core/models/session_feed.dart';
import '../../ui/format.dart';

/// Sections of the home list, in display order.
enum SectionKind {
  /// ask / limit: the person is needed.
  waiting('Te espera'),

  /// The agent is working.
  working('Trabajando'),

  /// Finished with a reply the person has not seen.
  unread('Sin leer'),

  /// Everything else, most recent first.
  rest('El resto');

  const SectionKind(this.title);
  final String title;
}

class SessionSection {
  const SessionSection(this.kind, this.sessions);
  final SectionKind kind;
  final List<SessionSummary> sessions;
}

SectionKind sectionOf(SessionSummary s) {
  if (s.state.waitsForHuman) return SectionKind.waiting;
  if (s.state == SessionState.work) return SectionKind.working;
  if (s.unread) return SectionKind.unread;
  return SectionKind.rest;
}

/// Groups [sessions] into non-empty sections ("Te espera", "Trabajando",
/// "Sin leer", "El resto"), each newest first, keeping only labels (or
/// session names) that contain [query] (case- and accent-insensitive).
List<SessionSection> buildSections(List<SessionSummary> sessions, {String query = ''}) {
  final String q = foldForSearch(query.trim());
  final Map<SectionKind, List<SessionSummary>> groups = <SectionKind, List<SessionSummary>>{
    for (final SectionKind k in SectionKind.values) k: <SessionSummary>[],
  };
  for (final SessionSummary s in sessions) {
    if (q.isNotEmpty &&
        !foldForSearch(s.label).contains(q) &&
        !foldForSearch(s.session).contains(q)) {
      continue;
    }
    groups[sectionOf(s)]!.add(s);
  }
  return <SessionSection>[
    for (final SectionKind k in SectionKind.values)
      if (groups[k]!.isNotEmpty)
        SessionSection(k, groups[k]!..sort((SessionSummary a, SessionSummary b) => b.ts.compareTo(a.ts))),
  ];
}
