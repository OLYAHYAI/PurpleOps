plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    // Identifiant DISTINCT de l'appli : s'installe à côté, sans rien remplacer
    // ni accéder à ses clés Keystore.
    namespace = "com.jbr.purpleops.proto"
    compileSdk = 35
    defaultConfig {
        applicationId = "com.jbr.purpleops.proto"
        minSdk = 33
        targetSdk = 35
        versionCode = 1
        versionName = "proto-0.1"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildTypes {
        // Le build « release » non minifié sert à mesurer taille et démarrage
        // (signé par la clé debug de la CI : prototype jamais publié).
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}
