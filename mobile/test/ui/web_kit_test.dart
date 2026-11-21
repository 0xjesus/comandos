import 'package:comandos_mobile/ui/theme.dart';
import 'package:comandos_mobile/ui/web_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('every icon parses into a path inside its 24×24 box', () {
    for (final MapEntry<String, WebIconData> e in WebIcons.byName.entries) {
      for (final String part in e.value.parts) {
        if (!part.contains('p:')) continue;
        final Path p = svgPath(part.substring(part.indexOf(':') + 1));
        final Rect b = p.getBounds();
        expect(b.isEmpty && b.width == 0 && b.height == 0 && !part.contains('h.01'), isFalse,
            reason: '${e.key}: $part');
        expect(b.left >= -1 && b.top >= -1 && b.right <= 27 && b.bottom <= 27, isTrue, reason: '${e.key}: $b');
      }
    }
  });

  test('path parser: relative commands, arcs with packed flags, implicit lines', () {
    final Rect r = svgPath('M4 6h16M4 12h16').getBounds();
    expect(r, const Rect.fromLTRB(4, 6, 20, 12));
    final Rect a = svgPath('M10.3 21a1.94 1.94 0 0 0 3.4 0').getBounds();
    expect(a.left, closeTo(10.3, .01));
    expect(a.right, closeTo(13.7, .01));
    final Rect m = svgPath('m15 18-6-6 6-6').getBounds();
    expect(m, const Rect.fromLTRB(9, 6, 15, 18));
  });

  testWidgets('WebButton: lip, on/brand inks, press sinks, tap fires', (WidgetTester tester) async {
    int taps = 0;
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Column(
          children: <Widget>[
            WebButton(onTap: () => taps++, size: 34, child: const WebIcon(WebIcons.menu)),
            const WebButton(on: true, size: 34, child: WebIcon(WebIcons.smartphone)),
            WebButton(brand: true, on: true, onTap: () {}, size: 42, child: const WebIcon(WebIcons.prompt)),
            const WebChip(style: WebChipStyle.filter, on: true, child: Text('Todas')),
            const WebSectionLabel('Ahora'),
          ],
        ),
      ),
    ));
    final List<WebIcon> icons = tester.widgetList<WebIcon>(find.byType(WebIcon)).toList();
    expect(icons, hasLength(3));
    IconThemeData theme(int i) => IconTheme.of(tester.element(find.byType(WebIcon).at(i)));
    expect(theme(0).color, Orbita.dim);
    expect(theme(1).color, Orbita.done);
    expect(theme(2).color, Colors.white);
    await tester.tap(find.byType(WebButton).first);
    await tester.pump(const Duration(milliseconds: 100));
    expect(taps, 1);
    expect(find.text('AHORA'), findsOneWidget);
  });

  testWidgets('showWebToast shows one toast and removes it after 3.5 s', (WidgetTester tester) async {
    await tester.pumpWidget(MaterialApp(
      home: Builder(
        builder: (BuildContext c) => TextButton(onPressed: () => showWebToast(c, 'Abre una sesión primero'), child: const Text('x')),
      ),
    ));
    await tester.tap(find.text('x'));
    await tester.pump();
    expect(find.text('Abre una sesión primero'), findsOneWidget);
    await tester.pump(const Duration(seconds: 4));
    expect(find.text('Abre una sesión primero'), findsNothing);
  });
}
