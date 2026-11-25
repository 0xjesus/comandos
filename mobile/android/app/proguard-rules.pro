# R8 rules for ComandOS. Flutter and the plugins ship their own consumer
# rules; the app's Kotlin code (NativeBridge, WatchService, ScanActivity…) is
# referenced from the manifest or registered explicitly, so R8 keeps it.

# Play Core is referenced by the Flutter embedding for deferred components,
# which this app does not use.
-dontwarn com.google.android.play.core.**

# Drop verbose/debug logging calls from library code.
-assumenosideeffects class android.util.Log {
    public static int v(...);
    public static int d(...);
}

# Vosk («OK ComandOS») calls libvosk through JNA, which reflects on these.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class org.vosk.** { *; }
-dontwarn java.awt.**
