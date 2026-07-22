#!/usr/bin/env python3

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_ios_ipa_is_flattened_for_release_assembly(self) -> None:
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertIn('ipa_output="dist/ios/iris-chat-v${IRIS_APP_VERSION_NAME}-ios.ipa"', workflow)
        self.assertIn('cp "$ipa" "$ipa_output"', workflow)
        self.assertIn("${{ env.APP_DIR }}/dist/ios/*.ipa", workflow)
        self.assertNotIn("${{ env.APP_DIR }}/dist/ios/**/*.ipa", workflow)

    def test_release_verifier_requires_flat_assets(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('compgen -G "artifacts/$pattern"', workflow)
        self.assertIn("'*.ipa'", workflow)


if __name__ == "__main__":
    unittest.main()
