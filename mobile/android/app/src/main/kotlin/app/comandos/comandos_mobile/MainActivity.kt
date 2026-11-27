package app.comandos.comandos_mobile

import android.content.Context
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

/**
 * The only activity. Its Flutter engine comes from [EngineHolder] so the Dart
 * isolate can outlive the activity while "Avisos en segundo plano" is on
 * (the notice watcher runs in the main isolate; see ARCHITECTURE.md) or a
 * Modo auto call is on. Otherwise the engine dies with the activity as usual.
 */
class MainActivity : FlutterActivity() {
    override fun provideFlutterEngine(context: Context): FlutterEngine =
        EngineHolder.obtain(context)

    // No side effects here: the embedding also calls it while saving state.
    // A Modo auto call (CarService) keeps the engine too: the call lives in
    // Dart and must survive the activity with the screen off.
    override fun shouldDestroyEngineWithHost(): Boolean = !WatchService.isRunning && !CarService.running
}
