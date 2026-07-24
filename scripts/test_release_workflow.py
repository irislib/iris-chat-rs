#!/usr/bin/env python3

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_app_store_release_uses_the_exact_tagged_ipa(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-app-store-release.yml").read_text()

        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn('ipa_name="iris-chat-${RELEASE_TAG}-ios.ipa"', workflow)
        self.assertIn('gh release download "$RELEASE_TAG"', workflow)
        self.assertIn("app_store_connect_api_key(", workflow)
        self.assertIn("submit_for_review:", workflow)
        self.assertIn('release_options[:ipa] = ENV.fetch("IPA_PATH")', workflow)
        self.assertIn(
            'release_options[:build_number] = ENV.fetch("BUILD_NUMBER")',
            workflow,
        )
        self.assertIn(
            'skip_app_version_update: ENV.fetch("REUSE_APP_STORE_VERSION") == "true"',
            workflow,
        )
        self.assertIn("environment: ios-app-store-release", workflow)
        self.assertNotIn("./scripts/ios-release archive", workflow)

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

    def test_github_and_self_publish_share_release_notes_renderer(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        local_release = (ROOT / "scripts/release").read_text()
        renderer = "scripts/render-release-notes.py"

        self.assertIn(renderer, workflow)
        self.assertIn(renderer, local_release)
        self.assertIn("--asset-base-url", workflow)
        self.assertIn("--verification-line", workflow)
        self.assertIn("--install-url", local_release)

    def test_local_release_handles_an_empty_skipped_platform_list(self) -> None:
        local_release = (ROOT / "scripts/release").read_text()

        self.assertIn(
            'for line in ${SKIPPED_LINES[@]+"${SKIPPED_LINES[@]}"}; do',
            local_release,
        )
        self.assertIn(
            'for built in ${BUILT_STEPS[@]+"${BUILT_STEPS[@]}"}; do',
            local_release,
        )

    def test_local_release_can_resume_an_exact_staged_build(self) -> None:
        local_release = (ROOT / "scripts/release").read_text()

        self.assertIn("--resume-staged", local_release)
        self.assertIn('load_staged_release "$EXPECTED_COMMIT"', local_release)
        self.assertIn('"$ROOT/scripts/validate-staged-release.py"', local_release)
        self.assertIn('COMMIT="$EXPECTED_COMMIT"', local_release)

    def test_only_cli_archives_are_marked_executable(self) -> None:
        local_release = (ROOT / "scripts/release").read_text()

        self.assertIn(
            "iris-aarch64-*.tar.gz|iris-x86_64-*.tar.gz)",
            local_release,
        )
        self.assertNotIn(
            "case \"$name\" in\n        iris-*.tar.gz)",
            local_release,
        )

    def test_zapstore_reuses_the_staged_signed_apk(self) -> None:
        local_release = (ROOT / "scripts/release").read_text()

        self.assertIn(
            'ZAPSTORE_APK_PATH="$STAGE_DIR/assets/iris-chat-${TAG}-android-arm64.apk"',
            local_release,
        )


if __name__ == "__main__":
    unittest.main()
