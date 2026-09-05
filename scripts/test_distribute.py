#!/usr/bin/env python3

import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

from test_release_manifest import TAG, names


ROOT = Path(__file__).resolve().parents[1]
DISTRIBUTE = ROOT / "scripts" / "distribute"
HASHTREE_NPUB = "npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap"
ZAPSTORE_NPUB = "npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm"


class DistributeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.release = self.root / "release"
        self.release.mkdir()
        for name in names(TAG):
            (self.release / name).write_bytes(name.encode())
        self.manifest = self.release / f"iris-chat-{TAG}-manifest.json"
        subprocess.run(
            [
                str(ROOT / "scripts" / "release-manifest.py"),
                "create",
                "--tag",
                TAG,
                "--commit",
                "abc123",
                "--asset-dir",
                str(self.release),
                "--out",
                str(self.manifest),
            ],
            check=True,
        )

        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "commands.log"
        self.zapstore_config = self.root / "generated-zapstore.yaml"
        self.hashtree_nsec = self.root / "htree-nsec"
        self.zapstore_nsec = self.root / "zapstore-nsec"
        self.hashtree_nsec.write_text("hashtree-secret\n")
        self.zapstore_nsec.write_text("zapstore-secret\n")
        self.hashtree_config = self.root / "release profile"
        self.global_config = self.root / "unrelated profile"
        self.global_config.mkdir()
        (self.global_config / "keys").write_text("unrelated identity\n")
        self.notes = (
            "# Iris Chat Release Notes\n\n"
            f"## {TAG}\n\n"
            "### GitHub\n\n- Change.\n\n"
            "### Apple\n\n- Change.\n\n"
            "### Zapstore\n\n- Change.\n"
        )
        self.write_stub(
            "gh",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            printf 'gh %s\n' "$*" >> "$FAKE_COMMAND_LOG"
            if [[ "${FAKE_ATTESTATION_FAIL:-0}" == "1" && "$1 $2" == "attestation verify" ]]; then
              exit 1
            fi
            if [[ "$1 $2" == "release view" ]]; then
              if [[ " $* " == *" --json body "* ]]; then
                printf '%s\n' '{"body":"# Release notes"}'
              elif [[ " $* " == *" --jq .body "* ]]; then
                printf '%s\n' '# Release notes'
              else
                printf '%s\n' "$FAKE_RELEASE_VIEW_JSON"
              fi
              exit 0
            fi
            if [[ "$1 $2" == "release verify" || "$1 $2" == "release verify-asset" ]]; then
              exit 0
            fi
            if [[ "$1 $2" == "release download" ]]; then
              pattern=""
              directory=""
              while [[ $# -gt 0 ]]; do
                case "$1" in
                  --pattern) pattern="$2"; shift 2 ;;
                  --dir) directory="$2"; shift 2 ;;
                  *) shift ;;
                esac
              done
              cp "$FAKE_RELEASE_DIR/$pattern" "$directory/$pattern"
              exit 0
            fi
            if [[ "$1" == "attestation" ]]; then
              exit 0
            fi
            if [[ "$1" == "api" ]]; then
              printf '%s' "$FAKE_RELEASE_NOTES_BASE64"
              exit 0
            fi
            exit 2
            """,
        )
        self.write_stub(
            "nak",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            if [[ "$1 $2" == "key public" ]]; then
              case "$3" in
                hashtree-secret) printf '%s\n' hashtree-hex ;;
                zapstore-secret) printf '%s\n' zapstore-hex ;;
                *) printf '%s\n' wrong-hex ;;
              esac
            elif [[ "$1 $2" == "encode npub" ]]; then
              case "$3" in
                hashtree-hex) printf '%s\n' "$FAKE_HASHTREE_NPUB" ;;
                zapstore-hex) printf '%s\n' "$FAKE_ZAPSTORE_NPUB" ;;
                *) printf '%s\n' npub1wrong ;;
              esac
            fi
            """,
        )
        self.write_stub(
            "htree",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            printf 'htree %s\n' "$*" >> "$FAKE_COMMAND_LOG"
            if [[ "${HTREE_CONFIG_DIR:-}" != "$FAKE_EXPECTED_HTREE_CONFIG_DIR" ||
                  "${HTREE_DATA_DIR:-}" != "$IRIS_HASHTREE_DATA_DIR" ]]; then
              echo "htree did not receive the isolated release profile" >&2
              exit 27
            fi
            if [[ "$1" == "user" && $# -ne 1 ]]; then
              echo "publishing must not switch identities" >&2
              exit 28
            fi
            case "$1" in
              user) printf '%s\n' "$FAKE_ACTIVE_HTREE_NPUB" ;;
              add) printf '%s\n' 'cid: fake-cid' ;;
              release) exit 0 ;;
            esac
            """,
        )
        self.write_stub(
            "zsp",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            printf 'zsp %s\n' "$*" >> "$FAKE_COMMAND_LOG"
            for arg in "$@"; do
              if [[ "$arg" == *.yaml ]]; then
                cp "$arg" "$FAKE_ZAPSTORE_CONFIG"
                break
              fi
            done
            exit 0
            """,
        )
        self.write_stub(
            "curl",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            printf 'curl %s\n' "$*" >> "$FAKE_COMMAND_LOG"
            output=""
            url=""
            while [[ $# -gt 0 ]]; do
              case "$1" in
                -o) output="$2"; shift 2 ;;
                https://*) url="$1"; shift ;;
                *) shift ;;
              esac
            done
            if [[ "$url" == *"/api/nostr/resolve/"* ]]; then
              [[ "${FAKE_REFRESH_FAIL:-0}" != "1" ]] || exit 22
              printf '%s\n' '{"hash":"refreshed-root"}' > "$output"
            else
              [[ "${FAKE_READBACK_FAIL:-0}" != "1" ]] || exit 22
              printf '{"tag":"%s","commit":"%s"}\n' \
                "${FAKE_PUBLIC_TAG:-$FAKE_TAG}" "${FAKE_PUBLIC_COMMIT:-abc123}" > "$output"
            fi
            """,
        )
        self.write_stub(
            "git",
            r"""
            #!/usr/bin/env bash
            set -Eeuo pipefail
            if [[ "$1" == "ls-remote" && "$2" == https://upload.iris.to/* ]]; then
              exit 1
            fi
            exec /usr/bin/git "$@"
            """,
        )

    def tearDown(self) -> None:
        self.directory.cleanup()

    def write_stub(self, name: str, source: str) -> None:
        path = self.bin / name
        path.write_text(textwrap.dedent(source).lstrip())
        path.chmod(0o755)

    def environment(self) -> dict[str, str]:
        env = os.environ.copy()
        release_assets = [*names(TAG), f"iris-chat-{TAG}-manifest.json"]
        env.update(
            {
                "PATH": f"{self.bin}:{env['PATH']}",
                "FAKE_COMMAND_LOG": str(self.log),
                "FAKE_RELEASE_DIR": str(self.release),
                "FAKE_RELEASE_VIEW_JSON": json.dumps(
                    {
                        "assets": [{"name": name} for name in release_assets],
                        "isDraft": False,
                        "isImmutable": True,
                        "isPrerelease": False,
                        "publishedAt": "2026-07-28T10:00:00Z",
                        "tagName": TAG,
                        "url": "https://example.invalid/release",
                    }
                ),
                "FAKE_RELEASE_NOTES_BASE64": subprocess.check_output(
                    ["base64"], input=self.notes, text=True
                ).strip(),
                "FAKE_HASHTREE_NPUB": HASHTREE_NPUB,
                "FAKE_ZAPSTORE_NPUB": ZAPSTORE_NPUB,
                "FAKE_ZAPSTORE_CONFIG": str(self.zapstore_config),
                "FAKE_ACTIVE_HTREE_NPUB": HASHTREE_NPUB,
                "FAKE_EXPECTED_HTREE_CONFIG_DIR": str(self.hashtree_config),
                "FAKE_TAG": TAG,
                "HTREE_CONFIG_DIR": str(self.global_config),
                "HTREE_DATA_DIR": str(self.root / "unrelated data"),
                "IRIS_HASHTREE_NSEC_PATH": str(self.hashtree_nsec),
                "IRIS_HASHTREE_CONFIG_DIR": str(self.hashtree_config),
                "IRIS_HASHTREE_DATA_DIR": str(self.root / "htree-data"),
                "IRIS_ZAPSTORE_NSEC_PATH": str(self.zapstore_nsec),
            }
        )
        return env

    def run_distribution(
        self, channel: str, *extra: str, env=None
    ) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            [str(DISTRIBUTE), channel, "--tag", TAG, *extra],
            env=env or self.environment(),
            capture_output=True,
            text=True,
        )
        self.assertEqual((self.global_config / "keys").read_text(), "unrelated identity\n")
        return result

    def test_default_hashtree_profile_ignores_inherited_global_config(self) -> None:
        env = self.environment()
        del env["IRIS_HASHTREE_CONFIG_DIR"]
        env["FAKE_EXPECTED_HTREE_CONFIG_DIR"] = str(
            Path.home() / ".config/iris-chat/htree-release-config"
        )
        result = self.run_distribution("hashtree", "--check", env=env)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_zapstore_check_downloads_only_manifest_and_apk(self) -> None:
        result = self.run_distribution("zapstore", "--check")
        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text()
        self.assertIn(f"--pattern iris-chat-{TAG}-android-arm64.apk", log)
        self.assertNotIn("aarch64-apple-darwin", log)
        self.assertNotIn("zapstore-secret", result.stdout + result.stderr)
        self.assertIn("zsp publish --check", log)

    def test_zapstore_config_uses_absolute_existing_icon(self) -> None:
        result = self.run_distribution("zapstore", "--check")
        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.zapstore_config.read_text()
        icon_path = ROOT / "android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png"
        self.assertTrue(icon_path.is_file())
        self.assertIn(f"icon: {icon_path}\n", config)

    def test_hashtree_check_never_publishes(self) -> None:
        result = self.run_distribution("hashtree", "--check")
        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text()
        self.assertIn(f"iris-{TAG}-x86_64-unknown-linux-gnu.tar.gz", log)
        self.assertNotIn("htree add", log)
        self.assertNotIn("htree release publish", log)
        self.assertNotIn("curl", log)

    def test_homebrew_check_downloads_only_cli_archives(self) -> None:
        result = self.run_distribution("homebrew", "--check")
        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text()
        self.assertIn(f"iris-{TAG}-aarch64-apple-darwin.tar.gz", log)
        self.assertIn(f"iris-{TAG}-x86_64-unknown-linux-gnu.tar.gz", log)
        self.assertNotIn("android-arm64.apk", log)
        self.assertNotIn("htree add", log)

    def test_rejects_hashtree_active_identity_mismatch(self) -> None:
        env = self.environment()
        env["FAKE_ACTIVE_HTREE_NPUB"] = ZAPSTORE_NPUB
        result = self.run_distribution("hashtree", "--check", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Active Hashtree identity mismatch", result.stderr)

    def test_rejects_hashtree_signer_mismatch(self) -> None:
        self.hashtree_nsec.write_text("wrong-secret\n")
        result = self.run_distribution("hashtree", "--check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Hashtree signer mismatch", result.stderr)

    def test_rejects_attestation_failure(self) -> None:
        env = self.environment()
        env["FAKE_ATTESTATION_FAIL"] = "1"
        result = self.run_distribution("zapstore", "--check", env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("zsp publish", self.log.read_text())

    def test_requires_explicit_tag(self) -> None:
        result = subprocess.run(
            [str(DISTRIBUTE), "zapstore"],
            env=self.environment(),
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("--tag must be an explicit release-date tag", result.stderr)

    def test_rejects_draft_prerelease_and_mutable_releases(self) -> None:
        cases = (
            ("isDraft", "draft"),
            ("isPrerelease", "prerelease"),
            ("isImmutable", "not an immutable"),
        )
        for field, message in cases:
            with self.subTest(field=field):
                env = self.environment()
                metadata = json.loads(env["FAKE_RELEASE_VIEW_JSON"])
                metadata[field] = False if field == "isImmutable" else True
                env["FAKE_RELEASE_VIEW_JSON"] = json.dumps(metadata)
                result = self.run_distribution("zapstore", "--check", env=env)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_rejects_missing_or_unexpected_release_inventory(self) -> None:
        for mutation in ("missing", "unexpected"):
            with self.subTest(mutation=mutation):
                env = self.environment()
                metadata = json.loads(env["FAKE_RELEASE_VIEW_JSON"])
                if mutation == "missing":
                    metadata["assets"].pop()
                else:
                    metadata["assets"].append({"name": "latest.apk"})
                env["FAKE_RELEASE_VIEW_JSON"] = json.dumps(metadata)
                result = self.run_distribution("zapstore", "--check", env=env)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("exact canonical", result.stderr)
                self.assertNotIn("zsp publish", self.log.read_text())

    def test_rejects_modified_asset_digest(self) -> None:
        apk = self.release / f"iris-chat-{TAG}-android-arm64.apk"
        apk.write_bytes(b"modified")
        result = self.run_distribution("zapstore", "--check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mismatch", result.stderr)
        self.assertNotIn("zsp publish", self.log.read_text())

    def test_rejects_zapstore_signer_mismatch(self) -> None:
        self.zapstore_nsec.write_text("wrong-secret\n")
        result = self.run_distribution("zapstore", "--check")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Zapstore signer mismatch", result.stderr)

    def test_publish_retries_are_idempotent_and_do_not_leak_secrets(self) -> None:
        first = self.run_distribution("zapstore")
        second = self.run_distribution("zapstore")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(second.returncode, 0, second.stderr)
        output = first.stdout + first.stderr + second.stdout + second.stderr
        self.assertNotIn("zapstore-secret", output)
        log = self.log.read_text()
        self.assertEqual(log.count("zsp publish"), 2)
        self.assertEqual(log.count("--overwrite-release"), 2)
        self.assertNotIn("latest", log)

    def test_hashtree_publish_uses_exact_tag_and_is_retryable(self) -> None:
        first = self.run_distribution("hashtree")
        second = self.run_distribution("hashtree")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(second.returncode, 0, second.stderr)
        output = first.stdout + first.stderr + second.stdout + second.stderr
        self.assertNotIn("hashtree-secret", output)
        log = self.log.read_text()
        command = f"htree release publish releases/iris-chat-rs {TAG} fake-cid"
        self.assertEqual(log.count(command), 2)
        refresh = f"https://upload.iris.to/api/nostr/resolve/{HASHTREE_NPUB}/releases%2Firis-chat-rs?refresh=1"
        readback = f"https://upload.iris.to/{HASHTREE_NPUB}/releases%2Firis-chat-rs/{TAG}/release.json"
        self.assertLess(log.index(command), log.index(refresh))
        self.assertLess(log.index(refresh), log.index(readback))
        self.assertEqual(log.count(refresh), 2)
        for line in log.splitlines():
            if line.startswith("curl "):
                self.assertIn("--connect-timeout 10", line)
                self.assertIn("--max-time 30", line)
        self.assertNotIn("latest", log)

    def test_hashtree_publish_rejects_stale_or_mismatched_public_metadata(self) -> None:
        for field, value in (("FAKE_PUBLIC_TAG", "v2026.7.1"), ("FAKE_PUBLIC_COMMIT", "other")):
            with self.subTest(field=field):
                env = self.environment()
                env[field] = value
                result = self.run_distribution("hashtree", env=env)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("gateway readback does not match", result.stderr)
                self.assertNotIn(f"Published {TAG} to Hashtree:", result.stdout)

    def test_hashtree_publish_propagates_refresh_and_readback_failures(self) -> None:
        for field in ("FAKE_REFRESH_FAIL", "FAKE_READBACK_FAIL"):
            with self.subTest(field=field):
                env = self.environment()
                env[field] = "1"
                result = self.run_distribution("hashtree", env=env)
                self.assertEqual(result.returncode, 22)
                self.assertNotIn(f"Published {TAG} to Hashtree:", result.stdout)

    def test_homebrew_publish_is_retryable(self) -> None:
        first = self.run_distribution("homebrew")
        second = self.run_distribution("homebrew")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(second.returncode, 0, second.stderr)
        log = self.log.read_text()
        self.assertEqual(log.count("htree add"), 2)
        self.assertNotIn("latest", log)


if __name__ == "__main__":
    unittest.main()
