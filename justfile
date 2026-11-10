set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

info:
    @just --list --unsorted

run:
    @case "$(uname -s)" in \
        Darwin) just run-macos ;; \
        Linux) just run-linux ;; \
        MINGW*|MSYS*|CYGWIN*) just run-windows ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

run-ios:
    ./tools/run-ios

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

verify-fast:
    ./scripts/verify.sh fast

verify-full:
    ./scripts/verify.sh full

verify-health:
    ./scripts/verify.sh health

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
