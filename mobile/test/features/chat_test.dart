import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/features/chat/markdown.dart';
import 'package:comandos_mobile/features/chat/transcript_merge.dart';
import 'package:flutter_test/flutter_test.dart';

ChatMessage u(String t) => ChatMessage(ChatRole.user, t);
ChatMessage a(String t) => ChatMessage(ChatRole.assistant, t);
ChatMessage tool(String t) => ChatMessage(ChatRole.tool, t);

void main() {
  group('mergeTranscript', () {
    test('first load resets', () {
      final MergeOutcome m = mergeTranscript(<ChatMessage>[], <ChatMessage>[u('1'), a('2')]);
      expect(m.reset, isTrue);
      expect(m.messages, hasLength(2));
    });

    test('appends new messages and keeps existing objects', () {
      final List<ChatMessage> cur = <ChatMessage>[u('1'), a('2')];
      final MergeOutcome m = mergeTranscript(cur, <ChatMessage>[u('1'), a('2'), u('3')]);
      expect(m.reset, isFalse);
      expect(m.messages.map((ChatMessage x) => x.text), <String>['1', '2', '3']);
      expect(identical(m.messages[0], cur[0]), isTrue);
    });

    test('keeps messages that slid out of the server window', () {
      final List<ChatMessage> cur = <ChatMessage>[u('1'), a('2'), u('3')];
      final MergeOutcome m = mergeTranscript(cur, <ChatMessage>[a('2'), u('3'), a('4')]);
      expect(m.messages.map((ChatMessage x) => x.text), <String>['1', '2', '3', '4']);
    });

    test('a tool entry that grew in place is replaced, not duplicated', () {
      final List<ChatMessage> cur = <ChatMessage>[u('1'), tool('Bash · ls')];
      final MergeOutcome m =
          mergeTranscript(cur, <ChatMessage>[u('1'), tool('Bash · ls\nRead · /a.rs'), a('ok')]);
      expect(m.messages.map((ChatMessage x) => x.text), <String>['1', 'Bash · ls\nRead · /a.rs', 'ok']);
    });

    test('identical window is unchanged', () {
      final List<ChatMessage> cur = <ChatMessage>[u('1'), a('2')];
      final MergeOutcome m = mergeTranscript(cur, <ChatMessage>[u('1'), a('2')]);
      expect(m.changed, isFalse);
      expect(identical(m.messages, cur), isTrue);
    });

    test('no overlap replaces everything; the cap drops the oldest', () {
      final MergeOutcome m = mergeTranscript(<ChatMessage>[u('x')], <ChatMessage>[a('y'), a('z')]);
      expect(m.reset, isTrue);
      final List<ChatMessage> many = <ChatMessage>[for (int i = 0; i < 10; i++) u('$i')];
      final MergeOutcome capped =
          mergeTranscript(many, <ChatMessage>[...many.sublist(5), u('10'), u('11')], cap: 10);
      expect(capped.dropped, 2);
      expect(capped.messages.first.text, '2');
      expect(capped.messages.last.text, '11');
    });
  });

  group('markdown', () {
    test('blocks', () {
      final List<MdBlock> b = parseMarkdown(
          '# Título\n\nUn **párrafo** con `code`.\nsegunda línea\n\n- uno\n  - dos\n1. tres\n\n```dart\nvoid main() {}\n```\n> cita\n---\n| a | b |\n|---|---|');
      expect(b[0], isA<MdParagraph>().having((MdParagraph p) => p.heading, 'heading', 1));
      expect(b[1], isA<MdParagraph>().having((MdParagraph p) => p.text, 'text', contains('segunda línea')));
      expect(b[2], isA<MdListItem>().having((MdListItem l) => l.marker, 'marker', '•'));
      expect(b[3], isA<MdListItem>().having((MdListItem l) => l.indent, 'indent', 1));
      expect(b[4], isA<MdListItem>().having((MdListItem l) => l.marker, 'marker', '1.'));
      expect(b[5], isA<MdCode>().having((MdCode c) => c.code, 'code', 'void main() {}'));
      expect(b[6], isA<MdParagraph>().having((MdParagraph p) => p.quote, 'quote', isTrue));
      expect(b[7], isA<MdRule>());
      expect(b[8], isA<MdCode>().having((MdCode c) => c.table, 'table', isTrue));
    });

    test('an unclosed fence does not swallow the parser', () {
      final List<MdBlock> b = parseMarkdown('texto\n```\ncodigo sin cerrar');
      expect(b.last, isA<MdCode>());
    });
  });
}
