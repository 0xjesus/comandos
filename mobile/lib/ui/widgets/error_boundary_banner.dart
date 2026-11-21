import 'package:flutter/material.dart';

import '../../core/error_reporter.dart';
import '../../core/locator.dart';
import '../theme.dart';
import 'banner_strip.dart';

/// Overlays the global error banner at the top of every screen.
class ErrorBoundaryBanner extends StatelessWidget {
  const ErrorBoundaryBanner({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final ErrorReporter? reporter = sl.maybe<ErrorReporter>();
    if (reporter == null) return child;
    return Stack(
      children: <Widget>[
        child,
        ValueListenableBuilder<String?>(
          valueListenable: reporter.banner,
          builder: (BuildContext context, String? text, Widget? _) {
            if (text == null) return const SizedBox.shrink();
            return Positioned(
              left: 0,
              right: 0,
              top: 0,
              child: SafeArea(
                bottom: false,
                child: BannerStrip(
                  text: text,
                  color: Orbita.waiting,
                  icon: Icons.info_outline,
                  onClose: reporter.dismiss,
                ),
              ),
            );
          },
        ),
      ],
    );
  }
}

/// Replaces Flutter's red error screen with a quiet inline notice.
Widget quietErrorWidget(FlutterErrorDetails details) {
  return const Material(
    color: Colors.transparent,
    child: Padding(
      padding: EdgeInsets.all(8),
      child: Text(
        'No pude mostrar esta parte.',
        style: TextStyle(color: Orbita.faint, fontSize: 12),
      ),
    ),
  );
}
