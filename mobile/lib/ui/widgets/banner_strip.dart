import 'package:flutter/material.dart';

import '../theme.dart';

/// A thin full-width notice («Sin conexión…»), styled like the web's `.alert`:
/// `--panel` strip, 3 px coloured left edge, 1 px `--line` underneath, 13 px text.
/// Tappable when [onTap] is set.
class BannerStrip extends StatelessWidget {
  const BannerStrip({
    super.key,
    required this.text,
    this.color = Orbita.err,
    this.icon = Icons.error_outline,
    this.onTap,
    this.onClose,
  });

  final String text;
  final Color color;

  /// Kept for callers; the web strip has no icon, so it is not drawn.
  final IconData icon;
  final VoidCallback? onTap;
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: onTap != null,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          decoration: BoxDecoration(
            color: Orbita.panel,
            border: Border(
              left: BorderSide(color: color, width: 3),
              bottom: const BorderSide(color: Orbita.line),
            ),
          ),
          padding: const EdgeInsets.only(left: 12, right: 4),
          child: Row(
            children: <Widget>[
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  child: Text(
                    text,
                    style: const TextStyle(fontFamily: Orbita.sans, color: Orbita.text, fontSize: 13, height: 1.35),
                  ),
                ),
              ),
              if (onClose != null)
                // No tooltip: this strip is also shown above the Navigator
                // (global error banner), where there is no Overlay.
                Semantics(
                  label: 'Cerrar aviso',
                  button: true,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: onClose,
                    child: const SizedBox(
                      width: 44,
                      height: 44,
                      child: Center(
                        child: Text('×', style: TextStyle(fontSize: 18, height: 1, color: Orbita.dim)),
                      ),
                    ),
                  ),
                )
              else
                const SizedBox(width: 8),
            ],
          ),
        ),
      ),
    );
  }
}
