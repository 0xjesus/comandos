import 'package:flutter/widgets.dart';

import '../../ui/theme.dart';
import '../../ui/web_kit.dart';

/// Loading state over a page that is still opening: the page background with
/// one faint line, like the web while a panel fills in.
class WebLoading extends StatelessWidget {
  const WebLoading({super.key, this.text = 'Cargando…'});

  final String text;

  @override
  Widget build(BuildContext context) => ColoredBox(
        color: Orbita.bg,
        child: Center(
          child: Text(
            text,
            style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, color: Orbita.faint),
          ),
        ),
      );
}

/// Error state with «Reintentar» (copy and look of the web's own errors:
/// `--dim` text, one raised button).
class WebError extends StatelessWidget {
  const WebError({super.key, required this.message, required this.onRetry, this.detail});

  final String message;
  final String? detail;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => ColoredBox(
        color: Orbita.bg,
        child: Center(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                Text(
                  message,
                  textAlign: TextAlign.center,
                  style: const TextStyle(
                      fontFamily: Orbita.sans, fontSize: 14, fontWeight: FontWeight.w600, color: Orbita.text),
                ),
                if (detail != null) ...<Widget>[
                  const SizedBox(height: 6),
                  Text(
                    detail!,
                    textAlign: TextAlign.center,
                    style: const TextStyle(fontFamily: Orbita.sans, fontSize: 13, height: 1.4, color: Orbita.dim),
                  ),
                ],
                const SizedBox(height: 16),
                WebButton(
                  onTap: onRetry,
                  padding: const EdgeInsets.symmetric(horizontal: 18),
                  child: const Text('Reintentar'),
                ),
              ],
            ),
          ),
        ),
      );
}
