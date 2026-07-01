set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

export CARGO_TARGET_DIR := env_var_or_default("CARGO_TARGET_DIR", env_var("HOME") + "/.cache/cargo-target")

default:
    @just --list

info:
    @echo "Iris Chat commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo "  just run-mobile"
    @echo "  just run-android"
    @echo "  just run-ios"
    @echo "  just run-ios-sims 4"
    @echo "  just run-linux"
    @echo "  just run-macos"
    @echo "  just run-windows"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just release"
    @echo "  just release-publish"
    @echo
    @echo "Bindings and native builds"
    @echo "  just gen-kotlin"
    @echo "  just android-rust"
    @echo "  just android-assemble"
    @echo "  just android-beta-apk"
    @echo "  just android-release-bundle"
    @echo "  just ios-gen-swift"
    @echo "  just ios-rust"
    @echo "  just ios-xcframework"
    @echo "  just ios-xcodeproj"
    @echo "  just macos-gen-swift"
    @echo "  just macos-rust"
    @echo "  just macos-xcframework"
    @echo "  just macos-xcodeproj"
    @echo "  just macos-build"
    @echo "  just windows-doctor"
    @echo "  just windows-rust"
    @echo "  just windows-gen-cs"
    @echo "  just windows-dotnet"
    @echo "  just windows-build"
    @echo "  just ios-release-prepare"
    @echo "  just ios-release-archive"
    @echo "  just macos-app"
    @echo "  just macos-dmg"
    @echo "  just windows-installer"
    @echo "  just windows-zip"
    @echo "  just linux-release"
    @echo "  just release"
    @echo "  just release-publish"
    @echo
    @echo "Checks"
    @echo "  just doctor-ios"
    @echo "  just qa"
    @echo "  just test"
    @echo "  just release-gate"
    @echo "  just test-homebrew-tap"
    @echo "  just test-android"
    @echo "  just test-ios"
    @echo "  just test-ios-app-store-review"
    @echo "  just test-macos"
    @echo "  just test-linux"
    @echo "  just test-windows"
    @echo "  just test-all-platforms"
    @echo "  just qa-native-contract"
    @echo "  just qa-interop"
    @echo "  just qa-lan"

run:
    @case "$(uname -s)" in \
        Darwin) just run-macos ;; \
        Linux) just run-linux ;; \
        MINGW*|MSYS*|CYGWIN*) just run-windows ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

run-ios:
    ./tools/run-ios

run-ios-sims count="4":
    ./tools/run-ios-sims "{{count}}"

run-mobile:
    ./tools/run-mobile

run-macos:
    ./tools/run-macos

run-linux:
    ./tools/run-linux

run-android:
    ./tools/run-android

run-windows:
    ./tools/run-windows

build:
    @case "$(uname -s)" in \
        Darwin) just macos-build ;; \
        Linux) ./tools/run-linux cargo build ;; \
        MINGW*|MSYS*|CYGWIN*) just windows-build ;; \
        *) echo "No local build target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac
    @./scripts/build-output-path

ios-gen-swift:
    ./scripts/ios-build ios-gen-swift

ios-rust:
    ./scripts/ios-build ios-rust

ios-xcframework:
    ./scripts/ios-build ios-xcframework

ios-xcodeproj:
    ./scripts/ios-build ios-xcodeproj

macos-gen-swift:
    ./scripts/macos-build macos-gen-swift

macos-rust:
    ./scripts/macos-build macos-rust

macos-xcframework:
    ./scripts/macos-build macos-xcframework

macos-xcodeproj:
    ./scripts/macos-build macos-xcodeproj

macos-build:
    ./scripts/macos-build macos-build

macos-app:
    ./scripts/macos-build macos-app

macos-dmg:
    ./scripts/macos-build macos-dmg

windows-doctor:
    ./scripts/windows-build windows-doctor

windows-sync:
    ./scripts/windows-build windows-sync

windows-rust:
    ./scripts/windows-build windows-rust

windows-gen-cs:
    ./scripts/windows-build windows-gen-cs

windows-dotnet:
    ./scripts/windows-build windows-dotnet

windows-build:
    ./scripts/windows-build windows-build

windows-installer:
    ./scripts/windows-build windows-installer

windows-zip:
    ./scripts/windows-build windows-zip

linux-release:
    ./scripts/linux-release

release:
    ./scripts/release

release-publish:
    ./scripts/release --publish

android-rust:
    ./scripts/android-build android-rust

gen-kotlin:
    ./scripts/android-build gen-kotlin

android-assemble:
    ./scripts/android-build android-assemble

android-beta-apk:
    ./scripts/android-release beta-apk

android-release-bundle:
    ./scripts/android-release release-bundle

ios-release-prepare:
    ./scripts/ios-release prepare

ios-release-archive:
    ./scripts/ios-release archive

doctor-ios:
    ./tools/ios-runtime-doctor

qa:
    ./scripts/test_fast.sh

test:
    ./scripts/test

release-gate:
    ./scripts/test-release-gate

test-homebrew-tap:
    ./scripts/test_homebrew_tap

test-cli-install-docker:
    ./scripts/test_cli_install_docker

test-android:
    ./scripts/test-android

test-ios:
    ./scripts/test-ios

test-ios-app-store-review:
    ./scripts/e2e_ios_app_store_review_block_report.sh

test-macos:
    ./scripts/test-macos

test-linux:
    ./scripts/test-linux

test-windows:
    ./scripts/test-windows

test-all-platforms:
    ./scripts/test-all-platforms

qa-native-contract:
    ./scripts/test_native_contract.sh

qa-interop:
    ./scripts/test_interop_confidence.sh

qa-lan:
    ./scripts/nearby_lan_visibility_matrix.sh --allow-skip
