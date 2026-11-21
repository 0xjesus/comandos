import 'dart:io';

import 'package:comandos_mobile/core/locator.dart';
import 'package:comandos_mobile/core/outbox/drafts.dart';
import 'package:comandos_mobile/core/outbox/outbox.dart';
import 'package:comandos_mobile/core/outbox/outbox_store.dart';
import 'package:comandos_mobile/core/outbox/message_transport.dart';
import 'package:comandos_mobile/features/chat/composer.dart';
import 'package:comandos_mobile/features/voice/dictation.dart';
import 'package:comandos_mobile/features/voice/voice.dart';
import 'package:comandos_mobile/features/voice/voice_services.dart';
import 'package:comandos_mobile/ui/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'voice_fakes.dart';

class _NoTransport implements MessageTransport {
  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError();
}

void main() {
  late Directory dir;
  late FakeRecorder rec;
  late TextEditingController text;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('composer');
    rec = FakeRecorder();
    text = TextEditingController(text: 'Revisa');
    sl.reset();
    final Outbox outbox = Outbox(store: MemoryOutboxStore(), transport: _NoTransport());
    await outbox.load();
    sl
      ..register<Outbox>(outbox)
      ..register<DraftStore>(DraftStore('${dir.path}/drafts.json'));
    registerVoice();
  });

  tearDown(() async {
    sl.reset();
    VoiceServices.instance = null;
    text.dispose();
    await dir.delete(recursive: true);
  });

  Future<void> pumpComposer(WidgetTester tester) async {
    await tester.pumpWidget(MaterialApp(
      theme: buildTheme(),
      home: Scaffold(
        body: Align(
          alignment: Alignment.bottomCenter,
          child: Composer(
            session: 's1',
            pane: () => '%1',
            baselineFor: (_) => 0,
            onSent: () {},
            controller: text,
          ),
        ),
      ),
    ));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
    await tester.pump();
  }

  testWidgets('with a key: mic → recording strip (time, level, Cancelar) → cancel', (WidgetTester tester) async {
    final VoiceServices v = VoiceServices.instance =
        testVoiceServices(dir: dir.path, key: 'sk-test-0000000000000000000000abcd', recorder: rec);
    // Its disk scan is memoised: run it in the real zone first.
    await tester.runAsync(() => v.dictation.load());
    await pumpComposer(tester);
    expect(find.byTooltip('Dictar'), findsOneWidget);

    // Real I/O (files) must run in the real zone: tap inside runAsync.
    await tester.runAsync(() async {
      await tester.tap(find.byTooltip('Dictar'));
      await waitFor(() => rec.path != null);
    });
    await tester.pump();
    expect(find.textContaining('Escuchando'), findsOneWidget);
    expect(find.byTooltip('Terminar dictado'), findsOneWidget);
    expect(find.byKey(const ValueKey<String>('dictation-level')), findsOneWidget);

    await tester.runAsync(() async {
      await v.dictation.cancel(); // what Cancelar calls
      await waitFor(() => v.dictation.phase == DictationPhase.idle);
    });
    await tester.pump();
    expect(find.textContaining('Escuchando'), findsNothing);
    expect(rec.cancels, 1);
    expect(text.text, 'Revisa');
    expect(sl.get<Outbox>().items, isEmpty, reason: 'dictation never sends by itself');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('no key: no mic', (WidgetTester tester) async {
    VoiceServices.instance = testVoiceServices(dir: dir.path);
    await pumpComposer(tester);
    expect(find.byTooltip('Dictar'), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });
}
