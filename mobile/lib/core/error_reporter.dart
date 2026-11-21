import 'package:flutter/foundation.dart';

/// Global error boundary sink. Uncaught errors land here and the app shows
/// a small dismissible banner instead of a red screen. Never stores the
/// error text itself in the banner (it could contain request details).
class ErrorReporter {
  final ValueNotifier<String?> banner = ValueNotifier<String?>(null);

  void report(Object error, [StackTrace? stack]) {
    debugPrint('uncaught: ${error.runtimeType}');
    banner.value = 'Algo falló en la app, pero sigue funcionando. '
        'Si se repite, ciérrala y ábrela de nuevo.';
  }

  void dismiss() => banner.value = null;
}
