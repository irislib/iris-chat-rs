#!/usr/bin/env python3
"""Require executed Android contact-editor regressions in fresh Gradle results."""

import argparse
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET


CONTACT_CLASS = "to.iris.chat.ui.screens.ContactNicknameCardTest"
CONTACT_TESTS = frozenset({
    "opensEmptyContactAndSavesUnicodeDraftWithoutCrashing",
    "unicodeWhitespaceFormattingAloneDoesNotEnableSave",
    "codepointLimitsAllowSupplementaryCharactersAtBothBoundaries",
    "unchangedStoredNonWhitespaceControlsDoNotEnableSave",
    "clearingBothFieldsCanBeSaved",
    "cancelDoesNotSaveAndRemovalKeepsItsSeparateAction",
})


def verify(results: Path, since: float) -> dict:
    if not math.isfinite(since) or since <= 0:
        raise ValueError("A positive run start time is required")
    reports = sorted(path for path in results.rglob("TEST-*.xml")
                     if path.stat().st_mtime >= since)
    if not reports:
        raise ValueError("No fresh Android instrumentation results")
    completed = set()
    executed = 0
    for path in reports:
        root = ET.parse(path).getroot()
        if root.tag not in ("testsuite", "testsuites"):
            raise ValueError("Unexpected instrumentation report format")
        for suite in root.iter("testsuite"):
            if int(suite.get("tests", "-1")) != len(suite.findall("testcase")):
                raise ValueError("Incomplete Android instrumentation report")
            for field in ("failures", "errors", "skipped", "disabled"):
                if int(suite.get(field, "0")) != 0:
                    raise ValueError("Android instrumentation reported " + field)
        for case in root.iter("testcase"):
            if any(case.find(tag) is not None for tag in ("failure", "error", "skipped")):
                raise ValueError("Android test failed or did not execute")
            if case.get("status", "run") not in ("run", "passed"):
                raise ValueError("Android test did not execute")
            executed += 1
            if case.get("classname") == CONTACT_CLASS:
                name = case.get("name")
                if name in completed:
                    raise ValueError("Duplicate contact-editor result")
                completed.add(name)
    missing = CONTACT_TESTS - completed
    if missing:
        raise ValueError("Contact-editor tests missing: " + ", ".join(sorted(missing)))
    return {"passed": True, "executed": executed, "contact_tests": sorted(completed),
            "reports": [str(path) for path in reports]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--since", type=float, required=True)
    args = parser.parse_args()
    try:
        result = verify(args.results, args.since)
    except (ValueError, OSError, ET.ParseError) as error:
        parser.exit(1, str(error) + "\n")
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
