# Profile metadata across restart

## Reproduction

The regression was reproduced against `d4878baa` using generated identities,
temporary SQLite directories, and the in-process `TestRelay` bound to an
ephemeral loopback port. The test runs `start_primary_session`, receives and
persists an older kind-0 event through `handle_relay_event`, shuts the core down,
and publishes a newer profile through a separate SDK client while Iris is
stopped. It then restores the account bundle and processes the actual startup
and network messages.

Before the fix:

- The startup test failed with `restart published 1 newer metadata replacements
  from stale cache`. The emitted event contained the old cached name and lacked
  the newer remote picture, bio, custom fields, and tags.
- The cached-profile test failed with `cached self metadata must not skip
  refresh on sign in`.

The local relay keeps event history rather than deleting superseded events.
Assertions inspect the signed events it receives, and the final regression also
uses a fresh SDK client to fetch and select the newest profile. This establishes
the destructive timestamp ordering and the corrected remote result without
depending on a public relay's storage policy.

## Change

Cached identity publication now uses the profile's persisted timestamp. Startup
and nearby sharing cannot promote stale metadata into a new edit. The signed-in
user's profile bypasses the cache shortcut when fetching; peer cache behavior
and in-flight request deduplication remain intact.

Explicit edits use a timestamp strictly after the cached profile and at least
the current time. Deletion uses the same ordering so deleting immediately after
an edit still supersedes it. Existing preservation of unmodeled JSON fields and
tags is exercised through publication and SQLite restart.

## Verification

Ten focused core tests pass, covering:

- Older cache and newer remote name, display name, avatar, and bio across bundle
  restore, secret-key sign-in, and another restart after a local edit.
- Fresh-client reads of the newest profile on the local relay.
- Preservation of website, nested custom JSON, and custom tags.
- An intentional edit after a slightly future-dated remote profile, followed
  immediately by deletion.
- Cached-self refresh, profile creation, nearby profile publication, existing
  metadata-edit preservation, picture-upload completion, and the two existing
  targeted peer-profile fetch paths.

Run the focused suite from the repository root:

```sh
IRIS_DEFAULT_RELAYS=ws://127.0.0.1:9 \
IRIS_DEVICE_APPROVAL_RELAY_URL=ws://127.0.0.1:9 \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo test --manifest-path core/Cargo.toml --locked --lib -- \
  core::tests::profile_metadata_ \
  core::tests::editing_profile_preserves_extra_metadata_fields_and_tags \
  core::tests::local_identity_artifacts_offer_profile_metadata_to_nearby \
  core::tests::profile_picture_upload_propagates_to_account_snapshot \
  core::tests::opening_uncached_direct_chat_starts_targeted_profile_fetch \
  core::tests::incoming_uncached_direct_message_starts_targeted_profile_fetch \
  core::tests::create_account_without_name_still_offers_profile_metadata_to_nearby \
  core::tests::delete_profile_metadata_publishes_blank_profile_and_clears_local_record \
  --test-threads=2
```

Formatting, whitespace, and `scripts/check-rust-panics` checks pass. The repository source-size check fails
on the unchanged `core/src/core/fips_nearby.rs`: 1,014 lines versus the 1,000-line
limit, also present at the reproduction base. No source-size exception was added.

Full platform/release gates and live-profile recovery are outside this focused
verification. This change prevents stale startup publication; it cannot recover
metadata already overwritten on remote servers. Concurrent edits based on
fields not yet fetched are not automatically merged.
