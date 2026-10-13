plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// versionCode / versionName pilotes par la CI (-PverCode / -PverName), 1 / 0.1.0 en local.
val verCode = (project.findProperty("verCode") as String?)?.toIntOrNull() ?: 1
val verName = (project.findProperty("verName") as String?) ?: "0.1.0"

android {
    namespace = "com.jbr.purpleops"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.jbr.purpleops"
        minSdk = 33
        targetSdk = 35
        versionCode = verCode
        versionName = verName

        // Repo source pour l'auto-update (API GitHub Releases).
        buildConfigField("String", "GITHUB_REPO", "\"OLYAHYAI/PurpleOps\"")
    }

    // Cle debug FIXE commitee : toutes les builds (local + CI) sont signees pareil,
    // donc une mise a jour s'installe par-dessus la precedente (pm install -r).
    // Note securite : cle debug (mdp "android"), convient a un outil homelab ; a
    // regenerer si l'app sort du labo.
    signingConfigs {
        getByName("debug") {
            storeFile = file("debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        getByName("debug") {
            signingConfig = signingConfigs.getByName("debug")
        }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}
