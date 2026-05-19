import java.time.Instant
import java.util.Properties
import org.gradle.api.tasks.testing.Test
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.google.services)
    alias(libs.plugins.kotlin.compose)
}

val ndkVersionValue = "28.2.13676358"
val rustAppDir = rootProject.file("../core")
val rustManifestPath = rustAppDir.resolve("Cargo.toml")
val rustSourceDir = rustAppDir.resolve("src")
val rustPathDependencyDirs =
    listOf(rootProject.file("../../nostr-double-ratchet/rust/crates"))
        .filter { it.exists() }
val generatedJniDir = layout.buildDirectory.dir("generated/jniLibs")
val generatedUniffiDir = layout.buildDirectory.dir("generated/source/uniffi/main/java")
val localProperties =
    Properties().apply {
        val file = rootProject.file("local.properties")
        if (file.exists()) {
            file.inputStream().use(::load)
        }
    }
val releaseEnvProperties =
    Properties().apply {
        val file = rootProject.file("../release.env")
        if (file.exists()) {
            file.inputStream().use(::load)
        }
    }
val androidSdkDir =
    localProperties.getProperty("sdk.dir")
        ?: System.getenv("ANDROID_HOME")
        ?: System.getenv("ANDROID_SDK_ROOT")
        ?: error("Android SDK path was not found. Define sdk.dir in android/local.properties.")
val androidAppId = "to.iris.chat"
val androidNdkDir = file("$androidSdkDir/ndk/$ndkVersionValue")
val cargoBinary = file("${System.getProperty("user.home")}/.cargo/bin/cargo")

// Keep Rust artifacts out of the repo tree even when a long-lived Gradle daemon
// started without the user's shell environment. Override with -Pcargo.targetDir
// or android/local.properties: cargo.targetDir=/path/to/target.
fun nonBlankBuildValue(propertyName: String, envName: String): String? =
    providers.gradleProperty(propertyName).orNull?.takeIf { it.isNotBlank() }
        ?: localProperties.getProperty(propertyName)?.takeIf { it.isNotBlank() }
        ?: providers.environmentVariable(envName).orNull?.takeIf { it.isNotBlank() }
        ?: System.getenv(envName)?.takeIf { it.isNotBlank() }

fun pathWithExpandedHome(rawPath: String): String {
    val home = System.getProperty("user.home")
    return when {
        rawPath == "~" -> home
        rawPath.startsWith("~/") -> "$home/${rawPath.removePrefix("~/")}"
        else -> rawPath
    }
}

fun rustRelativeOrAbsoluteFile(rawPath: String) =
    file(pathWithExpandedHome(rawPath)).let { candidate ->
        if (candidate.isAbsolute) candidate else rustAppDir.resolve(candidate.path)
    }

val cargoTargetDir =
    rustRelativeOrAbsoluteFile(
        nonBlankBuildValue("cargo.targetDir", "CARGO_TARGET_DIR")
            ?: "${System.getProperty("user.home")}/.cache/cargo-target",
    )
val publicRelayFallbackCsv = "wss://relay.damus.io,wss://nos.lol,wss://relay.primal.net,wss://relay.snort.social,wss://temp.iris.to"

fun configValue(propertyName: String, envName: String): String? =
    localProperties.getProperty(propertyName)?.takeIf { it.isNotBlank() }
        ?: System.getenv(envName)?.takeIf { it.isNotBlank() }
        ?: releaseEnvProperties.getProperty(envName)
            ?.trim()
            ?.removeSurrounding("\"")
            ?.takeIf { it.isNotBlank() }

fun configValueAllowEmpty(propertyName: String, envName: String): String? =
    when {
        localProperties.containsKey(propertyName) -> localProperties.getProperty(propertyName)
        System.getenv().containsKey(envName) -> System.getenv(envName)
        releaseEnvProperties.containsKey(envName) ->
            releaseEnvProperties.getProperty(envName).trim().removeSurrounding("\"")
        else -> null
    }

fun configIntValue(propertyName: String, envName: String): Int? =
    configValue(propertyName, envName)?.toIntOrNull()

fun stringLiteral(value: String): String =
    "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

fun gitValue(vararg args: String): String? =
    runCatching {
        providers.exec {
            commandLine("git", "-C", rootProject.rootDir.absolutePath, *args)
        }.standardOutput.asText.get().trim()
    }.getOrNull()?.takeIf { it.isNotBlank() }

val appVersionCode = configIntValue("app.versionCode", "IRIS_APP_VERSION_CODE") ?: 1
val appVersionName = configValue("app.versionName", "IRIS_APP_VERSION_NAME") ?: "0.1.0"
val debugApplicationIdSuffix =
    configValueAllowEmpty("debug.applicationIdSuffix", "IRIS_DEBUG_APPLICATION_ID_SUFFIX") ?: ".debug"
val buildGitSha = configValue("build.gitSha", "IRIS_BUILD_GIT_SHA") ?: gitValue("rev-parse", "--short=12", "HEAD") ?: "unknown"
val buildTimestampUtc =
    configValue("build.timestampUtc", "IRIS_BUILD_TIMESTAMP_UTC")
        ?: System.getenv("SOURCE_DATE_EPOCH")?.toLongOrNull()?.let { Instant.ofEpochSecond(it).toString() }
        ?: gitValue("log", "-1", "--format=%ct", "HEAD")?.toLongOrNull()?.let { Instant.ofEpochSecond(it).toString() }
        ?: Instant.now().toString()
val mobilePushServerUrl = configValue("mobilePush.serverUrl", "IRIS_MOBILE_PUSH_SERVER_URL") ?: ""
val updateManifestUrl = configValue("update.manifestUrl", "IRIS_UPDATE_MANIFEST_URL") ?: ""
val updatePollSeconds = configValue("update.pollSeconds", "IRIS_UPDATE_POLL_SECONDS")?.toLongOrNull() ?: 0L

data class BuildRelayConfig(
    val relaySetId: String,
    val relaysCsv: String,
    val trustedTestBuild: Boolean,
)

val debugRelayConfig =
    BuildRelayConfig(
        relaySetId = configValue("debug.relaySetId", "IRIS_DEBUG_RELAY_SET_ID") ?: "public-dev",
        relaysCsv = configValue("debug.relays", "IRIS_DEBUG_RELAYS") ?: publicRelayFallbackCsv,
        trustedTestBuild = false,
    )
val betaRelayConfig =
    BuildRelayConfig(
        relaySetId = configValue("beta.relaySetId", "IRIS_BETA_RELAY_SET_ID") ?: "beta-fallback",
        relaysCsv = configValue("beta.relays", "IRIS_BETA_RELAYS") ?: publicRelayFallbackCsv,
        trustedTestBuild = true,
    )
val releaseRelayConfig =
    BuildRelayConfig(
        relaySetId = configValue("release.relaySetId", "IRIS_RELEASE_RELAY_SET_ID") ?: "public-release",
        relaysCsv = configValue("release.relays", "IRIS_RELEASE_RELAYS") ?: publicRelayFallbackCsv,
        trustedTestBuild = false,
    )
val betaSigningStoreFile = configValue("beta.storeFile", "IRIS_BETA_KEYSTORE_PATH")
val betaSigningStorePassword = configValue("beta.storePassword", "IRIS_BETA_KEYSTORE_PASSWORD")
val betaSigningKeyAlias = configValue("beta.keyAlias", "IRIS_BETA_KEY_ALIAS")
val betaSigningKeyPassword = configValue("beta.keyPassword", "IRIS_BETA_KEY_PASSWORD")
val releaseSigningStoreFile = configValue("release.storeFile", "IRIS_RELEASE_KEYSTORE_PATH")
val releaseSigningStorePassword = configValue("release.storePassword", "IRIS_RELEASE_KEYSTORE_PASSWORD")
val releaseSigningKeyAlias = configValue("release.keyAlias", "IRIS_RELEASE_KEY_ALIAS")
val releaseSigningKeyPassword = configValue("release.keyPassword", "IRIS_RELEASE_KEY_PASSWORD")
val hasDedicatedBetaSigning =
    !betaSigningStoreFile.isNullOrBlank() &&
        !betaSigningStorePassword.isNullOrBlank() &&
        !betaSigningKeyAlias.isNullOrBlank() &&
        !betaSigningKeyPassword.isNullOrBlank()
val hasReleaseSigning =
    !releaseSigningStoreFile.isNullOrBlank() &&
        !releaseSigningStorePassword.isNullOrBlank() &&
        !releaseSigningKeyAlias.isNullOrBlank() &&
        !releaseSigningKeyPassword.isNullOrBlank()
val hostLibraryFile =
    cargoTargetDir.resolve(
        when {
            System.getProperty("os.name").startsWith("Mac", ignoreCase = true) -> "debug/libiris_chat_core.dylib"
            System.getProperty("os.name").startsWith("Windows", ignoreCase = true) -> "debug/iris_chat_core.dll"
            else -> "debug/libiris_chat_core.so"
        },
    )

android {
    namespace = "to.iris.chat"
    compileSdk = 36
    ndkVersion = ndkVersionValue

    defaultConfig {
        applicationId = androidAppId
        minSdk = 26
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersionName
        testApplicationId = "$androidAppId.test"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(releaseSigningStoreFile!!)
                storePassword = releaseSigningStorePassword
                keyAlias = releaseSigningKeyAlias
                keyPassword = releaseSigningKeyPassword
            }
        }
        if (hasDedicatedBetaSigning) {
            create("beta") {
                storeFile = file(betaSigningStoreFile!!)
                storePassword = betaSigningStorePassword
                keyAlias = betaSigningKeyAlias
                keyPassword = betaSigningKeyPassword
            }
        }
    }

    buildTypes {
        debug {
            if (debugApplicationIdSuffix.isNotEmpty()) {
                applicationIdSuffix = debugApplicationIdSuffix
            }
            versionNameSuffix = "-debug"
            buildConfigField("String", "BUILD_CHANNEL", stringLiteral("debug"))
            buildConfigField("String", "BUILD_GIT_SHA", stringLiteral(buildGitSha))
            buildConfigField("String", "BUILD_TIMESTAMP_UTC", stringLiteral(buildTimestampUtc))
            buildConfigField("String", "MOBILE_PUSH_SERVER_URL", stringLiteral(mobilePushServerUrl))
            buildConfigField("String", "UPDATE_MANIFEST_URL", stringLiteral(updateManifestUrl))
            buildConfigField("long", "UPDATE_POLL_SECONDS", "${updatePollSeconds}L")
            buildConfigField("String", "RELAY_SET_ID", stringLiteral(debugRelayConfig.relaySetId))
            buildConfigField("String", "DEFAULT_RELAYS_CSV", stringLiteral(debugRelayConfig.relaysCsv))
            buildConfigField("boolean", "TRUSTED_TEST_BUILD", debugRelayConfig.trustedTestBuild.toString())
            buildConfigField("boolean", "SELF_UPDATE_ENABLED", "false")
        }

        create("beta") {
            initWith(getByName("release"))
            applicationIdSuffix = ".beta"
            versionNameSuffix = "-beta"
            isDebuggable = false
            matchingFallbacks += listOf("release")
            signingConfig =
                if (hasDedicatedBetaSigning) {
                    signingConfigs.getByName("beta")
                } else if (hasReleaseSigning) {
                    signingConfigs.getByName("release")
                } else {
                    signingConfigs.getByName("debug")
                }
            buildConfigField("String", "BUILD_CHANNEL", stringLiteral("beta"))
            buildConfigField("String", "BUILD_GIT_SHA", stringLiteral(buildGitSha))
            buildConfigField("String", "BUILD_TIMESTAMP_UTC", stringLiteral(buildTimestampUtc))
            buildConfigField("String", "MOBILE_PUSH_SERVER_URL", stringLiteral(mobilePushServerUrl))
            buildConfigField("String", "UPDATE_MANIFEST_URL", stringLiteral(updateManifestUrl))
            buildConfigField("long", "UPDATE_POLL_SECONDS", "${updatePollSeconds}L")
            buildConfigField("String", "RELAY_SET_ID", stringLiteral(betaRelayConfig.relaySetId))
            buildConfigField("String", "DEFAULT_RELAYS_CSV", stringLiteral(betaRelayConfig.relaysCsv))
            buildConfigField("boolean", "TRUSTED_TEST_BUILD", betaRelayConfig.trustedTestBuild.toString())
            buildConfigField("boolean", "SELF_UPDATE_ENABLED", "false")
        }

        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            buildConfigField("String", "BUILD_CHANNEL", stringLiteral("release"))
            buildConfigField("String", "BUILD_GIT_SHA", stringLiteral(buildGitSha))
            buildConfigField("String", "BUILD_TIMESTAMP_UTC", stringLiteral(buildTimestampUtc))
            buildConfigField("String", "MOBILE_PUSH_SERVER_URL", stringLiteral(mobilePushServerUrl))
            buildConfigField("String", "UPDATE_MANIFEST_URL", stringLiteral(updateManifestUrl))
            buildConfigField("long", "UPDATE_POLL_SECONDS", "${updatePollSeconds}L")
            buildConfigField("String", "RELAY_SET_ID", stringLiteral(releaseRelayConfig.relaySetId))
            buildConfigField("String", "DEFAULT_RELAYS_CSV", stringLiteral(releaseRelayConfig.relaysCsv))
            buildConfigField("boolean", "TRUSTED_TEST_BUILD", releaseRelayConfig.trustedTestBuild.toString())
            buildConfigField("boolean", "SELF_UPDATE_ENABLED", "false")
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }

        create("selfHosted") {
            initWith(getByName("release"))
            matchingFallbacks += listOf("release")
            buildConfigField("String", "BUILD_CHANNEL", stringLiteral("release"))
            buildConfigField("boolean", "SELF_UPDATE_ENABLED", "true")
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }

    sourceSets["main"].jniLibs.directories.add(generatedJniDir.get().asFile.absolutePath)
}

val buildRustHostDebug by tasks.registering(Exec::class) {
    group = "rust"
    description = "Build the host Rust library for UniFFI binding generation."
    workingDir = rustAppDir
    environment("CARGO_TARGET_DIR", cargoTargetDir.absolutePath)
    environment("IRIS_APP_VERSION", appVersionName)
    environment("IRIS_BUILD_CHANNEL", "debug")
    environment("IRIS_BUILD_GIT_SHA", buildGitSha)
    environment("IRIS_BUILD_TIMESTAMP_UTC", buildTimestampUtc)
    environment("IRIS_DEFAULT_RELAYS", debugRelayConfig.relaysCsv)
    environment("IRIS_RELAY_SET_ID", debugRelayConfig.relaySetId)
    environment("IRIS_TRUSTED_TEST_BUILD", debugRelayConfig.trustedTestBuild.toString())
    commandLine(
        cargoBinary.absolutePath,
        "build",
        "--manifest-path",
        rustManifestPath.absolutePath,
    )
    inputs.file(rustManifestPath)
    inputs.file(rustAppDir.resolve("uniffi.toml"))
    inputs.dir(rustSourceDir)
    rustPathDependencyDirs.forEach { inputs.dir(it) }
    inputs.property("ndrAppVersion", appVersionName)
    inputs.property("ndrBuildChannel", "debug")
    inputs.property("ndrBuildGitSha", buildGitSha)
    inputs.property("ndrBuildTimestampUtc", buildTimestampUtc)
    inputs.property("ndrDefaultRelays", debugRelayConfig.relaysCsv)
    inputs.property("ndrRelaySetId", debugRelayConfig.relaySetId)
    inputs.property("ndrTrustedTestBuild", debugRelayConfig.trustedTestBuild)
    inputs.property("cargoTargetDir", cargoTargetDir.absolutePath)
    outputs.file(hostLibraryFile)
}

val generateRustBindings by tasks.registering(Exec::class) {
    group = "rust"
    description = "Generate Kotlin bindings from the shared Rust UniFFI crate."
    dependsOn(buildRustHostDebug)
    workingDir = rustAppDir
    environment("CARGO_TARGET_DIR", cargoTargetDir.absolutePath)
    doFirst {
        generatedUniffiDir.get().asFile.deleteRecursively()
        generatedUniffiDir.get().asFile.mkdirs()
    }
    commandLine(
        cargoBinary.absolutePath,
        "run",
        "-q",
        "--manifest-path",
        rustAppDir.resolve("uniffi-bindgen/Cargo.toml").absolutePath,
        "--",
        "generate",
        "--library",
        hostLibraryFile.absolutePath,
        "--language",
        "kotlin",
        "--no-format",
        "--out-dir",
        generatedUniffiDir.get().asFile.absolutePath,
        "--config",
        rustAppDir.resolve("uniffi.toml").absolutePath,
    )
    inputs.file(rustAppDir.resolve("uniffi.toml"))
    inputs.file(hostLibraryFile)
    inputs.property("cargoTargetDir", cargoTargetDir.absolutePath)
    outputs.dir(generatedUniffiDir)
}

fun registerRustAndroidTask(
    taskName: String,
    descriptionText: String,
    buildChannel: String,
    relayConfig: BuildRelayConfig,
    releaseMode: Boolean,
) =
    tasks.register(taskName, Exec::class) {
        group = "rust"
        description = descriptionText
        workingDir = rustAppDir
        doFirst {
            generatedJniDir.get().asFile.deleteRecursively()
            generatedJniDir.get().asFile.mkdirs()
        }
        environment("CARGO_TARGET_DIR", cargoTargetDir.absolutePath)
        environment("ANDROID_HOME", androidSdkDir)
        environment("ANDROID_SDK_ROOT", androidSdkDir)
        environment("ANDROID_NDK_HOME", androidNdkDir.absolutePath)
        environment("NDK_HOME", androidNdkDir.absolutePath)
        environment("IRIS_APP_VERSION", appVersionName)
        environment("IRIS_BUILD_CHANNEL", buildChannel)
        environment("IRIS_BUILD_GIT_SHA", buildGitSha)
        environment("IRIS_BUILD_TIMESTAMP_UTC", buildTimestampUtc)
        environment("IRIS_DEFAULT_RELAYS", relayConfig.relaysCsv)
        environment("IRIS_RELAY_SET_ID", relayConfig.relaySetId)
        environment("IRIS_TRUSTED_TEST_BUILD", relayConfig.trustedTestBuild.toString())
        val command =
            mutableListOf(
                cargoBinary.absolutePath,
                "ndk",
                "-t",
                "arm64-v8a",
                "-P",
                "26",
                "-o",
                generatedJniDir.get().asFile.absolutePath,
                "--manifest-path",
                rustManifestPath.absolutePath,
                "build",
            )
        if (releaseMode) {
            command += "--release"
        }
        commandLine(command)
        inputs.file(rustManifestPath)
        inputs.file(rustAppDir.resolve("uniffi.toml"))
        inputs.dir(rustSourceDir)
        rustPathDependencyDirs.forEach { inputs.dir(it) }
        inputs.property("ndrAppVersion", appVersionName)
        inputs.property("ndrBuildChannel", buildChannel)
        inputs.property("ndrBuildGitSha", buildGitSha)
        inputs.property("ndrBuildTimestampUtc", buildTimestampUtc)
        inputs.property("ndrDefaultRelays", relayConfig.relaysCsv)
        inputs.property("ndrRelaySetId", relayConfig.relaySetId)
        inputs.property("ndrTrustedTestBuild", relayConfig.trustedTestBuild)
        inputs.property("cargoTargetDir", cargoTargetDir.absolutePath)
        outputs.dir(generatedJniDir)
    }

val buildRustAndroidDebug =
    registerRustAndroidTask(
        "buildRustAndroidDebug",
        "Build the Android Rust app core library for debug devices.",
        "debug",
        debugRelayConfig,
        releaseMode = false,
    )
val buildRustAndroidBeta =
    registerRustAndroidTask(
        "buildRustAndroidBeta",
        "Build the Android Rust app core library for beta devices.",
        "beta",
        betaRelayConfig,
        releaseMode = true,
    )
val buildRustAndroidRelease =
    registerRustAndroidTask(
        "buildRustAndroidRelease",
        "Build the Android Rust app core library for release devices.",
        "release",
        releaseRelayConfig,
        releaseMode = true,
    )

listOf(buildRustAndroidDebug, buildRustAndroidBeta, buildRustAndroidRelease).forEach { taskProvider ->
    taskProvider.configure {
        mustRunAfter(generateRustBindings)
    }
}

tasks.withType<KotlinCompile>().configureEach {
    dependsOn(generateRustBindings)
    source(generatedUniffiDir.get().asFile)
}

tasks.withType<Test>().configureEach {
    failOnNoDiscoveredTests = false
}

tasks.named("preBuild").configure {
    dependsOn(generateRustBindings)
}

tasks.configureEach {
    when (name) {
        "mergeDebugJniLibFolders" -> dependsOn(buildRustAndroidDebug)
        "mergeBetaJniLibFolders" -> dependsOn(buildRustAndroidBeta)
        "mergeReleaseJniLibFolders" -> dependsOn(buildRustAndroidRelease)
        "mergeSelfHostedJniLibFolders" -> dependsOn(buildRustAndroidRelease)
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(platform(libs.androidx.compose.bom))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.appcompat)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.material3)
    implementation("androidx.compose.material:material-icons-extended")
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.google.mlkit.barcode.scanning)
    implementation(libs.okhttp)
    implementation(libs.zxing.core)
    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)
    implementation("net.java.dev.jna:jna:5.12.0@aar")

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)

    androidTestImplementation(libs.androidx.junit)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)

    debugImplementation(libs.androidx.compose.ui.tooling)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
}
