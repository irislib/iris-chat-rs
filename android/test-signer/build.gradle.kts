plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "to.iris.test.signer"
    compileSdk = 36
    defaultConfig {
        applicationId = "to.iris.test.signer"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "test-only"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

// The intentionally controllable test signer must never become a release APK.
androidComponents {
    beforeVariants(selector().withBuildType("release")) { it.enable = false }
}

dependencies {
    testImplementation(libs.junit)
}
