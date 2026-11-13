pluginManagement {
    repositories {
        google()
        // Gradle Plugin Portal serves a different POM for this marker.
        // Resolve only the Maven Central artifact covered by our pinned digest.
        exclusiveContent {
            forRepository { mavenCentral() }
            filter {
                includeModule("org.jetbrains.kotlin.plugin.compose", "org.jetbrains.kotlin.plugin.compose.gradle.plugin")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "Iris Chat"
include(":app")
include(":fips-ble")
include(":test-signer")
project(":fips-ble").projectDir = file("fips-ble")
