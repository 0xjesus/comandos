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
}

flutter {
    source = "../.."
}
