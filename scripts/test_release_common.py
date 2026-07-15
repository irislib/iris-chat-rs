import pathlib
import subprocess
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
COMMON = ROOT / "scripts/release_common.sh"


def version_code(version: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "bash",
            "-c",
            'source "$1"; semantic_version_code "$2"',
            "bash",
            str(COMMON),
            version,
        ],
        check=False,
        capture_output=True,
        text=True,
    )


class ReleaseVersionCodeTests(unittest.TestCase):
    def assert_code(self, version: str, expected: int) -> None:
        result = version_code(version)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(expected))

    def test_date_versions_pack_as_yyyymmdd_build(self) -> None:
        self.assert_code("2026.7.15", 2_026_071_500)
        self.assert_code("2026.7.15.1", 2_026_071_501)
        self.assert_code("2026.7.15.99", 2_026_071_599)

    def test_month_and_day_values_do_not_overlap(self) -> None:
        july = version_code("2026.7.14")
        august = version_code("2026.8.4")
        self.assertEqual(july.returncode, 0, july.stderr)
        self.assertEqual(august.returncode, 0, august.stderr)
        self.assertNotEqual(july.stdout, august.stdout)
        self.assertLess(int(july.stdout), int(august.stdout))

    def test_rejects_component_and_android_limit_overflow(self) -> None:
        for version in ("2026.7.15.100", "2026.100.1", "2100.0.1", "0.0.0"):
            with self.subTest(version=version):
                self.assertNotEqual(version_code(version).returncode, 0)


if __name__ == "__main__":
    unittest.main()
