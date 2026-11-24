import 'package:comandos_mobile/core/models/ask_prompt.dart';
import 'package:comandos_mobile/features/chat/ask_card.dart';
import 'package:comandos_mobile/features/chat/chat_media.dart';
import 'package:comandos_mobile/features/chat/code_tokens.dart';
import 'package:comandos_mobile/features/chat/markdown.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// The menu Claude draws for AskUserQuestion, as the server sends it
/// (`dash/native/ask.rs::to_json`).
const Map<String, Object?> _question = <String, Object?>{
  'question': 'Which features?',
  'detail': <String>[],
  'tabs': <Map<String, Object?>>[
    <String, Object?>{'label': 'Images', 'done': true},
    <String, Object?>{'label': 'Extras', 'done': false},
  ],
  'multi': true,
  'sig': 'abc123',
  'options': <Map<String, Object?>>[
    <String, Object?>{'key': '1', 'label': 'Mermaid', 'desc': <String>['Draw diagrams'], 'on': true, 'checked': true, 'kind': 'option'},
    <String, Object?>{'key': '2', 'label': 'Videos', 'desc': <String>[], 'on': false, 'checked': false, 'kind': 'option'},
    <String, Object?>{'key': '3', 'label': 'Type something.', 'desc': <String>[], 'on': false, 'kind': 'free'},
    <String, Object?>{'key': '', 'label': 'Submit', 'desc': <String>[], 'on': false, 'kind': 'submit'},
    <String, Object?>{'key': '4', 'label': 'Chat about this', 'desc': <String>[], 'on': false, 'kind': 'chat'},
  ],
};

void main() {
  group('menu', () {
    test('every row is read with its place in the menu', () {
      final AskPrompt a = AskPrompt.tryParse(_question)!;
      expect(a.sig, 'abc123');
      expect(a.signature, 'abc123');
      expect(a.multi, isTrue);
      expect(a.tabs, <(String, bool)>[('Images', true), ('Extras', false)]);
      expect(a.options.map((AskOption o) => o.row), <int>[0, 1, 2, 3, 4]);
      expect(a.options.map((AskOption o) => o.kind),
          <AskKind>[AskKind.option, AskKind.option, AskKind.free, AskKind.submit, AskKind.chat]);
      expect(a.options.first.desc, <String>['Draw diagrams']);
      expect(a.options.first.checked, isTrue);
      // A voice names numbered rows, never the text field.
      expect(a.choices.map((AskOption o) => o.key), <String>['1', '2', '4']);
    });

    testWidgets('the card draws every row and answers the one tapped', (WidgetTester t) async {
      final List<(int, String?)> answered = <(int, String?)>[];
      await t.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: AskCard(ask: AskPrompt.tryParse(_question)!, onAnswer: (AskOption o, String? text) => answered.add((o.row, text))),
          ),
        ),
      ));
      expect(find.text('Which features?'), findsOneWidget);
      expect(find.text('Draw diagrams'), findsOneWidget);
      expect(find.text('✓ Images'), findsOneWidget);
      expect(find.text('Hablar de esto en el chat'), findsOneWidget);
      await t.tap(find.text('Videos'));
      await t.enterText(find.byType(TextField), 'GIFs también');
      await t.tap(find.text('Enviar').first);
      await t.pump();
      expect(answered, <(int, String?)>[(1, null), (2, 'GIFs también')]);
    });
  });

  group('media', () {
    test('only internet images, videos and PDFs are media', () {
      expect(mediaOf('https://x.io/a.GIF?x=1'), MediaKind.image);
      expect(mediaOf('http://x.io/v.mp4#t=3'), MediaKind.video);
      expect(mediaOf('https://x.io/doc.pdf'), MediaKind.pdf);
      expect(mediaOf('https://x.io/page'), isNull);
      expect(mediaOf('/srv/demo/a.png'), isNull);
      expect(mediaOf('file:///srv/demo/a.png'), isNull);
    });

    test('each address once, in order, from images, links and bare addresses', () {
      final List<MediaRef> m = mediaIn(
          '![gato](https://x.io/g.gif) mira [el manual](https://x.io/m.pdf) y https://x.io/v.webm. Otra vez https://x.io/g.gif, `https://x.io/no.png`');
      expect(m.map((MediaRef r) => (r.kind, r.url, r.alt)), <(MediaKind, String, String)>[
        (MediaKind.image, 'https://x.io/g.gif', 'gato'),
        (MediaKind.pdf, 'https://x.io/m.pdf', 'el manual'),
        (MediaKind.video, 'https://x.io/v.webm', ''),
      ]);
    });

    test('the viewer page cannot be broken out of', () {
      final String mermaid = viewerHtml(null, mermaid: 'graph TD\nA-->B</script><script>alert(1)</script>');
      expect(mermaid, isNot(contains('</script><script>alert')));
      expect(mermaid, contains('securityLevel:"strict"'));
      expect(mermaid, contains('integrity="sha384-'));
      final String video = viewerHtml(MediaKind.video, url: 'https://x.io/v.mp4"><script>');
      expect(video, isNot(contains('"><script>')));
    });

    testWidgets('an image alone is drawn without its markup; a Mermaid block gets its card', (WidgetTester t) async {
      await t.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: MarkdownView(parseMarkdown('![logo](https://x.io/logo.png)\n\n```mermaid\ngraph TD\nA-->B\n```')),
          ),
        ),
      ));
      expect(find.byType(MediaPreview), findsOneWidget);
      expect(find.textContaining('logo.png'), findsNothing);
      expect(find.textContaining('🖼'), findsNothing);
      expect(find.byType(MermaidCard), findsOneWidget);
      expect(find.text('Ver diagrama'), findsOneWidget);
    });
  });

  group('code', () {
    test('pieces cover the code exactly and name each one', () {
      const String rust = 'fn main() {\n    let s = "hola"; // saludo\n    println!("{}", 42);\n}';
      final List<(Tok, String)> r = codeTokens('rust', rust);
      expect(r.map(((Tok, String) p) => p.$2).join(), rust);
      expect(r, contains((Tok.keyword, 'fn')));
      expect(r, contains((Tok.function, 'main')));
      expect(r, contains((Tok.string, '"hola"')));
      expect(r, contains((Tok.comment, '// saludo')));
      expect(r, contains((Tok.function, 'println')));
      expect(r, contains((Tok.number, '42')));

      final List<(Tok, String)> sh = codeTokens('bash', r'echo "$HOME" # dónde');
      expect(sh, contains((Tok.function, 'echo')));
      expect(sh, contains((Tok.comment, '# dónde')));

      final List<(Tok, String)> json = codeTokens('json', '{"a": "b", "n": 1}');
      expect(json, contains((Tok.function, '"a"')));
      expect(json, contains((Tok.string, '"b"')));

      final List<(Tok, String)> diff = codeTokens('', '@@ -1 +1 @@\n-viejo\n+nuevo');
      expect(diff.map(((Tok, String) p) => p.$1), containsAll(<Tok>[Tok.meta, Tok.removed, Tok.added]));

      expect(codeTokens('brainfuck', '+-.'), <(Tok, String)>[(Tok.plain, '+-.')]);
    });
  });
}
