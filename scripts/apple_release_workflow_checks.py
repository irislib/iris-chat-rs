#!/usr/bin/env python3

from pathlib import Path
import json
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


class AppleReleaseWorkflowTests(unittest.TestCase):
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
