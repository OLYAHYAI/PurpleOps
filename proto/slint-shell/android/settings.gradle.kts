// Projet Gradle SÉPARÉ de l'appli PurpleOps : le prototype ne doit jamais
// entrer dans le build ni dans les Releases de l'appli.
pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories { google(); mavenCentral() }
}
rootProject.name = "PurpleOpsSlintProto"
include(":app")
