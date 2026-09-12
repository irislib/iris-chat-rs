# CLI profile schema round-trip

Checked 2026-09-12 at 22:46 UTC (13 September locally). The narrow synthetic
avatar workflow passed. Detailed snapshots and binary SHA-256 hashes are in
[the JSON evidence](2026-09-13-cli-profile-schema-roundtrip.json).

## Method and result

The installed CLI reported `iris 2026.7.14`; the development CLI reported
`iris 0.1.45` and was the binary previously tested for commit
`da4240a6556944d9fe38f603010605608c71c44d`. Binary hashes identify what actually ran;
this check does not independently attest either binary's source provenance.

Both binaries first ran `relay list` in separate temporary directories with
`IRIS_DEMO_RELAYS` set to an empty string. Both returned an empty server list.
Every subsequent invocation retained that environment and ran under macOS
`sandbox-exec` with `(version 1)(allow default)(deny network*)`. No live profile,
relay, device, installed binary or release was changed.

One further temporary directory then exercised these separate processes:

1. Old CLI: `account create --name "Synthetic Profile"`. SQLite schema was 26.
2. Seed its synthetic cached profile with a picture, bio, extra JSON and tags.
3. New CLI: `account profile --picture-url https://example.com/after.png`.
   Schema migrated to 30; name, bio, extra metadata and tags were preserved.
4. New CLI: `account profile`. The edited picture survived reopening.
5. Old CLI: `whoami`. Startup succeeded; profile fields remained intact.
6. New CLI: `account profile`. All checked profile fields still matched.

Every database snapshot returned `PRAGMA integrity_check = ok` and an empty
persisted server list. The old CLI left `user_version` at 30. Temporary synthetic
identities and directories were removed when the harness exited; no test process
remained. A first harness assertion compared tag JSON whitespace; the rerun
compared decoded JSON values and passed. No product fix was needed.

## Compatibility limits

The source migration from 26 adds discovery tables at 27, migrates discovery
app keys at 28, adds discovery account/social state at 29, and adds
`image_proxy_fallback_enabled` at 30. Existing profile columns are unchanged.
Both schema openers accept any version at least their own supported version.
Thus the old binary **opened a schema-30 database**; this was not a database
migration back to schema 26 or a general downgrade guarantee.

The source tagged `v2026.7.14` republishes cached metadata without preserving its
original timestamp. The newer source contains the correction documented in
[profile metadata across restart](../profile-metadata-restart-verification.md).
An old client with stale cache can therefore have different remote behavior;
this offline round-trip provides no relay-publication or remote overwrite proof.

This fixture contained no chats, linked devices, queued delivery, discovery
history or subsequent remote profile edits. Their compatibility was not tested.
The CLI truthfully reported local persistence requested and network publication
unverified; persistence in this fixture was established by SQLite and process
reopen checks. These results alone do not authorize upgrading a live data folder.
