import java.net.URI
import java.security.MessageDigest

plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "app.comandos.comandos_mobile"
    compileSdk = 36
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_17.toString()
    }

    defaultConfig {
        applicationId = "app.comandos.comandos_mobile"
        minSdk = 26
        targetSdk = 36
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    // Only Spanish (the app's language) and English resources from libraries.
    androidResources {
        localeFilters += listOf("es", "en")
    }

    packaging {
        resources {
            // Library version stamps and the coroutines debug agent: unused at runtime.
            excludes += listOf("META-INF/*.version", "DebugProbesKt.bin")
        }
    }

    buildTypes {
        release {
            // Personal sideload: release is signed with the local debug keystore
            // (~/.android/debug.keystore, created by the Android tooling), so
            // `flutter build apk --release` needs no extra files. Replace with a
            // real upload key before distributing outside this machine.
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

dependencies {
    // QR decoding for the pairing scanner (ScanActivity). R8 keeps only the QR reader.
    implementation("com.google.zxing:core:3.5.3")
    // «OK ComandOS» in Modo auto: offline speech recognition (WakeWord.kt).
    implementation("com.alphacephei:vosk-android:0.3.75")
    implementation("net.java.dev.jna:jna:5.18.1@aar")
    // Plain JVM tests of pure Kotlin pieces (TakeEndpointer).
    testImplementation("junit:junit:4.13.2")
}

// The wake phrase runs on Vosk's small Spanish model (Apache-2.0, ~40 MB). It
// is fetched once at build time into android/.vosk-cache (git-ignored), checked
// against its SHA-256 and packed as the assets folder `vosk-es`: never
// committed, never downloaded by the phone.
val voskModel = "vosk-model-small-es-0.42"
val voskSha256 = "09b239888f633ef2f0b4e09736e3d9936acfd810bc65d53fad45261762c6511f"
val voskAssets = layout.buildDirectory.dir("generated/vosk/assets")
val fetchVosk by tasks.registering {
    inputs.property("sha256", voskSha256)
    outputs.dir(voskAssets)
    doLast {
        val zip = rootProject.file(".vosk-cache/$voskModel.zip")
        fun sha(f: File): String = MessageDigest.getInstance("SHA-256").let { md ->
            f.inputStream().use { s ->
                val b = ByteArray(1 shl 16)
                while (true) {
                    val n = s.read(b)
                    if (n < 0) break
                    md.update(b, 0, n)
                }
            }
            md.digest().joinToString("") { "%02x".format(it) }
        }
        if (!zip.exists() || sha(zip) != voskSha256) {
            zip.parentFile.mkdirs()
            URI("https://alphacephei.com/vosk/models/$voskModel.zip").toURL().openStream().use { i ->
                zip.outputStream().use { o -> i.copyTo(o) }
            }
            check(sha(zip) == voskSha256) { "$voskModel.zip: SHA-256 does not match" }
        }
        val out = voskAssets.get().asFile
        out.deleteRecursively()
        val tmp = File(out, "tmp")
        project.copy {
            from(zipTree(zip))
            into(tmp)
        }
        val dir = File(out, "vosk-es")
        File(tmp, voskModel).renameTo(dir)
        tmp.deleteRecursively()
        // StorageService copies the model out of the APK again only when this changes.
        File(dir, "uuid").writeText(voskSha256.take(16) + "\n")
    }
}
android.sourceSets.getByName("main").assets.srcDir(voskAssets)
tasks.named("preBuild") { dependsOn(fetchVosk) }

flutter {
    source = "../.."
}
