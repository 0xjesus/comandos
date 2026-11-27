import 'package:flutter/widgets.dart';

import '../../ui/web_kit.dart';

// `logo(id)` of crates/comandos-web-view/src/command_sidebar.rs: the coloured
// provider logos of the «Consumos» cards, painted from the same SVG data.

/// True when [provider] has a painted logo (otherwise the card shows the
/// first letter of the CLI name).
bool hasUsageLogo(String provider) =>
    const <String>{'claude', 'codex', 'grok', 'opencode', 'agy', 'omp'}.contains(provider);

/// The logo of [provider] at [size] (14 in the cards).
class UsageLogo extends StatelessWidget {
  const UsageLogo(this.provider, {super.key, this.size = 14, this.ink = const Color(0xFFEAF0FB)});

  final String provider;
  final double size;

  /// `currentColor` (Grok's logo).
  final Color ink;

  @override
  Widget build(BuildContext context) => SizedBox.square(
        dimension: size,
        child: CustomPaint(painter: _LogoPainter(provider, ink)),
      );
}

const String _claude =
    'M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z';

const String _codexTile =
    'M19.503 0H4.496A4.496 4.496 0 000 4.496v15.007A4.496 4.496 0 004.496 24h15.007A4.496 4.496 0 0024 19.503V4.496A4.496 4.496 0 0019.503 0z';

const String _codexMark =
    'M9.064 3.344a4.578 4.578 0 012.285-.312c1 .115 1.891.54 2.673 1.275.01.01.024.017.037.021a.09.09 0 00.043 0 4.55 4.55 0 013.046.275l.047.022.116.057a4.581 4.581 0 012.188 2.399c.209.51.313 1.041.315 1.595a4.24 4.24 0 01-.134 1.223.123.123 0 00.03.115c.594.607.988 1.33 1.183 2.17.289 1.425-.007 2.71-.887 3.854l-.136.166a4.548 4.548 0 01-2.201 1.388.123.123 0 00-.081.076c-.191.551-.383 1.023-.74 1.494-.9 1.187-2.222 1.846-3.711 1.838-1.187-.006-2.239-.44-3.157-1.302a.107.107 0 00-.105-.024c-.388.125-.78.143-1.204.138a4.441 4.441 0 01-1.945-.466 4.544 4.544 0 01-1.61-1.335c-.152-.202-.303-.392-.414-.617a5.81 5.81 0 01-.37-.961 4.582 4.582 0 01-.014-2.298.124.124 0 00.006-.056.085.085 0 00-.027-.048 4.467 4.467 0 01-1.034-1.651 3.896 3.896 0 01-.251-1.192 5.189 5.189 0 01.141-1.6c.337-1.112.982-1.985 1.933-2.618.212-.141.413-.251.601-.33.215-.089.43-.164.646-.227a.098.098 0 00.065-.066 4.51 4.51 0 01.829-1.615 4.535 4.535 0 011.837-1.388zm3.482 10.565a.637.637 0 000 1.272h3.636a.637.637 0 100-1.272h-3.636zM8.462 9.23a.637.637 0 00-1.106.631l1.272 2.224-1.266 2.136a.636.636 0 101.095.649l1.454-2.455a.636.636 0 00.005-.64L8.462 9.23z';

const String _grok =
    'M9.27 15.29l7.978-5.897c.391-.29.95-.177 1.137.272.98 2.369.542 5.215-1.41 7.169-1.951 1.954-4.667 2.382-7.149 1.406l-2.711 1.257c3.889 2.661 8.611 2.003 11.562-.953 2.341-2.344 3.066-5.539 2.388-8.42l.006.007c-.983-4.232.242-5.924 2.75-9.383.06-.082.12-.164.179-.248l-3.301 3.305v-.01L9.267 15.292M7.623 16.723c-2.792-2.67-2.31-6.801.071-9.184 1.761-1.763 4.647-2.483 7.166-1.425l2.705-1.25a7.808 7.808 0 00-1.829-1A8.975 8.975 0 005.984 5.83c-2.533 2.536-3.33 6.436-1.962 9.764 1.022 2.487-.653 4.246-2.34 6.022-.599.63-1.199 1.259-1.682 1.925l7.62-6.815';

/// Oh My Pi (`dash/icons/omp.svg`): a π stroked in `currentColor`.
const List<String> _omp = <String>[
  'M4.5 8c1-1.6 2.4-2.5 4.2-2.5H19.5',
  'M9.5 5.5v13',
  'M15.5 5.5v9.5a3 3 0 0 0 3 3',
];

const String _agyMask =
    'M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z';

class _LogoPainter extends CustomPainter {
  _LogoPainter(this.provider, this.ink);

  final String provider;
  final Color ink;

  static final Map<String, Path> _paths = <String, Path>{};
  static Path _p(String d) => _paths[d] ??= svgPath(d);

  @override
  void paint(Canvas canvas, Size size) {
    final Paint paint = Paint()..isAntiAlias = true;
    canvas.save();
    switch (provider) {
      case 'claude':
        canvas.scale(size.width / 24);
        canvas.drawPath(_p(_claude), paint..color = const Color(0xFFD97757));
      case 'codex':
        canvas.scale(size.width / 24);
        canvas.drawPath(_p(_codexTile), paint..color = const Color(0xFFFFFFFF));
        paint.shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: <Color>[Color(0xFFB1A7FF), Color(0xFF7A9DFF), Color(0xFF3941FF)],
          stops: <double>[0, .5, 1],
        ).createShader(const Rect.fromLTRB(0, 3, 24, 21));
        canvas.drawPath(_p(_codexMark)..fillType = PathFillType.nonZero, paint);
      case 'grok':
        canvas.scale(size.width / 24);
        canvas.drawPath(_p(_grok)..fillType = PathFillType.evenOdd, paint..color = ink);
      case 'opencode':
        canvas.scale(size.width / 512);
        canvas.drawRRect(
          RRect.fromRectAndRadius(const Rect.fromLTWH(0, 0, 512, 512), const Radius.circular(96)),
          paint..color = const Color(0xFF131010),
        );
        canvas.drawRect(const Rect.fromLTRB(192, 224, 320, 352), paint..color = const Color(0xFF5A5858));
        final Path frame = Path()
          ..fillType = PathFillType.evenOdd
          ..addRect(const Rect.fromLTRB(128, 96, 384, 416))
          ..addRect(const Rect.fromLTRB(192, 160, 320, 352));
        canvas.drawPath(frame, paint..color = const Color(0xFFFFFFFF));
      case 'omp':
        canvas.scale(size.width / 24);
        final Paint stroke = Paint()
          ..isAntiAlias = true
          ..color = ink
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..strokeCap = StrokeCap.round
          ..strokeJoin = StrokeJoin.round;
        for (final String d in _omp) {
          canvas.drawPath(_p(d), stroke);
        }
      case 'agy':
        // The Antigravity arch filled with Google's blurred colour blobs.
        canvas.scale(size.width / 24);
        canvas.clipPath(_p(_agyMask));
        const List<(Offset, double, Color)> blobs = <(Offset, double, Color)>[
          (Offset(2, 6), 9, Color(0xFF00B95C)),
          (Offset(12, -2), 8, Color(0xFFFBBC04)),
          (Offset(6, -1), 5, Color(0xFFFFE432)),
          (Offset(22, 4), 8, Color(0xFFFC413D)),
          (Offset(23, 13), 4, Color(0xFF749BFF)),
          (Offset(16, 24), 9, Color(0xFF3186FF)),
          (Offset(2, 3), 3.5, Color(0xFFFFEE48)),
        ];
        for (final (Offset c, double r, Color color) in blobs) {
          canvas.drawCircle(
            c,
            r,
            Paint()
              ..color = color
              ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 2.5),
          );
        }
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_LogoPainter old) => old.provider != provider || old.ink != ink;
}
