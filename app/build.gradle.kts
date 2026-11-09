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

    // Signature : cle de RELEASE privee, jamais dans le depot. La CI la fournit
    // par variables d'environnement (secrets GitHub) :
    //   PO_KEYSTORE (chemin du .jks decode), PO_KEYSTORE_PASS, PO_KEY_ALIAS, PO_KEY_PASS.
    // Pourquoi : sur Android, une mise a jour signee par la meme cle herite de
    // l'UID de l'appli, donc de ses cles Keystore (secrets, future cle
    // d'extinction). L'ancienne cle debug etait publique (commitee, mdp
    // "android") : elle ne protegeait plus rien. Sans ces variables (build
    // local), le type release n'est pas signe et la CI echoue volontairement.
    val ksPath = System.getenv("PO_KEYSTORE")
    signingConfigs {
        if (!ksPath.isNullOrBlank()) {
            create("release") {
                storeFile = file(ksPath)
                storePassword = System.getenv("PO_KEYSTORE_PASS")
                keyAlias = System.getenv("PO_KEY_ALIAS")
                keyPassword = System.getenv("PO_KEY_PASS")
                enableV1Signing = false
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }

    buildTypes {
        // debug = builds locales et verification des PR (cle debug propre a la
        // machine, generee par Gradle) ; jamais publie.
        release {
            // Pas de minification : l'aide Java ScreenHelper est lancee par
            // app_process via son nom de classe.
            isMinifyEnabled = false
            isDebuggable = false
            if (!ksPath.isNullOrBlank()) signingConfig = signingConfigs.getByName("release")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    // p900ctl (module camera) est un executable range en lib*.so : il doit etre
    // extrait sur le disque a l'installation et laisse intact.
    packaging {
        jniLibs {
            useLegacyPackaging = true
            keepDebugSymbols += "**/libp900ctl.so"
        }
        // JSch est un JAR multi-release (classes Java 9+ sous META-INF/versions) :
        // Android n'en a pas l'usage.
        resources {
            excludes += "META-INF/versions/**"
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
    // Client SSH pour le réveil des nœuds via wol-pi (fork maintenu de JSch, BSD,
    // sans dépendance). La clé privée reste dans l'Android Keystore (WolKey).
    implementation("com.github.mwiede:jsch:2.28.8")
}
