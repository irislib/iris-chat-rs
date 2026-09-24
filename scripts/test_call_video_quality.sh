#!/usr/bin/env bash
# No app launch, device permissions, peer accounts, or public message servers.
# Native Apple codec timings plus bidirectional production FIPS/UDP transport.
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-all}"
if [[ "$MODE" != all && "$MODE" != codec && "$MODE" != transport ]]; then
    echo "usage: $0 [all|codec|transport]" >&2
    exit 2
fi
if [[ "$MODE" != transport ]]; then
    if [[ "$(uname -s)" != Darwin ]]; then
        echo 'The native codec benchmark requires macOS; use transport on other hosts.' >&2
        exit 2
    fi
    BUILD_DIR="$(mktemp -d)"
    trap 'rm -rf "$BUILD_DIR"' EXIT
    cat > "$BUILD_DIR/main.swift" <<'SWIFT'
import XCTest
let suite = CallVideoQualityTests.defaultTestSuite
suite.run()
exit(suite.testRun?.hasSucceeded == true && suite.testCaseCount > 0 ? 0 : 1)
SWIFT
    XCODE_DEVELOPER="${DEVELOPER_DIR:-$(xcode-select -p)}"
    TEST_FRAMEWORKS="$XCODE_DEVELOPER/Platforms/MacOSX.platform/Developer/Library/Frameworks"
    TEST_LIBRARIES="$XCODE_DEVELOPER/Platforms/MacOSX.platform/Developer/usr/lib"
    xcrun swiftc -O -D CALL_VIDEO_STANDALONE \
        -I "$TEST_LIBRARIES" -L "$TEST_LIBRARIES" \
        -Xlinker -rpath -Xlinker "$TEST_LIBRARIES" \
        -F "$TEST_FRAMEWORKS" -Xlinker -rpath -Xlinker "$TEST_FRAMEWORKS" \
        "$ROOT/ios/Sources/IrisH264Wire.swift" \
        "$ROOT/ios/Sources/IrisH264Encoder.swift" \
        "$ROOT/ios/Sources/IrisH264Decoder.swift" \
        "$ROOT/ios/Sources/IrisCallVideoReceiver.swift" \
        "$ROOT/ios/Tests/CallVideoQualityTests.swift" \
        "$BUILD_DIR/main.swift" -o "$BUILD_DIR/call-video-quality"
    "$BUILD_DIR/call-video-quality"
fi
if [[ "$MODE" != codec ]]; then
    cargo test --manifest-path "$ROOT/core/Cargo.toml" --locked --lib \
        calls_sustained_video_quality_over_local_fips_udp -- --nocapture
fi
