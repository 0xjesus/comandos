import 'package:comandos_mobile/features/shell/nav_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Widget _bar({VoidCallback? onCar}) => MaterialApp(
      home: Scaffold(
        body: NavKeys(
          panelOpen: false,
          chatMode: false,
          onPanel: () {},
          onNew: () {},
          onChat: () {},
          onOpen: () {},
          onCar: onCar,
        ),
      ),
    );

void main() {
  testWidgets('Modo auto has its own key in the top bar, one tap away', (WidgetTester tester) async {
    int opened = 0;
    await tester.pumpWidget(_bar(onCar: () => opened++));
    final Finder car = find.byKey(const ValueKey<String>('tab-car'));
    expect(car, findsOneWidget);
    expect(find.byTooltip('Modo auto'), findsOneWidget);
    await tester.tap(car);
    expect(opened, 1);
  });

  testWidgets('without Modo auto registered there is no key', (WidgetTester tester) async {
    await tester.pumpWidget(_bar());
    expect(find.byKey(const ValueKey<String>('tab-car')), findsNothing);
  });
}
