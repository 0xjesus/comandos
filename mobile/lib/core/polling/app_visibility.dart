import 'package:flutter/foundation.dart';

/// True while the app is in the foreground (resumed). Pollers listen to it
/// and stop when it turns false. Set by the lifecycle listener in `app.dart`.
class AppVisibility extends ValueNotifier<bool> {
  AppVisibility() : super(true);
}
