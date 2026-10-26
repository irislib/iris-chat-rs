"""Offline API-helper checks using generated keys and an in-memory HTTPS handler."""
import base64
import importlib.util
import io
import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import traceback
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse
import urllib.request
import urllib.response

import apple_connect_api as api


class FakeHTTPS(urllib.request.HTTPSHandler):
    def __init__(self, body=b'{"data":[]}', status=200, headers=None, failure=None):
        super().__init__()
        self.body, self.status = body, status
        self.headers, self.failure = headers or {}, failure
        self.calls = []

    def https_open(self, request):
        self.calls.append(request)
        if self.failure:
            raise self.failure
        response = urllib.response.addinfourl(io.BytesIO(self.body), self.headers, request.full_url, self.status)
        response.msg = "fixture"
        return response


class AppleConnectTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.node = shutil.which("node")
        if not cls.node:
            raise RuntimeError("Node.js is required for the offline ES256 checks")
        cls.directory = tempfile.TemporaryDirectory(prefix="iris-asc-test-")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.key = Path(cls.directory.name) / "generated-key.p8"
        cls.public_key = Path(cls.directory.name) / "generated-public.pem"
        cls.node_environment = {"PATH": os.environ.get("PATH", "")}
        cls.generate_key("prime256v1", cls.key, cls.public_key)

    @classmethod
    def generate_key(cls, curve, private, public):
        subprocess.run([cls.node, "-e", """
const fs = require('node:fs'), crypto = require('node:crypto');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const pair = crypto.generateKeyPairSync('ec', {namedCurve: input.curve});
fs.writeFileSync(input.private, pair.privateKey.export({type:'pkcs8',format:'pem'}), {mode:0o600});
fs.writeFileSync(input.public, pair.publicKey.export({type:'spki',format:'pem'}), {mode:0o600});
"""], input=json.dumps({"curve": curve, "private": str(private), "public": str(public)}),
            text=True, capture_output=True, check=True, timeout=10, env=cls.node_environment)

    def setUp(self):
        self.environment = {**self.node_environment, "IRIS_ASC_AUTH_KEY_PATH": str(self.key),
            "IRIS_ASC_AUTH_KEY_ID": "generated-key-id", "IRIS_ASC_AUTH_KEY_ISSUER_ID": "generated-issuer-id"}
        self.env_patch = patch.dict(os.environ, self.environment, clear=True)
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.records = []
        self.record_patch = patch.object(api, "record", self.records.append)
        self.record_patch.start()
        self.addCleanup(self.record_patch.stop)

    def transport(self, handler):
        real_build = urllib.request.build_opener
        return patch.object(api.urllib.request, "build_opener",
                            side_effect=lambda *handlers: real_build(handler, *handlers))

    def assert_redacted(self, action, forbidden):
        try:
            action()
        except api.AppleConnectError as error:
            rendered = "".join(traceback.format_exception(type(error), error, error.__traceback__))
            for secret in forbidden:
                self.assertNotIn(secret, str(error))
                self.assertNotIn(secret, rendered)
            return str(error)
        self.fail("Expected a redacted AppleConnectError")

    def test_import_does_not_read_credentials_sign_or_open_http(self):
        spec = importlib.util.spec_from_file_location("isolated_apple_connect", api.__file__)
        module = importlib.util.module_from_spec(spec)
        with patch.object(Path, "read_bytes", side_effect=AssertionError("credential read")), \
             patch.object(Path, "open", side_effect=AssertionError("credential read")), \
             patch.object(subprocess, "run", side_effect=AssertionError("signer started")), \
             patch.object(urllib.request, "build_opener", side_effect=AssertionError("HTTP started")), \
             patch.object(os.environ, "get", side_effect=AssertionError("credential lookup")):
            spec.loader.exec_module(module)
        self.assertIsInstance(module.ROOT, Path)
        self.assertTrue(callable(module.record))

    def test_generated_p256_signature_and_jwt_claims(self):
        real_run = subprocess.run
        with patch.object(api.time, "time", return_value=1700000000), \
             patch.object(api.subprocess, "run", wraps=real_run) as signer:
            token = api.create_token()
        header, payload, signature = token.split(".")
        decode = lambda value: base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))
        self.assertEqual(json.loads(decode(header)), {"alg": "ES256", "kid": "generated-key-id", "typ": "JWT"})
        self.assertEqual(json.loads(decode(payload)), {"iss": "generated-issuer-id", "iat": 1700000000,
            "exp": 1700000000 + api.TOKEN_LIFETIME_SECONDS, "aud": "appstoreconnect-v1"})
        self.assertEqual(len(decode(signature)), 64)
        argv = signer.call_args.args[0]
        self.assertNotIn(str(self.key), " ".join(argv))
        self.assertNotIn(token, " ".join(argv))
        self.assertEqual(signer.call_args.kwargs["timeout"], api.SIGNING_TIMEOUT_SECONDS)
        self.assertNotIn("PRIVATE KEY", signer.call_args.kwargs["input"])
        verify = """
const fs = require('node:fs'), crypto = require('node:crypto');
const input = JSON.parse(fs.readFileSync(0,'utf8'));
const parts = input.token.split('.');
const key = {key:input.publicKey,dsaEncoding:'ieee-p1363'};
const valid = crypto.verify('sha256',Buffer.from(parts[0]+'.'+parts[1]),key,Buffer.from(parts[2],'base64url'));
const changed = crypto.verify('sha256',Buffer.from(parts[0]+'.'+parts[1]+'x'),key,Buffer.from(parts[2],'base64url'));
process.stdout.write(JSON.stringify({valid,changed}));
"""
        result = real_run([self.node, "-e", verify], input=json.dumps({"token": token,
            "publicKey": self.public_key.read_text()}), text=True, capture_output=True,
            check=True, timeout=10, env=self.node_environment)
        self.assertEqual(json.loads(result.stdout), {"valid": True, "changed": False})
        self.assertEqual(self.records, [])

    def test_get_parameters_and_metadata_exclude_query_and_token(self):
        handler = FakeHTTPS(b'{"data":[{"id":"fixture"}]}')
        with self.transport(handler):
            result = api.request("get", "profiles", {"filter[name]": "private profile", "include": ["devices", "certificates"]})
        self.assertEqual(result, {"data": [{"id": "fixture"}]})
        self.assertEqual(len(handler.calls), 1)
        req = handler.calls[0]
        self.assertEqual(urllib.parse.parse_qs(urllib.parse.urlsplit(req.full_url).query),
                         {"filter[name]": ["private profile"], "include": ["devices", "certificates"]})
        self.assertEqual(req.timeout, api.HTTP_TIMEOUT_SECONDS)
        self.assertTrue(req.get_header("Authorization").startswith("Bearer "))
        self.assertEqual(self.records, [{"method": "GET", "resource": "/v1/profiles", "status": 200}])

    def test_post_json_and_empty_delete_response(self):
        handler = FakeHTTPS(b'{"data":{"id":"created"}}', status=201)
        body = {"data": {"type": "profiles", "attributes": {"name": "private name"}}}
        with self.transport(handler):
            self.assertEqual(api.request("POST", "/v1/profiles", body=body), {"data": {"id": "created"}})
        self.assertEqual(json.loads(handler.calls[0].data), body)
        self.assertEqual(handler.calls[0].get_header("Content-type"), "application/json")
        handler = FakeHTTPS(b"", status=204)
        with self.transport(handler):
            self.assertEqual(api.request("DELETE", "profiles/fixture"), {})

    def test_same_origin_absolute_pagination_url(self):
        handler = FakeHTTPS()
        with self.transport(handler):
            api.request("GET", api.BASE_URL + "profiles?cursor=opaque", params={"limit": 20})
        self.assertEqual(urllib.parse.parse_qs(urllib.parse.urlsplit(handler.calls[0].full_url).query),
                         {"cursor": ["opaque"], "limit": ["20"]})
        self.assertEqual(self.records[0]["resource"], "/v1/profiles")

    def test_foreign_urls_and_path_escapes_rejected_before_signing(self):
        resources = ("https://example.invalid/v1/profiles", "//example.invalid/v1/profiles",
            "http://api.appstoreconnect.apple.com/v1/profiles", "https://api.appstoreconnect.apple.com.evil/v1/profiles",
            "https://secret@api.appstoreconnect.apple.com/v1/profiles", "https://api.appstoreconnect.apple.com:444/v1/profiles",
            "https://api.appstoreconnect.apple.com/v2/profiles", "../profiles", "%2e%2e/profiles",
            "%252e%252e/profiles", "https://api.appstoreconnect.apple.com/%76%31/profiles", "profiles/%0a",
            "profiles#fragment", "profiles\nsecret")
        with patch.object(api, "create_token") as signer, patch.object(urllib.request, "build_opener") as opener:
            for resource in resources:
                with self.subTest(resource=resource):
                    with self.assertRaises(api.AppleConnectError):
                        api.request("GET", resource)
            signer.assert_not_called()
            opener.assert_not_called()

    def test_all_redirects_are_blocked_without_forwarding_authorization(self):
        for status in (301, 302, 303, 307, 308):
            for destination in ("https://example.invalid/steal", api.BASE_URL + "other", "/v1/other"):
                with self.subTest(status=status, destination=destination):
                    handler = FakeHTTPS(b"private redirect payload", status=status, headers={"Location": destination})
                    with self.transport(handler):
                        with self.assertRaisesRegex(api.AppleConnectError, "redirects are not allowed"):
                            api.request("POST", "profiles", body={"private": "payload"})
                    self.assertEqual(len(handler.calls), 1)
                    self.assertEqual(self.records[-1]["status"], status)

    def test_http_error_is_redacted_and_post_is_not_retried(self):
        handler = FakeHTTPS(b'{"errors":[{"detail":"private-response-secret"}]}', status=403)
        resource = "profiles?filter=private-query-secret"
        body = {"value": "private-body-secret"}
        with self.transport(handler), patch.object(api, "create_token", return_value="private-token-secret"):
            message = self.assert_redacted(lambda: api.request("POST", resource, body=body),
                ["private-response-secret", "private-token-secret", "private-query-secret", "private-body-secret"])
        self.assertIn("HTTP 403", message)
        self.assertEqual(len(handler.calls), 1)
        self.assertEqual(self.records, [{"method": "POST", "resource": "/v1/profiles", "status": 403}])

    def test_connection_error_and_invalid_json_are_redacted(self):
        for handler in (FakeHTTPS(failure=urllib.error.URLError("private-transport-secret")),
                        FakeHTTPS(failure=http.client.IncompleteRead(b"private-transport-secret")),
                        FakeHTTPS(b"private-response-secret")):
            with self.transport(handler):
                self.assert_redacted(lambda: api.request("GET", "profiles"),
                                     ["private-transport-secret", "private-response-secret"])

    def test_missing_credentials_and_bad_key_are_redacted(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(api.AppleConnectError, "IRIS_ASC_AUTH_KEY_PATH"):
                api.create_token()
        with patch.dict(os.environ, {"IRIS_ASC_AUTH_KEY_PATH": "/missing/private-key-path-secret.p8"}):
            self.assert_redacted(api.create_token, ["private-key-path-secret", "generated-issuer-id", "generated-key-id"])

    def test_non_p256_key_is_rejected(self):
        private = Path(self.directory.name) / "wrong-curve.p8"
        public = Path(self.directory.name) / "wrong-curve.pem"
        self.generate_key("secp384r1", private, public)
        with patch.dict(os.environ, {"IRIS_ASC_AUTH_KEY_PATH": str(private)}):
            with self.assertRaisesRegex(api.AppleConnectError, "P-256"):
                api.create_token()

    def test_signer_failure_and_timeout_do_not_leak_output(self):
        failures = (subprocess.TimeoutExpired(["node"], 10, output="private-token-secret", stderr="private-key-secret"),
                    OSError("private-key-secret"))
        for error in failures:
            with patch.object(api.subprocess, "run", side_effect=error):
                self.assert_redacted(api.create_token, ["private-token-secret", "private-key-secret"])
        result = subprocess.CompletedProcess(["node"], 1, stdout="private-token-secret", stderr="private-key-secret")
        with patch.object(api.subprocess, "run", return_value=result):
            self.assert_redacted(api.create_token, ["private-token-secret", "private-key-secret"])


if __name__ == "__main__":
    unittest.main()
