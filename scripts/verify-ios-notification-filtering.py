#!/usr/bin/env python3
"""Require notification filtering in a decoded profile or signed entitlements."""

import plistlib
import sys
from pathlib import Path


ENTITLEMENT = "com.apple.developer.usernotifications.filtering"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify-ios-notification-filtering.py <entitlements.plist>", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    try:
        document = plistlib.loads(path.read_bytes())
        entitlements = document.get("Entitlements", document)
        if entitlements.get(ENTITLEMENT) is not True:
            raise ValueError("notification-filtering entitlement must be true")
    except (OSError, ValueError, AttributeError, plistlib.InvalidFileException) as error:
        print(f"iOS notification filtering verification failed: {error}", file=sys.stderr)
        return 1
    print("iOS notification-filtering entitlement verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
