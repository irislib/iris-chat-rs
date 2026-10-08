#!/usr/bin/env python3

from pathlib import Path
import os
import plistlib
import sys
import tempfile
import re
import shutil
import subprocess
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_tagged_platform_policy_skips_only_ios_build_and_rejects_apple_distribution(self) -> None:
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        metadata = builds.split("\n  metadata:\n", 1)[1].split("\n  android:\n", 1)[0]
        self.assertIn('build_ios=true', metadata)
        self.assertIn('[[ -f release-platforms.json', metadata)
        self.assertIn('platform-enabled --tag "$requested_tag" --platform ios', metadata)
        self.assertIn('echo "build_ios=$build_ios"', metadata)
        ios = builds.split("\n  ios:\n", 1)[1].split("\n  macos:\n", 1)[0]
        self.assertIn("if: needs.metadata.outputs.build_ios == 'true'", ios)
        for platform in ("android", "macos", "windows", "linux", "cli"):
            job = re.split(r"\n  (?=\S)", builds.split(f"\n  {platform}:\n", 1)[1], maxsplit=1)[0]
            self.assertNotIn("build_ios", job)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('scripts/release-manifest.py list-assets --tag "$RELEASE_TAG"', release)
        apple = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        check = apple.index("- name: Check iOS release policy")
        self.assertLess(check, apple.index("- name: Download and verify exact release IPA"))
        self.assertLess(check, apple.index("- name: Configure App Store Connect authentication"))
        self.assertIn('source/scripts/release-manifest.py require-platform', apple)
        self.assertIn('--tag "$RELEASE_TAG" --platform ios', apple)
        self.assertEqual((ROOT / ".github/workflows/release-contract.yml").read_text().count(
            '      - "release-platforms.json"'), 2)
        script = textwrap.dedent(metadata.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            for index, (ref, expected) in enumerate([
                ("v2026.10.8.2", "false"), ("refs/tags/v2026.10.8.2", "false"),
                ("v2026.10.8.3", "false"), ("refs/tags/v2026.10.8.3", "false"),
                ("v2026.10.7", "true"), ("main", "true"),
            ]):
                with self.subTest(ref=ref):
                    output = Path(directory) / str(index)
                    env = dict(os.environ, REQUESTED_REF=ref, GITHUB_OUTPUT=str(output))
                    result = subprocess.run(["bash", "-c", script], cwd=ROOT, env=env,
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(f"build_ios={expected}\n", output.read_text())

    def test_ios_filtering_requires_entitlement_in_profile_and_signed_extension(self) -> None:
        verifier = ROOT / "scripts/verify-ios-notification-filtering.py"
        key = "com.apple.developer.usernotifications.filtering"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "entitlements.plist"
            for wrapped in (False, True):
                for value in (None, False, "true", 1, True):
                    with self.subTest(profile=wrapped, value=repr(value)):
                        entitlements = {} if value is None else {key: value}
                        document = {"Entitlements": entitlements} if wrapped else entitlements
                        path.write_bytes(plistlib.dumps(document))
                        result = subprocess.run(
                            [sys.executable, str(verifier), str(path)], capture_output=True, text=True
                        )
                        self.assertEqual(result.returncode, 0 if value is True else 1)
            path.write_bytes(b"not a plist")
            result = subprocess.run([sys.executable, str(verifier), str(path)], capture_output=True)
            self.assertEqual(result.returncode, 1)
        workflow = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertEqual(workflow.count("python3 scripts/verify-ios-notification-filtering.py"), 2)
        self.assertIn('"$app/PlugIns/NotificationService.appex"', workflow)
        source = plistlib.loads((ROOT / "ios/NotificationService/NotificationService.entitlements").read_bytes())
        self.assertIs(source.get(key), True)
        for gate in ("scripts/ios-build", ".github/workflows/ci.yml"):
            self.assertIn("-only-testing:NotificationServiceTests", (ROOT / gate).read_text())

    def test_selected_ios_ui_tests_name_existing_classes_and_methods(self) -> None:
        tests = {}
        for path in (ROOT / "ios/UITests").glob("*.swift"):
            current = None
            for line in path.read_text().splitlines():
                declaration = re.match(r"(?:final )?class (\w+)\s*:", line)
                if declaration:
                    current = declaration.group(1)
                    tests.setdefault(current, set())
                method = re.match(r"\s+func (test\w+)\s*\(", line)
                if current and method:
                    tests[current].add(method.group(1))
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        selectors = re.findall(r"-only-testing:IrisChatUITests/([\w/]+)", workflow)
        self.assertTrue(selectors)
        for selector in selectors:
            parts = selector.split("/")
            with self.subTest(selector=selector):
                self.assertIn(parts[0], tests)
                if len(parts) == 2:
                    self.assertIn(parts[1], tests[parts[0]])

    def test_tagged_gates_reuse_default_branch_cargo_caches(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        gate = workflow.split("\n  mesh-resource:\n", 1)[0]
        self.assertIn("CARGO_TARGET_DIR: ${{ github.workspace }}/cargo-target", gate)
        self.assertIn("working-directory: iris-chat-rs", gate)
        self.assertIn("path: iris-chat-rs", gate)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        for text in (ci, gate):
            cache = text.split("- name: Cache Cargo\n", 1)[1].split("- name:", 1)[0]
            self.assertIn("uses: actions/cache@v6", cache)
        for fragment in (
            "~/.cargo/registry",
            "~/.cargo/git",
            "${{ github.workspace }}/cargo-target",
            "key: rust-${{ runner.os }}-${{ hashFiles('iris-chat-rs/**/Cargo.lock') }}",
            "rust-${{ runner.os }}-",
        ):
            self.assertIn(fragment, ci)
            self.assertIn(fragment, gate)
        self.assertIn("scripts/verify.sh fast", gate)
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        recipes = {
            "android": (
                "android-${{ runner.os }}-${{ hashFiles('iris-chat-rs/**/Cargo.lock', 'iris-chat-rs/android/**/*.gradle*', 'iris-chat-rs/android/gradle/**/*.toml') }}",
                ("~/.gradle/caches", "~/.gradle/wrapper"),
            ),
            "macos": (
                "macos-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('iris-chat-rs/**/Cargo.lock', 'iris-chat-rs/macos/project.yml') }}",
                ("${{ github.workspace }}/iris-chat-rs/macos/.build/cargo-target",),
            ),
        }
        for job, (key, extra_paths) in recipes.items():
            with self.subTest(platform=job):
                caches = []
                for text in (ci, builds):
                    body = re.split(r"\n  (?=\S)", text.split(f"\n  {job}:\n", 1)[1], maxsplit=1)[0]
                    cache = body.split("      - name: Cache Cargo", 1)[1].split("\n      - ", 1)[0].strip()
                    caches.append(cache)
                    self.assertNotIn("cache-hit", body)
                self.assertEqual(caches[0], caches[1])
                self.assertIn("uses: actions/cache@v6", caches[0])
                self.assertIn(f"key: {key}", caches[0])
                self.assertIn(f"            {key.split('${{ hashFiles', 1)[0]}", caches[0])
                for path in ("~/.cargo/registry", "~/.cargo/git", "${{ github.workspace }}/cargo-target", *extra_paths):
                    self.assertIn(path, caches[0])

    def test_release_only_edits_keep_contract_checks_without_native_rebuilds(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        expected = [
            '      - "**/*.md"',
            '      - ".github/workflows/release.yml"',
            '      - "scripts/test_release_workflow.py"',
        ]
        # Keep this exclusion narrow: changing any app/build input must still
        # trigger the original full platform checks.
        for block in ci.split("    paths-ignore:\n")[1:]:
            lines = []
            for line in block.splitlines():
                if not line.startswith("      - "):
                    break
                lines.append(line)
            self.assertEqual(lines, expected)
        self.assertEqual(ci.count("    paths-ignore:\n"), 2)
        quick = (ROOT / ".github/workflows/release-checks.yml").read_text()
        for path in (".github/workflows/ci.yml", ".github/workflows/release.yml", "scripts/test_release_workflow.py"):
            self.assertEqual(quick.count(f'      - "{path}"'), 2)
        for script in ("test_release_workflow.py", "test_release_notes.py", "test_build_common.py"):
            self.assertIn(f"python3 scripts/{script}", quick)
        self.assertIn('"scripts/render-release-notes.py"', quick)
        self.assertIn('"--channel", "validate"', quick)

    def test_resource_gate_binds_tag_commit_and_blocks_publication(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        pin = "e34fa190b41a3ced5148e92b103418004bdd0b65"
        self.assertIn(f"irislib/iris-stack/.github/workflows/product-lab.yml@{pin}", workflow)
        self.assertIn(f"lab_rev: {pin}", workflow)
        self.assertIn("chat_rev: ${{ needs.verify.outputs.sha }}", workflow)
        self.assertIn("drive_rev: 7cb74966ddaecf90fb91b8f36a44ecc4bbda7b02", workflow)
        self.assertIn("htree_version: 0.2.148", workflow)
        release = workflow.split("\n  release:\n", 1)[1]
        self.assertIn("      - mesh-resource\n", release.split("    runs-on:", 1)[0])
        self.assertIn("pattern: iris-*-${{ needs.build.outputs.artifact_suffix }}", release)
        builds = (ROOT / ".github/workflows/build-artifacts.yml").read_text()
        self.assertIn("value: ${{ jobs.metadata.outputs.version_name }}-${{ jobs.metadata.outputs.short_sha }}", builds)

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
        self.assertIn("- testflight-public", workflow)
        self.assertIn("- app-store", workflow)
        self.assertIn("group: ios-distribution-${{ inputs.tag }}", workflow)
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
            "#{group_setting} must name at least one #{group_kind} group",
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

    @unittest.skipUnless(shutil.which("ruby"), "Ruby is needed to exercise the Fastfile")
    def test_testflight_routes_validate_audience_and_never_enter_app_store(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        fastfile = textwrap.dedent(
            workflow.split("<<'RUBY'\n", 1)[1].split("\n          RUBY", 1)[0]
        )
        # Run the production lane with read-only API stubs. Every publishing
        # action is recorded, including the pinned Pilot beta-review default.
        stub = r'''
          require "tempfile"
          module Deliver
            class UploadMetadata
            end
          end
          $LOADED_FEATURES << "deliver/upload_metadata.rb"
          class UserError < StandardError
          end
          module UI
            def self.user_error!(message)
              raise UserError, message
            end
            def self.success(message)
              $messages << message
            end
            def self.important(message)
              $messages << message
            end
          end
          Group = Struct.new(:name, :is_internal_group, :public_link_enabled)
          BetaDetail = Struct.new(:auto_notify_enabled, :external_build_state)
          FakeBuild = Struct.new(:id, :build_beta_detail)
          class FakeApp
            def id
              "test-app"
            end
            def get_beta_groups(client:)
              $events << :read_groups
              $groups
            end
            def get_app_store_versions(**)
              $events << :app_store
              raise "TestFlight entered the App Store route"
            end
          end
          module Spaceship
            module ConnectAPI
              module Token
                def self.create(**)
                  :test_token
                end
              end
              class Client
                def initialize(token:)
                end
                def test_flight_request_client
                  self
                end
                def post(path, body)
                  raise "Wrong notification endpoint" unless path == "v1/buildBetaNotifications"
                  expected = {data: {type: "buildBetaNotifications", relationships: {build: {data: {type: "builds", id: "exact-build"}}}}}
                  raise "Notifying wrong build" unless body == expected
                  $events << :activate_public_build
                end
              end
              module App
                def self.find(*, **)
                  FakeApp.new
                end
              end
              module Build
                def self.all(**options)
                  raise "Wrong exact version" unless options[:version] == "2026.10.100" && options[:build_number] == "1"
                  if $uploaded_options
                    $events << :read_public_status
                    return [] if $missing_build
                    return [FakeBuild.new("exact-build", $missing_detail ? nil : BetaDetail.new($auto_notify, $external_state))]
                  end
                  $existing_build ? [Object.new] : []
                end
                def self.get(client:, build_id:)
                  raise "Wrong build verification" unless build_id == "exact-build"
                  $events << :verify_activation
                  FakeBuild.new(build_id, BetaDetail.new(true, $activated_state))
                end
              end
              module Platform
                IOS = "IOS"
              end
            end
          end
          def default_platform(*)
          end
          def platform(*)
            yield
          end
          def lane(*, &block)
            $distribution_lane = block
          end
          def app_store_connect_api_key(**)
            :test_api_key
          end
          def upload_to_testflight(**options)
            $events << :testflight
            $uploaded_options = options
            # Fastlane 2.237.0 submits beta review for any supplied groups
            # unless this option explicitly overrides its true default.
            if options.fetch(:submit_beta_review, true) &&
               (options[:groups] || options[:distribute_external])
              $events << :beta_review
            end
            if options[:distribute_external] || options[:notify_external_testers] != false
              $events << :external_distribution_or_notification
            end
          end
          def upload_to_app_store(**)
            $events << :app_store
            raise "TestFlight called upload_to_app_store"
          end
          def sleep(*)
          end
          eval(STDIN.read, TOPLEVEL_BINDING, "Fastfile")
          ENV.update(
            "IRIS_ASC_AUTH_KEY_ID" => "test-key",
            "IRIS_ASC_AUTH_KEY_ISSUER_ID" => "test-issuer",
            "IRIS_ASC_AUTH_KEY_PATH" => "unused.p8",
            "IRIS_IOS_BUNDLE_ID" => "test.iris",
            "APP_VERSION" => "2026.10.100",
            "BUILD_NUMBER" => "1",
            "DISTRIBUTION_TARGET" => "testflight",
            "IPA_PATH" => "attested.ipa"
          )
          Tempfile.create("iris-testflight-notes") do |notes|
            notes.write("Tagged release notes\n")
            notes.flush
            ENV["RELEASE_NOTES_PATH"] = notes.path
            internal = [Group.new("Team", true), Group.new("QA", true)]
            public_groups = [Group.new("Public", false, true), Group.new("Preview", false, true)]
            audience_cases = {
              "testflight" => [
                [" Team, QA, Team ", internal, nil],
                ["Team, Missing", internal, "not found"],
                ["Team, Public", internal + public_groups, "internal"],
                ["Team", internal + [Group.new("Team", false, true)], "internal"],
                ["Unknown", [Group.new("Unknown", nil)], "internal"],
                [" , ", internal, "at least one internal group"]
              ],
              "testflight-public" => [
                [" Public, Preview, Public ", public_groups, nil],
                ["Public, Missing", public_groups, "not found"],
                ["Public, Team", public_groups + internal, "public external"],
                ["Public", public_groups + [Group.new("Public", true, true)], "public external"],
                ["Unknown", [Group.new("Unknown", nil, true)], "public external"],
                ["Closed", [Group.new("Closed", false, false)], "public link enabled"],
                ["Unknown", [Group.new("Unknown", false, nil)], "public link enabled"],
                [" , ", public_groups, "at least one public external group"]
              ]
            }
            [false, true].each do |existing|
              audience_cases.each do |target, cases|
                public_target = target == "testflight-public"
                ENV["DISTRIBUTION_TARGET"] = target
                cases.each do |configured, available, expected_error|
                  ENV["IRIS_TESTFLIGHT_GROUPS"] = public_target ? "Must not use internal groups" : configured
                  ENV["IRIS_TESTFLIGHT_PUBLIC_GROUPS"] = public_target ? configured : "Must not use public groups"
                  $groups = available
                  $existing_build = existing
                  $events = []
                  $messages = []
                  $auto_notify = true
                  $external_state = "IN_BETA_TESTING"
                  $uploaded_options = nil
                  error = nil
                  begin
                    $distribution_lane.call
                  rescue UserError => failure
                    error = failure.message
                  end
                  if expected_error
                    raise "Missing expected rejection: #{target}: #{configured}" unless error&.include?(expected_error)
                    raise "Mutation before rejection: #{$events}" unless ($events - [:read_groups]).empty?
                  else
                    raise error if error
                    expected_events = [:read_groups, :testflight]
                    expected_events += [:beta_review, :external_distribution_or_notification] if public_target
                    expected_events << :read_public_status if public_target
                    raise "Unexpected actions: #{$events}" unless $events == expected_events
                    options = $uploaded_options
                    wanted_groups = public_target ? ["Public", "Preview"] : ["Team", "QA"]
                    raise "Wrong groups" unless options[:groups] == wanted_groups
                    raise "Wrong release notes" unless options[:changelog] == "Tagged release notes"
                    raise "Public release must enable automatic distribution after approval" unless options[:notify_external_testers] == public_target
                    raise "Expired or rejected another build" if options[:expire_previous_builds] || options[:reject_build_waiting_for_review]
                    if existing
                      raise "Existing build was re-uploaded" unless options[:distribute_only] == true && !options.key?(:ipa)
                      raise "Existing build lacks explicit iOS platform" unless options[:app_platform] == "ios"
                    else
                      raise "New IPA was skipped" unless options[:ipa] == "attested.ipa" && !options.key?(:distribute_only)
                    end
                  end
                end
              end
            end
            ENV["DISTRIBUTION_TARGET"] = "testflight-public"
            ENV["IRIS_TESTFLIGHT_PUBLIC_GROUPS"] = "Public"
            $groups = public_groups
            [false, true].each do |existing|
              [
                ["IN_BETA_TESTING", true, "IN_BETA_TESTING", "available to testers", false],
                ["WAITING_FOR_BETA_REVIEW", true, nil, "awaiting Apple review", false],
                ["IN_BETA_REVIEW", true, nil, "awaiting Apple review", false],
                ["BETA_APPROVED", true, "IN_BETA_TESTING", "available to testers", false],
                ["READY_FOR_BETA_TESTING", true, "IN_BETA_TESTING", "available to testers", false],
                ["BETA_APPROVED", true, "BETA_APPROVED", "not available", true],
                ["BETA_REJECTED", true, nil, "not available", true],
                ["EXPIRED", true, nil, "not available", true],
                ["IN_BETA_TESTING", false, nil, "not enabled", true],
                ["missing-build", true, nil, "not found", true],
                ["missing-detail", true, nil, "not enabled", true]
              ].each do |state, auto_notify, activated, message, fails|
                $existing_build = existing
                $external_state = state
                $auto_notify = auto_notify
                $activated_state = activated
                $missing_build = state == "missing-build"
                $missing_detail = state == "missing-detail"
                $uploaded_options = nil
                $events = []
                $messages = []
                error = nil
                begin
                  $distribution_lane.call
                rescue UserError => failure
                  error = failure.message
                end
                raise "Wrong public result for #{state}: #{error || $messages}" unless fails ? error&.include?(message) : error.nil? && $messages.any? { |m| m.include?(message) }
                should_activate = ["BETA_APPROVED", "READY_FOR_BETA_TESTING"].include?(state)
                raise "Wrong activation count" unless $events.count(:activate_public_build) == (should_activate ? 1 : 0)
                raise "Repeated notification while waiting" if $events.count(:activate_public_build) > 1
                raise "Entered App Store" if $events.include?(:app_store)
              end
            end
            $uploaded_options = nil
            ENV["DISTRIBUTION_TARGET"] = "typo"
            $events = []
            begin
              $distribution_lane.call
              raise "Unknown target was accepted"
            rescue UserError => failure
              raise failure unless failure.message.include?("Unknown Apple distribution target")
              raise "Unknown target mutated Apple state" unless $events.empty?
            end
          end
        '''
        result = subprocess.run(
            [shutil.which("ruby"), "-e", textwrap.dedent(stub)],
            input=fastfile,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipUnless(shutil.which("ruby"), "Ruby is needed to exercise the Fastfile")
    def test_app_store_preserves_review_attachments_without_skipping_metadata(self) -> None:
        workflow = (ROOT / ".github/workflows/ios-distribution.yml").read_text()
        fastfile = textwrap.dedent(
            workflow.split("<<'RUBY'\n", 1)[1].split("\n          RUBY", 1)[0]
        )
        # Execute the actual embedded Fastfile against the pinned deliver method's
        # destructive default. The lane itself must never contact Apple in a test.
        stub = r'''
          module Deliver
            class UploadMetadata
              attr_reader :options, :events
              def initialize(options)
                @options = options
                @events = []
              end
              def upload(version)
                events << [:release_notes, options[:release_notes]]
                events << [:review_notes, options[:app_review_information]]
                review_attachment_file(version)
              end
              private
              def review_attachment_file(version)
                events << [:attachment, version]
                version.clear
                if options[:app_review_attachment_file]
                  raise "replacement failed" if options[:app_review_attachment_file] == "bad.pdf"
                  version << options[:app_review_attachment_file]
                end
              end
            end
          end
          $LOADED_FEATURES << "deliver/upload_metadata.rb"
          def default_platform(*)
          end
          def platform(*)
            yield
          end
          def lane(*)
          end
          eval(STDIN.read, TOPLEVEL_BINDING, "Fastfile")

          metadata = {
            release_notes: {"en-US" => "Release notes"},
            app_review_information: {notes: "Review instructions"}
          }
          [nil, "replacement.pdf"].each do |replacement|
            options = metadata.dup
            options[:app_review_attachment_file] = replacement if replacement
            uploader = Deliver::UploadMetadata.new(options)
            attachments = ["existing.pdf"]
            uploader.upload(attachments)
            expected = replacement ? [replacement] : ["existing.pdf"]
            raise "attachment was not preserved or replaced" unless attachments == expected
            expected_events = [[:release_notes, metadata[:release_notes]],
                               [:review_notes, metadata[:app_review_information]]]
            expected_events << [:attachment, attachments] if replacement
            raise "metadata or replacement path was skipped" unless uploader.events == expected_events
          end

          uploader = Deliver::UploadMetadata.new(metadata.merge(app_review_attachment_file: "bad.pdf"))
          begin
            uploader.upload(["existing.pdf"])
            raise "replacement error was swallowed"
          rescue RuntimeError => error
            raise unless error.message == "replacement failed"
          end
        '''
        result = subprocess.run(
            [shutil.which("ruby"), "-e", textwrap.dedent(stub)],
            input=fastfile,
            text=True,
            capture_output=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("app_review_information", fastfile)
        self.assertIn("release_notes:", fastfile)
        self.assertNotIn("skip_metadata", fastfile)

    def test_only_supported_operator_entrypoint_is_active(self) -> None:
        self.assertTrue((ROOT / "scripts/distribute").exists())
        self.assertFalse((ROOT / "scripts/release").exists())
        self.assertTrue((ROOT / "scripts/legacy/release/local-build-and-publish").exists())


if __name__ == "__main__":
    unittest.main()
