package app.comandos.comandos_mobile

import android.content.Context
import io.flutter.FlutterInjector
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor

/**
 * Owns the app's single Flutter engine. There is never a second engine: the
 * activity and the background watcher share this one.
 */
object EngineHolder {
    @Volatile
    private var engine: FlutterEngine? = null

    /** For the activity: the live engine, or a new one the activity runs (`main`). */
    fun obtain(context: Context): FlutterEngine = engine ?: create(context)

    /**
     * For the watch service after a restart by the system or at boot: when no
     * engine is alive, starts one without UI running `watchMain` (it boots
     * the services and the notice watcher only; the UI starts if an activity
     * attaches later).
     */
    fun ensureHeadless(context: Context) {
        if (engine != null) return
        val loader = FlutterInjector.instance().flutterLoader()
        loader.startInitialization(context.applicationContext)
        loader.ensureInitializationComplete(context.applicationContext, null)
        val e = create(context)
        e.dartExecutor.executeDartEntrypoint(
            DartExecutor.DartEntrypoint(loader.findAppBundlePath(), "watchMain"),
        )
    }

    /** True while an activity shows this engine. */
    @Volatile
    var activityAttached: Boolean = false

    /** Releases the engine when nothing needs it (service stopped, no activity). */
    fun releaseIfIdle() {
        val e = engine ?: return
        if (!activityAttached && !WatchService.isRunning) e.destroy()
    }

    private fun create(context: Context): FlutterEngine {
        val e = FlutterEngine(context.applicationContext)
        e.plugins.add(NativeBridge())
        e.addEngineLifecycleListener(object : FlutterEngine.EngineLifecycleListener {
            override fun onPreEngineRestart() {}
            override fun onEngineWillDestroy() {
                if (engine === e) engine = null
            }
        })
        engine = e
        return e
    }
}
