import 'package:flutter/material.dart';

/// "Noche Órbita" palette from the desktop app.
abstract final class Orbita {
  static const Color bg = Color(0xFF0A0D13);
  static const Color bar = Color(0xFF0D1119);
  static const Color panel = Color(0xFF121722);
  static const Color panel2 = Color(0xFF161C29);
  static const Color line = Color(0xFF222A3A);
  static const Color line2 = Color(0xFF2E3852);
  static const Color text = Color(0xFFEAF0FB);
  static const Color dim = Color(0xFF9AA6BF);
  static const Color faint = Color(0xFF5E6980);
  static const Color brand = Color(0xFF8B7CFF);
  static const Color waiting = Color(0xFFFFAE1A);
  static const Color done = Color(0xFF2EE59D);
  static const Color working = Color(0xFF7AA5FF);
  static const Color err = Color(0xFFFF7580);

  static const Color onBrand = Color(0xFF080B11);
  static const Color glow = Color(0xFF141B2E);
  static const Color code = Color(0xFFA3C0FF);

  /// Same faces as the web dashboard: `--sans` and `--mono` (bundled).
  static const String sans = 'Inter';
  static const String mono = 'UbuntuSansMono';
}

/// Minimum touch target.
const double kTouch = 48;

ThemeData buildTheme() {
  const ColorScheme scheme = ColorScheme(
    brightness: Brightness.dark,
    primary: Orbita.brand,
    onPrimary: Color(0xFF0A0D13),
    secondary: Orbita.working,
    onSecondary: Color(0xFF0A0D13),
    error: Orbita.err,
    onError: Color(0xFF0A0D13),
    surface: Orbita.bg,
    onSurface: Orbita.text,
    surfaceContainerLowest: Orbita.bg,
    surfaceContainerLow: Orbita.bar,
    surfaceContainer: Orbita.panel,
    surfaceContainerHigh: Orbita.panel2,
    surfaceContainerHighest: Orbita.line,
    onSurfaceVariant: Orbita.dim,
    outline: Orbita.line2,
    outlineVariant: Orbita.line,
  );
  final ThemeData base = ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    brightness: Brightness.dark,
    fontFamily: Orbita.sans,
    scaffoldBackgroundColor: Orbita.bg,
    canvasColor: Orbita.bg,
    // Ripple is cheaper than the sparkle shader (no first-tap compile jank).
    splashFactory: InkRipple.splashFactory,
    materialTapTargetSize: MaterialTapTargetSize.padded,
    visualDensity: VisualDensity.standard,
  );
  return base.copyWith(
    appBarTheme: const AppBarTheme(
      backgroundColor: Orbita.bar,
      foregroundColor: Orbita.text,
      elevation: 0,
      scrolledUnderElevation: 0,
      centerTitle: false,
    ),
    dividerTheme: const DividerThemeData(color: Orbita.line, thickness: 1, space: 1),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Orbita.panel,
      hintStyle: const TextStyle(color: Orbita.faint),
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Orbita.line),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Orbita.line),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(14),
        borderSide: const BorderSide(color: Orbita.brand),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(kTouch, kTouch),
        backgroundColor: Orbita.brand,
        foregroundColor: Orbita.bg,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(kTouch, kTouch),
        foregroundColor: Orbita.text,
        side: const BorderSide(color: Orbita.line2),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        minimumSize: const Size(kTouch, kTouch),
        foregroundColor: Orbita.brand,
      ),
    ),
    snackBarTheme: const SnackBarThemeData(
      backgroundColor: Orbita.panel2,
      contentTextStyle: TextStyle(color: Orbita.text),
      behavior: SnackBarBehavior.floating,
    ),
    progressIndicatorTheme: const ProgressIndicatorThemeData(color: Orbita.brand),
    listTileTheme: const ListTileThemeData(iconColor: Orbita.dim, minVerticalPadding: 12),
  );
}
