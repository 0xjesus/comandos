import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:comandos_mobile/app_navigator.dart';
import 'package:comandos_mobile/core/api/api_client.dart';
import 'package:comandos_mobile/core/config/credential_store.dart';
import 'package:comandos_mobile/core/config/paired_server.dart';
import 'package:comandos_mobile/core/connection.dart';
import 'package:comandos_mobile/core/error_reporter.dart';
import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/models/transcript.dart';
import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_item.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/polling/app_visibility.dart';
import 'package:comandos_mobile/features/chat/dock.dart';
import 'package:comandos_mobile/features/chat/markdown.dart';
import 'package:comandos_mobile/features/chat/outbox_bubble.dart';
import 'package:comandos_mobile/features/chat/pane_rows.dart';
import 'package:comandos_mobile/features/chat/session_view.dart';
import 'package:comandos_mobile/features/chat/term_keys.dart';
import 'package:comandos_mobile/features/chat/transcript_merge.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

ChatMessage u(String t) => ChatMessage(ChatRole.user, t);
ChatMessage a(String t) => ChatMessage(ChatRole.assistant, t);
ChatMessage tool(String t) => ChatMessage(ChatRole.tool, t);

class _NoTimer implements Timer {
  @override
  void cancel() {}
  @override
  bool get isActive => false;
  @override
  int get tick => 0;
}

/// Records what the dock types over the terminal WebSocket.
class FakeTermInput implements TermInput {
  final List<({String? pane, bool split, String bytes})> typed = <({String? pane, bool split, String bytes})>[];

  @override
  Future<String?> type({required String session, String? pane, required bool split, required String bytes}) async {
    typed.add((pane: pane, split: split, bytes: bytes));
    return null;
  }

  @override
  void close() {}
}

http.Response _json(Object body, [int status = 200]) => http.Response.bytes(utf8.encode(jsonEncode(body)), status,
    headers: <String, String>{'content-type': 'application/json; charset=utf-8'});

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
          '# Título\n\nUn **párrafo** con `code`.\nsegunda línea\n\n- uno\n  - dos\n1. tres\n\n```Rust title="x"\nfn main() {}\n```\n> cita\n---\n| a | b:|\n|---|--:|\n| 1 | 2 |\n\n- [x] hecho\n\n> [!WARNING]\n> cuidado');
      expect(b[0], isA<MdParagraph>().having((MdParagraph p) => p.heading, 'heading', 1));
      expect(b[1], isA<MdParagraph>().having((MdParagraph p) => p.text, 'text', contains('segunda línea')));
      expect(b[2], isA<MdListItem>().having((MdListItem l) => l.marker, 'marker', '•'));
      expect(b[3], isA<MdListItem>().having((MdListItem l) => l.indent, 'indent', 1));
      expect(b[4], isA<MdListItem>().having((MdListItem l) => l.marker, 'marker', '1.'));
      expect(b[5], isA<MdCode>().having((MdCode c) => c.code, 'code', 'fn main() {}').having((MdCode c) => c.lang, 'lang', 'rust'));
      expect(b[6], isA<MdQuote>().having((MdQuote q) => q.alert, 'alert', isNull));
      expect(b[7], isA<MdRule>());
      expect(
        b[8],
        isA<MdTable>()
            .having((MdTable t) => t.header, 'header', <String>['a', 'b:'])
            .having((MdTable t) => t.align, 'align', <MdAlign>[MdAlign.left, MdAlign.right])
            .having((MdTable t) => t.rows.single, 'row', <String>['1', '2']),
      );
      expect(b[9], isA<MdListItem>().having((MdListItem l) => l.checked, 'checked', isTrue));
      expect(b[10], isA<MdQuote>().having((MdQuote q) => q.alert, 'alert', 'warning'));
    });

    test('an unclosed fence does not swallow the parser', () {
      final List<MdBlock> b = parseMarkdown('texto\n```\ncodigo sin cerrar');
      expect(b.last, isA<MdCode>());
    });

    test('plain text for the clipboard drops the marks', () {
      expect(markdownPlain(parseMarkdown('## Plan\n\nHaz **esto** y [mira](https://x.io).\n\n- uno\n- dos')),
          'Plan\n\nHaz esto y mira (https://x.io).\n\n• uno\n• dos');
    });

    test('only http(s)/mailto links open anything', () {
      expect(safeHref('https://x.io'), 'https://x.io');
      expect(safeHref('mailto:a@b.c'), 'mailto:a@b.c');
      expect(safeHref('javascript:alert(1)'), isNull);
      expect(safeHref('src/main.rs'), isNull);
    });
  });

  group('web rules', () {
    PaneRow row(String pane, String agent, String model, {bool active = false}) =>
        PaneRow(pane: pane, agent: agent, model: model, active: active);

    test('pane chips name agent and model', () {
      expect(paneLabel(row('%1', 'claude', 'claude-opus-5-5')), 'Claude · opus-5.5');
      expect(paneLabel(row('%1', 'claude', 'claude-fable-5-2')), 'Claude · fable-5.2');
      expect(paneLabel(row('%1', 'codex', 'gpt-6-astra')), 'Codex · gpt-6-astra');
      expect(paneLabel(row('%1', 'codex', 'gpt-6.1-sol')), 'Codex · gpt-6.1-sol');
      expect(paneLabel(row('%1', 'claude', '')), 'Claude');
      expect(paneLabel(row('%1', '', '')), 'Terminal');
      expect(paneLabel(row('%1', 'grok', '')), 'Grok');
    });

    test('the phone keeps its pane and moves on when it closes', () {
      final List<PaneRow> rows = <PaneRow>[row('%7', 'claude', ''), row('%39', 'claude', '', active: true), row('%44', '', '')];
      expect(choosePane(rows, ''), (pane: '%39', closed: false));
      expect(choosePane(rows, '%7'), (pane: '%7', closed: false));
      expect(choosePane(rows, '%40'), (pane: '%44', closed: true));
      expect(choosePane(rows, '%99'), (pane: '%7', closed: true));
      expect(choosePane(const <PaneRow>[], '%7'), (pane: '%7', closed: false));
    });

    test('placeholder names the target on a split', () {
      final List<PaneRow> rows = <PaneRow>[row('%1', 'claude', 'claude-fable-5-2'), row('%2', 'claude', 'claude-fable-5-2')];
      expect(composerHint(rows, '%1'), 'Escribe a 1 · Claude · fable-5.2…');
      expect(composerHint(rows.sublist(0, 1), '%1'), 'Escribe a la sesión…');
    });

    test('/state rows: live panes of the session only, in tmux order', () {
      final List<PaneRow> rows = paneRowsFor(<Object?>[
        <String, Object?>{'session': 's', 'pane': '%76', 'agent': 'claude', 'status': 'waiting', 'alive': true},
        <String, Object?>{'session': 's', 'pane': '%51', 'agent': 'claude', 'status': 'done', 'alive': true, 'paneActive': true},
        <String, Object?>{'session': 's', 'agent': 'claude', 'alive': false},
        <String, Object?>{'session': 'otra', 'pane': '%3', 'alive': true},
        <String, Object?>{'session': 's', 'pane': '%51', 'alive': true},
      ], 's');
      expect(rows.map((PaneRow r) => r.pane), <String>['%51', '%76']);
      expect(rows.first.active, isTrue);
      expect(rows.last.status, 'waiting');
    });

    test('collapsed view starts at the last answer', () {
      final List<ChatMessage> list = <ChatMessage>[u('a'), a('b'), u('c'), tool('Bash'), a('d'), tool('Read')];
      expect(foldIndex(list), 4);
      expect(foldIndex(list.sublist(0, 4)), 1);
      expect(foldIndex(<ChatMessage>[tool('x'), u('y')]), 1);
      expect(moreLabel(9, expanded: false), 'Ver 9 mensajes anteriores');
      expect(moreLabel(1, expanded: false), 'Ver 1 mensaje anterior');
      expect(moreLabel(9, expanded: true), 'Ocultar mensajes anteriores');
    });

    test('only a clear sideways swipe changes session', () {
      expect(swipeStep(-120, 10), 1);
      expect(swipeStep(120, -20), -1);
      expect(swipeStep(-60, 0), 0);
      expect(swipeStep(-100, 80), 0);
    });

    test('Ctrl + letter', () {
      expect(controlByte('c'), 3);
      expect(controlByte('C'), 3);
      expect(controlByte(' '), 0);
      expect(controlByte('['), 27);
      expect(controlByte('1'), isNull);
    });
  });

  testWidgets('15b: a message held by an open prompt says so (queued or retrying)', (WidgetTester tester) async {
    for (final OutboxState st in <OutboxState>[OutboxState.queued, OutboxState.sending]) {
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: OutboxBubble(
            item: OutboxItem(
              id: 'x',
              host: 'h',
              session: 's',
              text: 'sigue',
              createdAtMs: 1,
              state: st,
              waitCode: WaitCode.promptOpen,
              lastError: kWaitingForPrompt,
            ),
            blocked: false,
            onRetry: () {},
            onEdit: () {},
            onDiscard: () {},
            onArrived: () {},
          ),
        ),
      ));
      expect(find.text('Esperando a que respondas el permiso'), findsOneWidget, reason: st.name);
    }
  });

  testWidgets('an uncertain message offers Sí llegó, Reenviar, Editar and an inline Descartar', (WidgetTester tester) async {
    int discarded = 0;
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: OutboxBubble(
          item: OutboxItem(id: 'x', host: 'h', session: 's', text: 'hola', createdAtMs: 1, state: OutboxState.uncertain),
          blocked: false,
          onRetry: () {},
          onEdit: () {},
          onDiscard: () => discarded++,
          onArrived: () {},
        ),
      ),
    ));
    for (final String label in <String>['Sí llegó', 'Reenviar', 'Editar', 'Descartar']) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    await tester.tap(find.text('Descartar'));
    await tester.pump();
    expect(discarded, 0, reason: 'asks first');
    await tester.tap(find.text('Descartar'));
    await tester.pump();
    expect(discarded, 1);
  });

  group('SessionView', () {
    late Directory dir;
    late FakeTermInput term;
    final List<(String, Map<String, Object?>)> calls = <(String, Map<String, Object?>)>[];
    late List<Map<String, Object?>> messages;
    Map<String, Object?>? ask;
    late List<Map<String, Object?>> stateRows;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('session_view');
      calls.clear();
      savedPanes.clear();
      ask = null;
      messages = <Map<String, Object?>>[
        <String, Object?>{'r': 'u', 't': 'primera pregunta'},
        <String, Object?>{'r': 'a', 't': 'Respuesta vieja'},
        <String, Object?>{'r': 'u', 't': '¿ya abrió algún PR?'},
        <String, Object?>{'r': 't', 't': 'Bash · gh pr list'},
        <String, Object?>{'r': 'a', 't': '## Lo que ya corre\n\n- **Conversaciones:** cada 5 minutos.\n- Errores en `pagos-api`.'},
      ];
      stateRows = <Map<String, Object?>>[
        <String, Object?>{'session': 'term-r1', 'pane': '%1', 'agent': 'claude', 'model': 'claude-fable-5-2', 'status': 'waiting', 'alive': true, 'paneActive': true},
        <String, Object?>{'session': 'term-r1', 'pane': '%2', 'agent': 'claude', 'model': 'claude-fable-5-2', 'status': 'waiting', 'alive': true},
      ];
      term = FakeTermInput();
      TermKeys.debugInput = (Connection _) => term;
      sl.reset();
      final MockClient client = MockClient((http.Request r) async {
        final Map<String, Object?> body = r.body.isEmpty ? <String, Object?>{} : jsonDecode(r.body) as Map<String, Object?>;
        calls.add((r.url.path, body));
        switch (r.url.path) {
          case '/state':
            return _json(stateRows);
          case '/chat/transcript':
            return _json(<String, Object?>{
              'ok': true, 'agent': 'claude', 'pane': body['pane'] ?? '%1', 'token': 't-${messages.length}', 'age': 500,
              'messages': messages,
              'ask': ask,
            });
          case '/session-seen' || '/key':
            return _json(<String, Object?>{'ok': true});
          case '/session/message':
            return _json(<String, Object?>{'ok': true, 'status': 'delivered', 'duplicate': false});
        }
        return http.Response('{"error":"Not found"}', 404);
      });
      final MemoryCredentialStore creds = MemoryCredentialStore()
        ..value = const PairedServer(host: 'mi-equipo.mi-red.ts.net', token: 'TOKEN');
      final Connection connection = Connection(creds, apiFactory: (PairedServer s) => ComandosApi(s, client: client));
      await connection.load();
      final Outbox outbox = Outbox(
        store: MemoryOutboxStore(),
        transport: ApiMessageTransport(() => connection.api),
        timer: (Duration d, void Function() cb) => _NoTimer(),
      );
      await outbox.load();
      outbox.setHost('mi-equipo.mi-red.ts.net');
      sl
        ..register<ErrorReporter>(ErrorReporter())
        ..register<AppVisibility>(AppVisibility())
        ..register<Connection>(connection)
        ..register<Outbox>(outbox)
        ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'));
    });

    tearDown(() async {
      TermKeys.debugInput = null;
      sl.reset();
      await dir.delete(recursive: true);
    });

    Future<void> settle(WidgetTester tester) async {
      for (int i = 0; i < 4; i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
        await tester.pump();
      }
    }

    Future<void> pumpView(WidgetTester tester, {bool terminal = false, VoidCallback? onNext}) async {
      tester.view.physicalSize = const Size(412 * 2, 915 * 2);
      tester.view.devicePixelRatio = 2;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(MaterialApp(
        theme: buildTheme(),
        navigatorKey: AppNavigator.key,
        navigatorObservers: <NavigatorObserver>[AppNavigator.routes],
        home: Scaffold(
          body: SafeArea(
            child: SessionView(session: 'term-r1', title: 'Tienda', subtitle: 'fable-5-2 +1', terminal: terminal, onNext: onNext),
          ),
        ),
      ));
      await settle(tester);
    }

    Future<void> leave(WidgetTester tester) async {
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 6));
    }

    testWidgets('head, folded feed with the last answer, Copiar / ✎ Editar, dock', (WidgetTester tester) async {
      await pumpView(tester);
      expect(find.text('Tienda'), findsOneWidget);
      expect(find.text('fable-5-2 +1'), findsOneWidget);
      expect(find.text('Lo que ya corre'), findsOneWidget);
      // Scrolled up: the pill that unfolds the older messages (screenshot 03).
      await tester.ensureVisible(find.text('Ver 4 mensajes anteriores', skipOffstage: false));
      await tester.pump();
      expect(find.text('Ver 4 mensajes anteriores'), findsOneWidget);
      expect(find.textContaining('Conversaciones:'), findsOneWidget);
      expect(find.text('primera pregunta'), findsNothing, reason: 'folded behind the pill');
      expect(find.text('Copiar'), findsWidgets);
      expect(find.text('✎ Editar'), findsOneWidget);

      // Pane chips (a split) and the composer naming its target.
      expect(find.text('Claude · fable-5.2'), findsNWidgets(2));
      expect(find.text('Escribe a 1 · Claude · fable-5.2…'), findsOneWidget);
      // The web's keys, in its order.
      final List<String> labels = <String>[
        for (final Element e in find.byType(DockKeyButton).evaluate()) (e.widget as DockKeyButton).label,
      ];
      const List<String> web = <String>[
        'Esc', 'Ctrl+C', 'Seleccionar', 'Paneles', '⌫', 'Historial', 'Copiar', 'Pegar', 'Teclado directo', 'Ctrl', 'Tab',
        'Izquierda', 'Arriba', 'Abajo', 'Derecha', '1', '2',
      ];
      final int n = labels.length < web.length ? labels.length : web.length;
      expect(n, greaterThan(2));
      expect(labels.sublist(0, n), web.sublist(0, n));

      await tester.tap(find.text('Ver 4 mensajes anteriores'));
      await tester.pump();
      expect(find.text('primera pregunta'), findsOneWidget);
      expect(find.text('Ocultar mensajes anteriores'), findsOneWidget);
      await leave(tester);
    });

    testWidgets('✎ Editar edits only here: Listo keeps it, Ver original restores', (WidgetTester tester) async {
      await pumpView(tester);
      await tester.ensureVisible(find.text('✎ Editar'));
      await tester.pump();
      await tester.tap(find.text('✎ Editar'));
      await tester.pump();
      expect(find.text('Copiar editado'), findsOneWidget);
      expect(find.text('Listo'), findsOneWidget);
      expect(find.text('Deshacer'), findsOneWidget);
      final Finder editor = find.descendant(of: find.byType(SelectionArea), matching: find.byType(TextField));
      await tester.enterText(editor, 'Texto corregido');
      await tester.ensureVisible(find.text('Listo'));
      await tester.pump();
      await tester.tap(find.text('Listo'));
      await tester.pump();
      expect(find.text('Texto corregido'), findsOneWidget);
      expect(find.text('Editado solo aquí'), findsOneWidget);
      await tester.ensureVisible(find.text('Ver original'));
      await tester.pump();
      await tester.tap(find.text('Ver original'));
      await tester.pump();
      expect(find.text('Lo que ya corre'), findsOneWidget);
      expect(find.text('Editado solo aquí'), findsNothing);
      expect(calls.where(((String, Map<String, Object?>) c) => c.$1 == '/session/message'), isEmpty,
          reason: 'the agent never sees an edit');
      await leave(tester);
    });

    testWidgets('a pane chip switches what the chat reads and where it writes', (WidgetTester tester) async {
      await pumpView(tester);
      final Finder chip2 = find.byWidgetPredicate((Widget w) => w is PaneChip && w.number == 2);
      await tester.ensureVisible(chip2);
      await tester.pump();
      await tester.tap(chip2);
      await settle(tester);
      expect(calls.where(((String, Map<String, Object?>) c) => c.$1 == '/chat/transcript').last.$2['pane'], '%2');
      expect(find.text('Escribe a 2 · Claude · fable-5.2…'), findsOneWidget);
      await leave(tester);
    });

    testWidgets('send goes through the outbox; empty send is Enter; keys go to the right route', (WidgetTester tester) async {
      await pumpView(tester);
      final Finder composer = find.byType(TextField).last;
      await tester.enterText(composer, 'hola agente');
      await tester.pump();
      await tester.runAsync(() async {
        await tester.tap(find.byTooltip('Enviar; vacío envía Enter'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
        await sl.get<Outbox>().pump();
      });
      await settle(tester);
      final Map<String, Object?> sent = calls.firstWhere(((String, Map<String, Object?>) c) => c.$1 == '/session/message').$2;
      expect(sent['text'], 'hola agente');
      expect(sent['pane'], '%1');
      expect(tester.widget<TextField>(composer).controller!.text, isEmpty);

      calls.clear();
      await tester.tap(find.byTooltip('Enviar; vacío envía Enter'));
      await settle(tester);
      expect(calls.where(((String, Map<String, Object?>) c) => c.$1 == '/key').single.$2['key'], 'Enter');

      calls.clear();
      await tester.tap(find.text('Esc'));
      await settle(tester);
      expect(calls.firstWhere(((String, Map<String, Object?>) c) => c.$1 == '/key').$2,
          <String, Object?>{'session': 'term-r1', 'pane': '%1', 'key': 'Escape'});

      await tester.tap(find.text('Ctrl+C'));
      await settle(tester);
      expect(term.typed.single.bytes, '\x03');
      expect(term.typed.single.pane, '%1');
      expect(term.typed.single.split, isTrue);

      final Finder two = find.byWidgetPredicate((Widget w) => w is DockKeyButton && w.label == '2');
      await tester.dragUntilVisible(two, find.byType(DockKeys), const Offset(-200, 0));
      calls.clear();
      await tester.tap(two);
      await settle(tester);
      expect(calls.firstWhere(((String, Map<String, Object?>) c) => c.$1 == '/key').$2['key'], '2');
      await leave(tester);
    });

    testWidgets('Ctrl armed turns the next letter into Ctrl+letter', (WidgetTester tester) async {
      await pumpView(tester);
      final Finder ctrl = find.byWidgetPredicate((Widget w) => w is DockKeyButton && w.label == 'Ctrl');
      await tester.dragUntilVisible(ctrl, find.byType(DockKeys), const Offset(-200, 0));
      await tester.tap(ctrl);
      await tester.pump();
      final Finder composer = find.byType(TextField).last;
      await tester.enterText(composer, 'd');
      await settle(tester);
      expect(term.typed.single.bytes, '\x04');
      expect(tester.widget<TextField>(composer).controller!.text, isEmpty);
      await leave(tester);
    });

    testWidgets('a permission prompt: sending waits; the answer is bound to the prompt shown', (WidgetTester tester) async {
      ask = <String, Object?>{
        'question': 'Do you want to proceed?',
        'detail': <Object?>['Bash command', 'git push'],
        'options': <Object?>[
          <String, Object?>{'key': '1', 'label': 'Yes', 'on': true},
          <String, Object?>{'key': '2', 'label': 'No', 'on': false},
        ],
      };
      await pumpView(tester);
      expect(find.text('Do you want to proceed?'), findsOneWidget);
      expect(find.text('Responde primero el permiso de arriba'), findsOneWidget);

      // The prompt on the desktop changes before the tap lands: nothing is typed.
      ask = <String, Object?>{
        'question': 'Another question?',
        'options': <Object?>[
          <String, Object?>{'key': '1', 'label': 'Yes', 'on': true},
          <String, Object?>{'key': '2', 'label': 'No', 'on': false},
        ],
      };
      calls.clear();
      await tester.ensureVisible(find.byTooltip('1. Yes'));
      await tester.tap(find.byTooltip('1. Yes'));
      await settle(tester);
      expect(calls.where(((String, Map<String, Object?>) c) => c.$1 == '/key'), isEmpty);

      // The same prompt still open: digit, then Enter.
      ScaffoldMessenger.of(tester.element(find.byType(SessionView))).removeCurrentSnackBar();
      await tester.pump(const Duration(seconds: 3));
      await settle(tester);
      expect(find.text('Another question?'), findsOneWidget);
      calls.clear();
      ScaffoldMessenger.of(tester.element(find.byType(SessionView))).clearSnackBars();
      await tester.pumpAndSettle();
      await tester.drag(find.byType(CustomScrollView), const Offset(0, -3000));
      await tester.pumpAndSettle();
      await tester.runAsync(() async {
        await tester.tap(find.byTooltip('2. No'));
        await Future<void>.delayed(const Duration(milliseconds: 400));
      });
      await settle(tester);
      expect(<Object?>[for (final (String, Map<String, Object?>) c in calls) if (c.$1 == '/key') c.$2['key']], <Object?>['2', 'Enter']);
      await leave(tester);
    });
  });
}
