# Production AdHoc preparation

`ios_adhoc_prepare.py` prepares a copy of an already accepted official IPA for
the production AdHoc route in `RELEASE.md`. It does not install or launch the
app, obtain profiles, contact Apple, create a release receipt, or accept physical
tests. Run its offline regressions with:

```sh
python3 -B scripts/test_ios_adhoc_prepare.py
```

Actual preparation requires `--input`, `--input-sha256`, a fresh `--out`
directory, and `--execute`. The release owner must first review the source,
qualify the actual platform tools, coordinate device ownership and disk capacity,
and prepare current private inputs. No input values, device identifiers,
certificates, profiles, tool paths or outputs belong in Git.

The `iris-adhoc-preparation-input-v1` private JSON contains:

- `candidate`: the six exact candidate fields from `ios_runtime_acceptance.py`.
- `ipa`, `manifest`, `release_acceptance`, `authorization`, `certificate`:
  absolute regular-file bindings with exactly `path` and `sha256`. The
  certificate is the existing distribution leaf in DER format; no private key
  is exported. Authorization retains the existing explicit user instruction,
  with `route: "authorized-production-adhoc"` and
  `direct_install_authorized: true`.
- `target`: `device`, `coredevice_id`, `observed_utc`, and bindings named
  `inventory` and `reservation`. The inventory is the raw CoreDevice device-list
  JSON. The reservation records the same two identifiers and `started_utc` /
  `expires_utc`. The device must be connected and the inventory no more than
  120 seconds old when preparation begins and again immediately before signing.
- `profiles`: exactly `app`, `notification-service`, and `share-extension`.
  Each role contains `profile` and `api` file bindings plus `api_observed_utc`.
  The API file is an unchanged Apple profile response, with either one object
  or an array in `data`; its exact profile content must match the local file.
  The API observation must be no more than 300 seconds old. Only an active,
  unexpired `IOS_APP_ADHOC` profile provisioning that single reserved device
  is supported. Existing profiles are reused; the tool cannot create any.
- `tools`: bindings for the reviewed system `security` and `codesign` tools.
- `minimum_free_bytes`: the host's required reserve. Preparation additionally
  requires space for the declared extracted payload plus 64 MiB. Keep the
  outer task resource guard active; this admission check does not monitor other
  writers or replace that guard.

The synthetic fixture in `test_ios_adhoc_prepare.py` demonstrates the complete
shape. It is not evidence for any real target.

Preparation verifies the immutable accepted manifest and IPA, captures original
signature verification, entitlement and certificate results, and checks current
profiles against the exact device, bundle, team, certificate and retained
entitlements. The selected private key must be locally available and valid.
For Associated Domains, the profile's scalar `*` allowlist grant authorizes the
app's unchanged array of domain strings. This narrow type exception does not
apply to other capabilities; every signed entitlement remains exact. See
[Apple's profile allowlist explanation](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles)
and [entitlement wildcard guidance](https://developer.apple.com/library/archive/technotes/tn2415/).
Repeated valid keychain entries with the same exact certificate fingerprint are
one signing selection; unrelated fingerprints and text in identity labels do
not satisfy it. The actual re-signed leaf must still equal the selected DER
certificate exactly.
Certificate-chain checks use system code-signing trust with local certificates
and cached revocation information only; their outputs do not claim a fresh
online revocation check. The exact current Apple profile response remains a
separate bound input.

The original IPA stays unchanged. Its optional `Symbols` and `Signatures`
sidecars remain unchanged in the private extraction; only the app is an install
candidate. One new extraction is re-signed, extensions
first and app last. Debugging is disabled, the main app retains production APNs,
and the only permitted entitlement removal is `beta-reports-active`. All other
entitlements and non-signature resources must remain exact. Any extra framework,
watch app, unknown extension, link, encrypted image or unsupported Mach-O layout
fails. A failure preserves the partial output and command logs; it is not an
automatic retry or permission to install that output.

## Executable comparison

The explicit policy `terminal-codesign-region-and-linkedit-extent-v2` supports thin
arm64 executables. It preserves the full original and re-signed load-command
bytes and raw digests in the private record. To compare unsigned content, it
excludes only the structurally validated terminal `LC_CODE_SIGNATURE` region.
It represents the `LC_CODE_SIGNATURE` size as zero and `__LINKEDIT` file/VM
extent as the unsigned extent, with the standard 16 KiB VM alignment. The
signature offset and linkedit offset must remain identical. All other command
bytes, header fields, UUID, instructions and unsigned data remain hash-bound.
Other segment and data references must not overlap the signature; unsupported
load commands fail rather than being ignored.

The embedded signature blob may be shorter than its allocated region. Zero
padding is accepted. Nonzero residual bytes are accepted only when the entire
tail, including its zero bytes, exactly matches the independently hash-verified
official IPA at identical offsets, entirely inside both original and re-signed
terminal signature regions. The original must independently pass structural
validation; signature starts cannot move. The record preserves both raw region
and tail hashes, blob length, tail length and the exact original file digest.
This handles unchanged old signature residue left by `codesign` without erasing
bytes or excluding any additional instruction or load-command content.

This is **not** a claim that full signed binaries or all raw load-command bytes
are identical. The exact signature-layout delta is visible in `originalCode`
and `resignedCode`. The owner must review that actual delta before accepting
provenance. The policy does not allow normalization of arbitrary differences,
changes to instruction data, or changes to non-signature command content.

## Handoff to ordinary-launch verification

`preparation.json` binds the original signing capture, profiles, exact candidate,
target, tool invocations, code comparisons and prepared app path. Its
`code_bindings` already has the original/re-signed triples expected by
`ios_ordinary_relaunch.validate_distribution`: `uuid`, `loadCommandsSHA256` and
`unsignedContentSHA256` for all three roles. Their digest encoding is the
unchanged v3 sorted, compact JSON encoding.

The record deliberately says `physicalAcceptance: false`,
`installationPerformed: false`, and `ownerProvenanceReviewRequired: true`.
After a separately verified in-place installation, the release owner must bind
the actual installation receipt, previous build, installed URL and finish time,
then independently review the complete provenance chain. Only that reviewed
chain can supply the launch helper's `provenance_qualification` and normalized
private distribution input. This preparation tool does not manufacture those
records or infer installation, preserved data, normal launch or runtime success.
