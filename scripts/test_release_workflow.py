#!/usr/bin/env python3

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_release_builds_manifest_and_attests_every_file(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("scripts/release-manifest.py create", workflow)
        self.assertIn("subject-path: artifacts/*", workflow)
        self.assertIn("git cat-file -t", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn("gh release verify-asset", workflow)
        self.assertIn("--json isImmutable", workflow)
        self.assertIn("if: steps.existing.outputs.exists != 'true'", workflow)
        self.assertIn("--channel github", workflow)
        self.assertIn("--notes RELEASE_NOTES.md", workflow)
        self.assertNotIn("CHANGELOG.md", workflow)
        self.assertNotIn("ZAPSTORE_RELEASE_NOTES.md", workflow)

    def test_build_workflow_uploads_only_canonical_names(self) -> None:
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertIn("iris-chat-v${IRIS_APP_VERSION_NAME}-android-arm64.apk", workflow)
        self.assertIn("iris-v${IRIS_APP_VERSION_NAME}-${target}.tar.gz", workflow)
        upload_paths = "\n".join(
            line for line in workflow.splitlines() if "dist/android/" in line
        )
        self.assertNotIn("dist/android/*.apk", upload_paths)
        self.assertNotIn("latest", upload_paths)
        self.assertIn("runs-on: macos-26", workflow)

    def test_one_apple_workflow_reuses_exact_tagged_ipa(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        self.assertIn("- testflight", workflow)
        self.assertIn("- app-store", workflow)
        self.assertIn('ipa_name="iris-chat-${RELEASE_TAG}-ios.ipa"', workflow)
        self.assertIn("gh attestation verify", workflow)
        self.assertIn("gh release verify-asset", workflow)
        self.assertIn("--json isDraft,isImmutable,isPrerelease,url", workflow)
        self.assertIn("--commit \"$(git -C source rev-parse HEAD)\"", workflow)
        self.assertIn('expected_version="$(apple_marketing_version', workflow)
        self.assertIn('expected_build="$(semantic_version_code', workflow)
        self.assertIn("existing_build", workflow)
        self.assertIn("distribute_only", workflow)
        self.assertIn(
            "IRIS_TESTFLIGHT_GROUPS must name at least one internal group",
            workflow,
        )
        self.assertIn("get_edit_app_store_version", workflow)
        self.assertIn("completed_states", workflow)
        self.assertIn("attached_build = exact_version.build&.version", workflow)
        self.assertIn("submit_for_review: true", workflow)
        self.assertNotIn("./scripts/ios-release archive", workflow)
        self.assertFalse((ROOT / ".github/workflows/ios-testflight-upload.yml").exists())
        self.assertFalse((ROOT / ".github/workflows/android-release-apk.yml").exists())

    def test_app_store_retry_resumes_ready_review_submission(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        completed_states = workflow.split("completed_states = [", 1)[1].split("]", 1)[0]
        self.assertNotIn("READY_FOR_REVIEW", completed_states)

        resume_start = workflow.index(
            "if exact_version&.app_version_state == ready_state"
        )
        normal_start = workflow.index("edit_version = app.get_edit_app_store_version")
        build_check_start = workflow.index(
            "if exact_version &&", workflow.index("completed_states = [")
        )
        build_check = workflow[build_check_start:resume_start]
        resume_path = workflow[resume_start:normal_start]
        self.assertIn("exact_version.app_version_state == ready_state", build_check)
        self.assertIn("attached_build = exact_version.build&.version", build_check)
        self.assertIn("if attached_build != build_number", build_check)
        self.assertIn('includes: "appStoreVersionForReview"', resume_path)
        self.assertIn(
            "submission&.app_store_version_for_review&.id == exact_version.id",
            resume_path,
        )
        self.assertIn(
            "Spaceship::ConnectAPI::ReviewSubmissionItem.all(", resume_path
        )
        self.assertIn("review_submission_id: submission.id", resume_path)
        self.assertIn('includes: "appStoreVersion"', resume_path)
        self.assertIn("items.one?", resume_path)
        self.assertIn(
            "items.first.app_store_version&.id == exact_version.id", resume_path
        )
        self.assertNotIn("submission.items", resume_path)
        self.assertIn(
            "submitted = submission.submit_for_review(client: client)", resume_path
        )
        self.assertIn(
            "ReviewSubmissionState::WAITING_FOR_REVIEW", resume_path
        )
        self.assertIn("ReviewSubmissionState::IN_REVIEW", resume_path)
        self.assertIn(
            "submitted_states.include?(submitted&.state)", resume_path
        )
        self.assertEqual(resume_path.count("submit_for_review"), 1)
        self.assertNotIn("upload_to_app_store", resume_path)
        self.assertLess(
            resume_path.index("UI.user_error!"),
            resume_path.index("submitted = submission.submit_for_review"),
        )
        self.assertLess(
            resume_path.index("submitted_states.include?(submitted&.state)"),
            resume_path.index("UI.success"),
        )
        self.assertIn("next", resume_path[resume_path.index("UI.success") :])

    def test_app_store_retry_never_recreates_an_unhandled_exact_version(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        self.assertIn("if exact_version && !edit_version", workflow)
        self.assertIn("existing_version = !exact_version.nil?", workflow)
        self.assertNotIn("existing_version = !edit_version.nil?", workflow)

    def test_only_supported_operator_entrypoint_is_active(self) -> None:
        self.assertTrue((ROOT / "scripts/distribute").exists())
        self.assertFalse((ROOT / "scripts/release").exists())
        self.assertTrue((ROOT / "scripts/legacy/release/local-build-and-publish").exists())


if __name__ == "__main__":
    unittest.main()
