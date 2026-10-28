import org.gradle.api.artifacts.dsl.LockMode

// Plugin resolution happens before the project body: lock its classpath here.
buildscript {
    configurations.getByName("classpath").resolutionStrategy.activateDependencyLocking()
}

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.google.services) apply false
    alias(libs.plugins.kotlin.compose) apply false
}

allprojects {
    dependencyLocking {
        lockAllConfigurations()
        lockMode.set(LockMode.STRICT)
    }
    buildscript.configurations.configureEach {
        resolutionStrategy.activateDependencyLocking()
    }
}
