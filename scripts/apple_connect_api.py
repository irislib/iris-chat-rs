"""Small App Store Connect client; credentials and signing are loaded on request.

Callers may override ``record(metadata)`` to retain method/resource/status only.
Load the existing release environment before calling; this module never loads it.
"""
from __future__ import annotations

import json
import http.client
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BASE_URL = "https://api.appstoreconnect.apple.com/v1/"
HTTP_TIMEOUT_SECONDS = 30
SIGNING_TIMEOUT_SECONDS = 10
TOKEN_LIFETIME_SECONDS = 600
_CREDENTIALS = ("IRIS_ASC_AUTH_KEY_PATH", "IRIS_ASC_AUTH_KEY_ID", "IRIS_ASC_AUTH_KEY_ISSUER_ID")
_SIGNER = r"""
const fs = require('node:fs');
const crypto = require('node:crypto');
try {
  const {header, payload} = JSON.parse(fs.readFileSync(0, 'utf8'));
  const key = crypto.createPrivateKey(fs.readFileSync(process.env.IRIS_ASC_AUTH_KEY_PATH));
  if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails.namedCurve !== 'prime256v1') {
    throw new Error('Wrong signing key type');
  }
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const input = encode(header) + '.' + encode(payload);
  const signature = crypto.sign('sha256', Buffer.from(input), {key, dsaEncoding: 'ieee-p1363'});
  if (signature.length !== 64) throw new Error('Wrong signature size');
  process.stdout.write(input + '.' + signature.toString('base64url'));
} catch {
  process.stderr.write('App Store Connect signing failed.\n');
  process.exitCode = 1;
}
"""


class AppleConnectError(RuntimeError):
    """A redacted credential, transport, or API error."""


def record(metadata):
    """Optional caller-provided sink. No credential or payload is supplied."""


def create_token():
    """Sign a short-lived ES256 JWT with Node's built-in crypto implementation."""
    if any(not os.environ.get(name, "").strip() for name in _CREDENTIALS):
        raise AppleConnectError("Configure " + ", ".join(_CREDENTIALS) + " before requesting Apple APIs.")
    now = int(time.time())
    claims = {"header": {"alg": "ES256", "kid": os.environ[_CREDENTIALS[1]], "typ": "JWT"},
              "payload": {"iss": os.environ[_CREDENTIALS[2]], "iat": now,
                          "exp": now + TOKEN_LIFETIME_SECONDS, "aud": "appstoreconnect-v1"}}
    try:
        result = subprocess.run(["node", "-e", _SIGNER], input=json.dumps(claims),
                                capture_output=True, text=True, timeout=SIGNING_TIMEOUT_SECONDS)
    except (OSError, subprocess.SubprocessError):
        raise AppleConnectError("App Store Connect signing could not run; check Node.js and the configured key.") from None
    if result.returncode or not re.fullmatch(r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]{86}", result.stdout):
        raise AppleConnectError("App Store Connect signing failed; check the configured P-256 key.")
    return result.stdout


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def _url(resource, params):
    try:
        if not isinstance(resource, str) or not resource or any(ord(c) <= 32 or ord(c) == 127 for c in resource):
            raise ValueError()
        parts = urllib.parse.urlsplit(resource)
        if parts.scheme or parts.netloc:
            url = resource
        elif resource.startswith("/v1/"):
            url = "https://api.appstoreconnect.apple.com" + resource
        else:
            url = BASE_URL + resource.lstrip("/")
        parts = urllib.parse.urlsplit(url)
        path = urllib.parse.unquote(parts.path)
        if (parts.scheme != "https" or parts.netloc != "api.appstoreconnect.apple.com"
                or parts.fragment or not parts.path.startswith("/v1/") or not path.startswith("/v1/")
                or "\\" in path or "%" in path or any(ord(c) <= 32 or ord(c) == 127 for c in path)
                or any(part in (".", "..") for part in path.split("/"))):
            raise ValueError()
        query = parts.query
        if params:
            query += ("&" if query else "") + urllib.parse.urlencode(params, doseq=True)
        return urllib.parse.urlunsplit(parts._replace(query=query)), parts.path
    except (ValueError, TypeError):
        raise AppleConnectError("Use an App Store Connect HTTPS resource under /v1/.") from None


def request(method, resource, params=None, body=None):
    """Return decoded JSON (or {} for an empty response); never retry requests."""
    if not isinstance(method, str) or method.upper() not in {"GET", "POST", "PATCH", "PUT", "DELETE", "HEAD"}:
        raise AppleConnectError("Unsupported App Store Connect request method.")
    method = method.upper()
    url, path = _url(resource, params)
    try:
        data = None if body is None else json.dumps(body).encode("utf-8")
    except (TypeError, ValueError):
        raise AppleConnectError("App Store Connect request body must be JSON-serializable.") from None
    token = create_token()
    req = urllib.request.Request(url, data=data, method=method,
        headers={"Authorization": "Bearer " + token, "Accept": "application/json", "Content-Type": "application/json"})
    status = None
    try:
        with urllib.request.build_opener(_NoRedirect()).open(req, timeout=HTTP_TIMEOUT_SECONDS) as response:
            status = response.status
            raw = response.read()
        if not 200 <= status < 300:
            raise AppleConnectError("App Store Connect returned an unsuccessful HTTP status.")
        try:
            return json.loads(raw) if raw else {}
        except (ValueError, UnicodeError):
            raise AppleConnectError("App Store Connect returned invalid JSON.") from None
    except urllib.error.HTTPError as error:
        status = error.code
        error.close()
        if 300 <= status < 400:
            raise AppleConnectError("App Store Connect redirects are not allowed.") from None
        raise AppleConnectError(f"App Store Connect request failed (HTTP {status}).") from None
    except (urllib.error.URLError, OSError, http.client.HTTPException, UnicodeError):
        raise AppleConnectError("App Store Connect connection failed or timed out.") from None
    finally:
        try:
            record({"method": method, "resource": path, "status": status})
        except Exception:
            raise AppleConnectError("Could not record App Store Connect request metadata.") from None
