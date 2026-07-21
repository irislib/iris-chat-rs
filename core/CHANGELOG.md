# Changelog

## 0.1.43

- Update FIPS to 0.4.34 for reliable direct-path recovery across network
  changes, reconnects, handshakes, and rekeys.
- Update the FIPS peer adapter to `nostr-pubsub-fips` 0.4.7 while keeping
  `nostr-pubsub` on the newest 0.1.13 release.

## 0.1.42

- Add portable FIPS BLE v2 transport on iOS and Android, using BLE discovery
  only to bootstrap authenticated, length-framed L2CAP links. Nearby Iris
  events now reuse normal signed-event ingestion, delivery channels, and
  durable reconnect replay.
- Use Osiris and LNVPS as the built-in authenticated FIPS WebSocket entry
  points while preserving an explicit environment override for isolated and
  operator-managed deployments.
- Update the peer path to `nostr-pubsub-fips` 0.4.3 and add the
  `nostr-pubsub-relay` 0.1.11 adapter so signed update announcements arrive
  through both connected FIPS peers and configured Nostr relays.

## 0.1.41

- Replace the retired Nostr relay packet carrier with authenticated FIPS
  WebSocket seed peers while preserving ordinary Nostr relays for events and
  discovery/signaling.
- Update device sync and update-announcement pubsub to FIPS 0.4.11 and the
  shared INV/WANT-capable `nostr-pubsub` 0.1.13 / FIPS adapter 0.4.1 stack.

## 0.1.40

- Replace the removed combined same-host store/transport adapter with the
  canonical Hashtree `BlobRouter` and `RoutedStore`: reads share one verified
  local-cache, FIPS-provider, and Blossom route set, while writes and pins stay
  on Chat's explicit primary store.
- Own TCP/FIPS transport lifetime separately from storage policy and update to
  FIPS 0.4.6. Provider miss, failure, or exit still permits standalone reads;
  linked-device and application-owned outbound links remain independent.
- Publish `iris-chat-protocol` 0.1.8 so packaged builds include signed
  invite-owner authorization and bounded pending group-fanout recovery instead
  of relying on the repository-local protocol source.

## 0.1.39

- Upgrade roster-authorized device links from the canonical Nostr relay
  fallback to direct WebRTC through FIPS's existing authenticated port-257
  negotiation service.
- Update to `fips-core` 0.4.4 and `hashtree-fips-transport` 0.4.3. The relay
  remains a low-priority fallback; same-host-only attachment reuse remains
  relay-free and application-owned outbound links remain independent.
- Bind device-sync services before relay ingress, retire stale or disappeared
  TCP/FIPS streams, and retain records until cumulative TCP acknowledgement so
  direct-path promotion and stream replacement converge without data loss.
- Restrict device-sync TCP acceptance and dialing to the owner's sibling roster
  when the endpoint is also connected to same-host Hashtree providers.
- Disable the global `fipsctl` control socket on Chat's embedded endpoint so
  multiple Chat or Iris processes do not contend for daemon-owned state.

## 0.1.38

- Run linked-device FIPS traffic through the canonical `fips-core` 0.4.2 Nostr
  relay adapter, with one low-priority relay route per authorized roster
  sibling and direct WebRTC paths still preferred.
- Reject relay handshakes from devices outside the signed sibling roster, and
  keep single-device same-host attachment reuse completely relay-free.
- Add a feature-gated process fixture for the Iris Stack released-product lab;
  it drives the production Chat attachment-read path without adding another
  blob, discovery, or transport implementation.
- Update the same-host blob route to `hashtree-fips-transport` 0.4.2 so the
  released Chat fixture and Hashtree provider share the corrected relay
  carrier boundary.

## 0.1.37

- Update the embedded endpoint to the FIPS 0.4.1 WebRTC fix and `nostr-identity` 0.4.0.
- Let attachment reads opt into authenticated same-host `hashtree.blob/1` providers through the existing Chat endpoint, including single-device profiles without relays, while retaining the ordinary Blossom path after a provider miss, failure, or exit.
- Keep attachment writes, linked-device peers, public relay/WebRTC links, and standalone operation application-owned.

## 0.1.36

- Carry linked-device requests and snapshots as bounded framed records over TCP/FIPS service 7369.
- Keep authenticated device admission and durable chat Delivered/Seen receipts above the transport.
- Use FIPS 0.4, TCP/FIPS 0.2, and the shared FIPS pubsub 0.3 provider without a duplicate relay-signaling configuration path.

## 0.1.35

- Sync chats, group metadata, recent messages, and relevant AppKeys to newly linked devices with `iris-chat-protocol` 0.1.7.

## 0.1.33

- Publish group roster fact sync and AppKeys-only linked-device authorization with `iris-chat-protocol` 0.1.5.
- Keep native group membership aligned with the web app's shared roster fact snapshots.
- Stop fetching or publishing old roster-op events; owner-signed kind 37368 AppKeys snapshots are authoritative.

## 0.1.32

- Publish the AppKeys roster fact migration with `iris-chat-protocol` 0.1.4 so packaged builds use shared fact-event roster projection and parent tracking.
- Keep linked-device labels through AppKeys updates and republish local identity artifacts after relay/account changes.

## 0.1.31

- Publish the protocol retry fixes with `iris-chat-protocol` 0.1.3 so packaged builds use the new retry scheduler API.
- Let chat recovery retry missing protocol state after restart, offline use, or message-server reconnects.

## 0.1.30

- Update to `nostr-double-ratchet` 0.0.147 so new group messages and one-to-many protocol messages no longer expose sender-key key ids or message counters to message servers.
- Keep recovery compatible with older no-header group messages while avoiding re-exposing hidden counters in sender-key repair requests.

## 0.1.29

- Fetch missing profile metadata on demand so newly discovered chats can fill in names, photos, and profile details without a manual search.
- Make desktop notification decisions come from shared core logic and keep macOS/Linux notifications firing after the app is backgrounded.
- Remove device-key copy/export surfaces from native settings and profile screens.
- Harden sender-key repair handling by returning recoverable errors instead of panicking on unexpected pending-repair states.

## 0.1.28

- Update to `nostr-double-ratchet` 0.0.146 with sender-key repair hardening, authenticated repair requests, repair snapshots, and shared retry helpers.
- Keep group sender-key recovery retryable after restart without moving app scheduling state into the protocol core.
- Resolve ownerless NDR invites through known app-key rosters so first-contact delivery to known devices does not stall.

## 0.1.27

- Advance app-key roster timestamps when a linked device is removed so stale device rosters cannot restore the removed device.

## 0.1.26

- Add group photo persistence and projection so group pictures survive restart and appear in chat lists, open chats, and group details.

## 0.1.25

- Publish clearer linked-device labels from every native platform so device lists show the app, OS, and device family where available.
- Keep direct messages to a newly restored linked device queued until its AppKeys invite arrives, then retry delivery automatically.

## 0.1.24

- Harden logout and Delete all local data across iOS, Android, Linux, Windows, and CLI so secret key clear is verified before local app data is deleted.
- Rebind the iOS/macOS Rust core after local reset so restoring a profile with a secret key starts from a fresh writable database.
- Remove catch-up time and count bounds so old messages can be found after restoring or reconnecting a device.
- Keep stale device keys from surviving logout/reset, avoiding sends to device sessions that the phone no longer has.

## 0.1.23

- Add shared update policy for automatic update checks, so desktop shells can decide when to poll while core keeps the timing rules consistent.
- Add self-update support for Android APK installs, including update discovery, download, and handoff to the system installer.
- Reuse the existing tabbed QR code sheet for New Chat show/scan actions.
- Simplify group creation across iOS, Android, and Linux: typed or pasted user IDs are added automatically and the selected member list stays above the input.
- Keep nearby mailbag sync status fresher and make nearby peer rows open the expected peer flow.
- Fix iOS multi-image album tile sizing when an album has more than four images.

## 0.1.22

- Add Signal-style image album layouts for multiple images, including side-by-side pairs, three-image mosaics, four-image grids, and larger-album overlays.
- Add a swipe-through image carousel with sender, date, share, and forward actions.
- Show image thumbnails in the composer attachment row.
- Report upload progress as chunks land on the network.

## 0.1.21

- Nearby ingest now tags transport channels with the relaying peer name (e.g. "bluetooth · Alice") so message-info Transport rows show who carried each event.
- Add nearby_enabled master preference with kill-switch behavior — turning it off forces nearby_bluetooth_enabled and nearby_lan_enabled to false.
- Extend the accept_unknown_direct_messages gate to silently drop incoming public-invite responses from senders we have no thread with.
- Schema bumped to v16 with `nearby_enabled` column (default 1).
- Expose `should_accept_direct_runtime_message` to sibling modules so the invite path reuses the DM gate.

## 0.1.20

- Improve iOS chat list ergonomics with UIKit swipe actions, calmer navigation chrome, and a search close button that dismisses the keyboard.
- Align Android chat search, settings, setup, and group management screens with simpler Signal-style flows.
- Keep session restore and loading states visually steadier across native shells.

## 0.1.19

- Add Homebrew tap packaging for the `iris` CLI and wire tap updates into the htree release publish path.
- Restore desktop update controls with automatic checks, automatic installs, and a signed macOS htree updater archive.
- Add desktop chat-list context actions for read/unread, pin, mute, and delete.
- Harden mobile push notification payload parsing so APNS/FCM event aliases decrypt the same as canonical event payloads.
- Suppress generic mobile push placeholders when an encrypted message notification cannot be decrypted into sender/message text.
- Keep iOS foreground notification handling aligned with the Notification Service Extension so generic "New message" fallbacks are not shown.
- Keep iOS notification state readable by the Notification Service Extension while Iris is backgrounded or the phone is locked.

## 0.1.18

- Keep direct runtime rumor author validation canonical to the authenticated owner pubkey.
- Require parsed runtime rumors to carry a verified inner event id and use that id as the app message id.

## 0.1.17

- Switch AppCore to the protocol engine path and remove legacy NDR runtime state migration.
- Use verified Nostr rumor event ids for group message identity and fanout dedupe.
- Update to `nostr-double-ratchet` 0.0.142 with unsigned inner event id and author hardening.

## 0.1.16

- Republish 0.1.15 bundle as 3.0.17 because TestFlight rejected the 3.0.16 upload (cfBundleVersion 31600 was already on file from an earlier partial upload).

## 0.1.15

- Plumb the marketing version (`IRIS_APP_VERSION_NAME`) all the way into the running binary so `iris --version`, the FFI app constructor, and the Linux About panel agree on the same string instead of falling back to the cargo crate semver.
- Expose `app_version()` from the core crate so shells don't have to reach for `CARGO_PKG_VERSION`.
- Add a Linux Settings → Updates panel with the current version and a "Check for updates" button that compares against the published htree release.

## 0.1.14

- Update to `nostr-double-ratchet` 0.0.138 with snapshot-only group metadata and the restored legacy sender-key wire format.

## 0.1.13

- Update to `nostr-double-ratchet` 0.0.137 with the merged htree/master private invite, linked-device fanout, and runtime durability work.

## 0.1.12

- Create fresh one-use private invite links in the invite UI instead of exposing the relay-published local device invite secret.
- Subscribe for private invite responses immediately and include them in mobile push invite-response filters.
- Update to split `nostr-double-ratchet` 0.0.136 with private invite owner-key handling.

## 0.1.11

- Update to `nostr-double-ratchet` 0.0.135, including the current direct
  subscription behavior, skipped-key sender coverage, and send-session
  selection parity.

## 0.1.10

- Update to `nostr-double-ratchet` 0.0.134, where low-level sessions only
  encrypt/decrypt unsigned inner events and chat/reaction/typing/expiration
  rumor construction lives in shared builder helpers.

## 0.1.9

- Update to `nostr-double-ratchet` 0.0.133 so Rust runtime/device-roster
  inspection includes stored device sessions even when AppKeys are not cached.

## 0.1.8

- Update to `nostr-double-ratchet` 0.0.132, including the shared runtime
  known-device roster helper and fresh same-owner send regression coverage.

## 0.1.7

- Keep restored same-secret CLI sessions from publishing a one-device AppKeys
  roster before relay backfill has merged existing devices.
- Make `iris sync --wait-ms` wait for protocol catch-up, and keep logged-in
  `iris relay set` from blocking before the new message-server list is saved.
- Add CLI interop coverage for a fresh same-secret client sending to a peer
  while an older session receives the message as its own outgoing sender copy.

## 0.1.6

- Update to `nostr-double-ratchet` 0.0.128 so TypeScript and Rust stacks share
  the same inactive send-capable session recovery release.

## 0.1.5

- Import matching legacy `ndr` filesystem ratchet sessions into Iris SQLite storage on account startup.
- Preserve active Iris ratchet state while filling missing or empty records from legacy storage.

## 0.1.4

- Make `iris listen` run the full Iris core/network listener and own the data-dir lock.
- Move read-only SQLite streaming to `iris tail --follow`.
- Add CLI interop coverage for receiving messages through `iris listen`.

## 0.1.3

- Add `iris sync` for explicit CLI relay catch-up.
- Add group delete, member removal, admin add/remove, and group reaction commands.
- Add Iris CLI interop coverage against an independent `nostr-double-ratchet` protocol client.

## 0.1.2

- Add a per-data-dir core lock so only one writer/ratcheting Iris core can use a data directory at a time.
- Keep `iris listen`, `iris search`, and `iris tail` read-only so they can inspect SQLite without owning the core lock.

## 0.1.1

- Allow `iris send <user-id> ...` and related chat actions to accept direct user IDs without pre-creating a chat.
- Keep `iris send <npub> ...` compatible with the old `ndr send <npub> ...` agent workflow.

## 0.1.0

- Initial crates.io release of the `iris` command line client and `iris_chat_core` library.
