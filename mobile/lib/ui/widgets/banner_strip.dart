import 'package:flutter/material.dart';

import '../theme.dart';

/// A thin full-width notice ("Sin conexión…"). Tappable when [onTap] is set.
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
  final IconData icon;
  final VoidCallback? onTap;
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: color.withValues(alpha: 0.12),
      child: InkWell(
        onTap: onTap,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: kTouch),
          child: Padding(
            padding: const EdgeInsets.only(left: 16, right: 4),
            child: Row(
              children: <Widget>[
                Icon(icon, size: 18, color: color),
                const SizedBox(width: 10),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: 8),
                    child: Text(text, style: const TextStyle(color: Orbita.text, fontSize: 14)),
                  ),
                ),
                if (onClose != null)
                  // No tooltip: this strip is also shown above the Navigator
                  // (global error banner), where there is no Overlay.
                  Semantics(
                    label: 'Cerrar aviso',
                    button: true,
                    child: IconButton(
                      onPressed: onClose,
                      icon: const Icon(Icons.close, size: 18, color: Orbita.dim),
                    ),
                  )
                else
                  const SizedBox(width: 12),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
