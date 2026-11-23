import 'package:comandos_mobile/features/shell/nav_bar.dart';
import 'package:comandos_mobile/features/shell/tabs_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final List<TabEntry> _tabs = <TabEntry>[
  for (int i = 0; i < 30; i++) TabEntry(session: 's$i', label: 'Sesión número $i'),
];

Widget _strip(String? active, {String? detail}) => MaterialApp(
      home: Scaffold(
        body: SizedBox(
          width: 400,
          child: TabChips(
            tabs: _tabs,
            feed: null,
            active: active,
            activeDetail: detail,
            onTap: (_) {},
            onClose: (_) {},
            onLongPress: (_) {},
          ),
        ),
      ),
    );

void main() {
  testWidgets('a far-off active tab is centred, with its split and model under the name',
      (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(400, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(_strip('s0'));
    await tester.pumpAndSettle();
    await tester.pumpWidget(_strip('s27', detail: '1 · Claude · opus-4.8'));
    await tester.pumpAndSettle();
    final Rect chip = tester.getRect(find.text('Sesión número 27'));
    expect(chip.center.dx, closeTo(200, 60), reason: 'the active chip sits in the middle');
    expect(find.text('1 · Claude · opus-4.8'), findsOneWidget);
  });

  testWidgets('the catch-up key shows and lights while on', (WidgetTester tester) async {
    int taps = 0;
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: NavKeys(
          panelOpen: false,
          chatMode: true,
          onPanel: () {},
          onNew: () {},
          onChat: () {},
          onOpen: () {},
          onCatchUp: () => taps++,
          catchUpOn: true,
        ),
      ),
    ));
    expect(find.byTooltip('Salir de Ponerse al día'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey<String>('tab-catchup')));
    expect(taps, 1);
  });
}
