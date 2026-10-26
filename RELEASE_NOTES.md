# Iris Chat Release Notes

Each release has channel-specific notes. The release tag must match the `##` heading exactly.

## v2026.10.26.1

### GitHub

- Avoid repeating app-state saves, subscription updates and pending-message retries when unchanged signed device lists and invitations are received again.
- Preserve real authorization updates and make completed invitation recovery durable and visible. Keep anonymous discovery and private message delivery boundaries unchanged.
- Extend populated-account CPU and wire checks to process replayed metadata through the app, with no redundant state writes.
- Add recent recipients to Android's system share sheet, opening shared text and files in the selected chat for review before sending.
- Update the protocol source package to 0.3.11 and the CLI source package to 0.1.55. Message formats and saved state remain compatible with v2026.10.26.

### Apple

- Reduce repeated processing while Iris is open.
- Keep recovered chats visible after an invitation completes.

### Zapstore

- Reduce repeated processing while Iris is open.
- Keep recovered chats visible after an invitation completes.
- Share directly to recent chats from other Android apps.

## v2026.10.26

### GitHub

- Keep isolated discovery connections open after harmless acknowledgements of completed requests, avoiding unnecessary reconnections.
- Preserve separate author and message-server scopes, anonymous authentication and bounded idle processing. Unexpected data, re-authentication, flooding and expiry still discard the connection.
- Update the CLI source package to 0.1.54. The protocol package remains at 0.3.10, with unchanged message formats and saved state.

### Apple

- Avoid unnecessary reconnections to message servers.

### Zapstore

- Avoid unnecessary reconnections to message servers.

## v2026.10.25.3

### GitHub

- Pace group-history decryption using measured work while keeping newly arriving messages responsive. Try locally expected message positions without exposing counters or identities, and preserve suspended history searches, skipped keys and recovery after failed saves.
- Replace a later history timer when fresh messages become eligible sooner. Keep duplicate traffic and retries within the same decryption budget.
- Avoid duplicate checkpoint payload reads while retaining raw type, length, ordering and hash validation. Reuse validated shared payloads during reconstruction.
- Save restored authorization projections once before publishing completed app state or acknowledging decrypted deliveries. Preserve durable recovery when a page read or save fails.
- Update the Rust ratchet library to 0.0.175, the protocol package to 0.3.10 and the CLI package to 0.1.53. Message formats and saved history remain compatible with v2026.10.25.2.

### Apple

- Reduce background processing while recovering older messages.
- Keep new messages moving while older history recovers.
- Open saved chats with less repeated database work.

### Zapstore

- Reduce background processing while recovering older messages.
- Keep new messages moving while older history recovers.
- Open saved chats with less repeated database work.

## v2026.10.25.2

### GitHub

- Reuse existing isolated discovery workers when the app returns to the foreground, including profile updates published while offline. Redeliver unchanged metadata when recovery explicitly requests it, without repeating cold fetches for covered discovery filters.
- Preserve separate author and message-server scopes, fresh anonymous authentication on new connections, and the existing cold-fetch path for other selectors.
- Add a foreground recovery regression that checks serialized requests and progress when one message server stalls.
- Require both one-server and optimized five-server populated discovery CPU gates, covering 2,400 author/server pairs including live-endpoint discovery, with 64 retained connections across the whole process.
- Reduce SQLite work when saving queued-message retry timers by updating validated existing rows directly. Preserve transaction, corruption and restart checks.
- Fix Ping/ACK handling in the public-advert test relay to make wire checks reliable.
- Keep the protocol source package at 0.3.9 and the CLI source package at 0.1.52. The message wire format and stored message state are unchanged from v2026.10.25.

### Apple

- Avoid repeated connection setup when returning to Iris.
- Reduce processing when saving messages waiting for delivery.
- Catch up on profile changes while keeping chats responsive when a message server is slow.

### Zapstore

- Avoid repeated connection setup when returning to Iris.
- Reduce processing when saving messages waiting for delivery.
- Catch up on profile changes while keeping chats responsive when a message server is slow.

## v2026.10.25.1

### GitHub

- Reuse existing isolated discovery workers when the app returns to the foreground, including profile updates published while offline. Redeliver unchanged metadata when recovery explicitly requests it, without repeating cold fetches for covered discovery filters.
- Preserve separate author and message-server scopes, fresh anonymous authentication on new connections, and the existing cold-fetch path for other selectors.
- Add a foreground recovery regression that checks serialized requests and progress when one message server stalls.
- Require both one-server and optimized five-server populated discovery CPU gates, covering 2,400 author/server pairs including live-endpoint discovery, with 64 retained connections across the whole process.
- Reduce SQLite work when saving queued-message retry timers by updating validated existing rows directly. Preserve transaction, corruption and restart checks.
- Update the protocol source package to 0.3.9 and the CLI source package to 0.1.52. The message wire format and stored message state are unchanged from v2026.10.25.

### Apple

- Avoid repeated connection setup when returning to Iris.
- Reduce processing when saving messages waiting for delivery.
- Catch up on profile changes while keeping chats responsive when a message server is slow.

### Zapstore

- Avoid repeated connection setup when returning to Iris.
- Reduce processing when saving messages waiting for delivery.
- Catch up on profile changes while keeping chats responsive when a message server is slow.

## v2026.10.25

### GitHub

- Reduce database reads when saving large pending-message backlogs. Maintain compact chunk metadata in the same transaction, preserving full-read validation and existing partial writes.
- Avoid copying the pending backlog and repeating saves when startup replays unchanged device authorizations. Preserve authorization reconciliation, conflicting signed evidence and rollback on failed saves.
- Schedule durable group-message publications after restart so recovery reoffers the complete saved batch until the application outbox commits it.
- Add regressions for physical database reads, authorization replay allocations, migration failures, corruption checks and older-client writes.
- Update the protocol source package to 0.3.8 and the CLI source package to 0.1.51. The message wire format is unchanged from v2026.10.24.1.

### Apple

- Open saved chats faster and reduce processing when many messages are waiting for delivery.
- Resume pending group-message delivery after reopening the app.

### Zapstore

- Open saved chats faster and reduce processing when many messages are waiting for delivery.
- Resume pending group-message delivery after reopening the app.

## v2026.10.24.1

### GitHub

- Reduce repeated TCP and TLS setup by reusing read-only discovery connections within one exact public-author, message-server and filter scope.
- Bound retained connections and handle idle server pings without opening extra queries. Preserve the current discovery schedule and first-contact capacity.
- Discard connections on configuration changes, cancellation and restart. Reconnect with fresh anonymous authentication; keep private publication on separate send-only connections.
- Add serialized request, authentication, TLS, restart, cancellation and resource-limit tests for discovery connection reuse.
- Update the CLI source package to 0.1.50. The protocol wire format and stored message state are unchanged from v2026.10.24.

### Apple

- Reduce idle processing and repeated connections to message servers.

### Zapstore

- Reduce idle processing and repeated connections to message servers.

## v2026.10.24

### GitHub

- Avoid repeating signature checks for unchanged public discovery records. Keep a bounded cache within each author's message-server configuration, recompute every event ID, and apply current filters on every response.
- Discard cached verification when discovery settings change. Preserve separate author scopes, fresh anonymous authentication, restart recovery and explicit redelivery.
- Add tampering, eviction, configuration-change and serialized request tests for retained discovery verification.
- Reuse validated peer keys while checking connections to reduce idle processing.
- Recover the saved checkpoint layout during cold-start restore to avoid rewriting unchanged queued payloads.
- Make iOS startup database work interruptible during suspension and retry saved-account restoration on foreground return, preserving committed state.
- Update the protocol source package to 0.3.7 and the CLI source package to 0.1.49.

### Apple

- Reduce repeated processing while checking for connection updates.
- Reduce disk writes while restoring saved chats.
- Safely pause loading saved chats in the background and retry when you return.

### Zapstore

- Reduce repeated processing while checking for connection updates.
- Reduce disk writes while restoring saved chats.

## v2026.10.23.1

### GitHub

- Avoid rewriting large saved checkpoints when they grow or queued messages are added or delivered. Store immutable queued payloads separately from retry metadata, committing every change with the ratchet state in one transaction.
- Reuse cached payloads while saving and retain existing accounts, pending messages and encryption state during the storage migration. Migrated checkpoints require this version or newer.
- Save incoming-message retry timers without copying or rewriting their encrypted payloads. Preserve pending work and encryption state after a failed save or restart.
- Reuse database update statements while saving queued-message metadata to avoid repeated SQL compilation.
- Compare queued-message descriptors directly from SQLite rows and borrow proposed metadata, avoiding copies while preserving corruption checks and transactional recovery.
- Recognize ready recovery work before scanning deferred queues, without advancing encryption state or changing retry deadlines.
- Remember the next group-delivery retry deadline after checking deferred work, and refresh it after queue changes or failed-save recovery.
- Skip publication preparation when group retries have no deliverable messages, preserving pending recipients, retry clocks and queue limits.
- Preserve unreadable or unsupported saved state and return an error instead of replacing it with a fresh session.
- Add physical SQLite write checks for growth, queue admission, successful retry retirement, unchanged saves and failed-commit recovery.
- Bind Apple upgrade acceptance to the exact official build and authorized installation route, with preserved-state, lifecycle, CPU, disk-write, startup and send checks.
- Update the protocol source package to 0.3.6 and the CLI source package to 0.1.48.

### Apple

- Reduce processing and disk writes when many messages are waiting to be delivered.
- Improve recovery of saved chats after restarting.

### Zapstore

- Reduce processing and disk writes when many messages are waiting to be delivered.
- Improve recovery of saved chats after restarting.

## v2026.10.23

### GitHub

- Bound retry bookkeeping to the pending deliveries being processed. Avoid copying deferred incoming messages, scanning the full active decryption queue and rebuilding the complete group delivery queue on every continuation.
- Cache the next incoming retry deadline, reduce temporary allocations during key derivation, and reuse fixed setup within each group-key search. Preserve derived keys, key-search limits, delivery order and failed-save recovery.
- Remember failed incoming decryption only while the exact message, receiving sessions and device authorization remain unchanged. Keep this cache in memory and retry after relevant state changes, preserving restart and delivery-journal recovery.
- Avoid saving the account checkpoint when a recipient has no known device list and only its retry timer changes. Preserve durable delivery progress, expiration, revocation and failed-save recovery.
- Avoid saving unchanged group-key repair work when only its retry timers advance. Keep new queued work, delivery progress and linked-device session changes durable.
- Compare saved checkpoints in larger blocks while preserving page-sized writes and transactional recovery.
- Require a restored-backlog CPU regression with validly encoded messages that fail authentication, real expiring controls, per-turn durable saves, preserved pending messages and restart checks.
- Update FIPS dependencies for automatic connections between devices on the same network while retaining anonymous message-provider publication and discovery.
- Add Apple startup lifecycle observations without account, contact or message data.
- Update the protocol source package to 0.3.5 and the CLI source package to 0.1.47.

### Apple

- Reduce processing when many messages are waiting to be delivered.
- Improve connections between devices on the same network.

### Zapstore

- Reduce processing when many messages are waiting to be delivered.
- Improve connections between devices on the same network.

## v2026.10.22.5

### GitHub

- Cache public message-server addresses briefly during discovery, with bounded storage and refresh after failed connections, loss of connectivity or returning to the foreground. Keep sockets, anonymous authentication and TLS sessions isolated by author and message server.
- Reuse unchanged queued group payloads when saving protocol state, preserving atomic writes and crash recovery. Avoid queuing repeated requests for the same missing group key while preserving existing pending work and distinct recovery requests.
- Update the protocol source package and registry CLI consumer to the corrected protocol implementation.
- Add `iris invite create --full` for durable invite links that do not require a file upload. Preserve account recovery and existing group-invite authorization.
- Save conversation acceptance after verifying an explicitly accepted direct invite, so the first incoming message can receive an encrypted delivery confirmation. Preserve later rejection and blocking decisions.
- Pin Windows binding generation to its reviewed commit, select the intended Windows SDK explicitly, and verify locked package versions and content hashes before building the app.
- Keep sign-in hidden while a saved account restores, including slow restores, and preserve background storage protection.
- Stop republishing unchanged connection adverts every two seconds; retain immediate endpoint updates and refresh before expiration.
- Reuse TLS 1.3 sessions only within an unchanged, single-author discovery scope and exact message-server URL, with fresh key exchange and anonymous authentication on every connection.
- Keep nearby-device discovery on its quiet schedule when accepted connection adverts are renewed or replayed, while preserving signed expiry and cache recovery.
- Preserve the visible message when an older page finishes loading just after a held drag ends.
- Show the hand cursor over clickable Mac controls and message links while preserving text selection and image zoom.
- Add Haps source-install recipes for the desktop app and CLI, with checks that keep package metadata and build versions aligned with the tag. Include a Start Menu shortcut and icon for Windows Haps installs. Locally built packages remain distinct from signed release binaries.

### Apple

- Reduce repeated network work while Iris is open.
- Reduce processing when many group deliveries are pending.
- Show delivery confirmations after accepting a chat invitation.
- Prevent a brief sign-in screen while your saved account loads.
- Keep your place when loading older messages on iPhone.
- Show the hand cursor on clickable controls and message links on Mac.

### Zapstore

- Reduce repeated network work while Iris is open.
- Reduce processing when many group deliveries are pending.
- Show delivery confirmations after accepting a chat invitation.
- Reduce repeated network startup while restoring your account.

## v2026.10.22.4

### GitHub

- Keep sign-in hidden while a saved account restores, including slow restores, and preserve background storage protection.
- Stop republishing unchanged connection adverts every two seconds; retain immediate endpoint updates and refresh before expiration.
- Reuse TLS 1.3 sessions only within an unchanged, single-author discovery scope and exact message-server URL, with fresh key exchange and anonymous authentication on every connection.
- Keep nearby-device discovery on its quiet schedule when accepted connection adverts are renewed or replayed, while preserving signed expiry and cache recovery.
- Measure recurring discovery work across two complete polling windows, with serialized checks for separate identities and fresh anonymous authentication.
- Check aggregate discovery limits independently of network deadlines, retain real-wire scope checks, and bound failure diagnostics.
- Preserve the visible message when an older page finishes loading just after a held drag ends.
- Show the hand cursor over clickable Mac controls and message links while preserving text selection and image zoom.
- Add Haps source-install recipes for the desktop app and CLI, with checks that keep package metadata and build versions aligned with the tag. Locally built packages remain distinct from signed release binaries.

### Apple

- Prevent a brief sign-in screen while your saved account loads.
- Reduce repeated connection work while Iris is open.
- Keep your place when loading older messages on iPhone.
- Show the hand cursor on clickable controls and message links on Mac.

### Zapstore

- Reduce repeated network startup while restoring your account.
- Reduce repeated connection work while Iris is open.

## v2026.10.22.3

### GitHub

- Add Haps source-install recipes for the desktop app and CLI, with release checks that keep package metadata and build versions aligned with the tag. Locally built packages remain distinct from the signed release binaries.
- Stop a changing expiry timestamp from republishing unchanged connection adverts every two seconds; retain immediate endpoint updates and refresh before expiration.
- Reuse TLS 1.3 sessions only within an unchanged, single-author discovery scope and exact message-server URL, with fresh key exchange and anonymous authentication on every connection. Keep older TLS connections fully validated without session reuse.
- Keep nearby-device discovery on its quiet schedule when accepted connection adverts are renewed or replayed, while preserving signed expiry and cache recovery.
- Measure recurring TLS discovery work across two complete polling windows, with serialized checks for separate identities and fresh anonymous authentication.
- Preserve the visible message when an older page finishes loading just after a held drag ends.

### Apple

- Reduce repeated connection work while Iris is open.
- Keep your place when loading older messages on iPhone.

### Zapstore

- Reduce repeated connection work while Iris is open.

## v2026.10.22.2

### GitHub

- Keep sign-in hidden while a saved account is still restoring, including restores longer than eight seconds. Preserve genuine restore errors and background storage protection.
- Fade in the loading label after two seconds, respecting reduced-motion settings.
- Reduce repeated discovery requests for unchanged contacts and devices, while refreshing promptly when an invite or live connection needs them. Preserve separate anonymous lookup and publication connections.
- Check sustained discovery work with a large synthetic database containing multiple devices for the account and its contacts.
- Show the hand cursor over clickable Mac controls and message links, preserving text selection, link handling and image zoom cursors.
- Create the release test output directory before running fixtures, so a clean runner does not depend on test execution order.

### Apple

- Prevent a brief sign-in screen while your saved account loads.
- Gently show the loading label only when startup takes more than two seconds.
- Reduce repeated network requests while Iris is open.
- Show the hand cursor on clickable controls and message links on Mac.

### Zapstore

- Reduce repeated network startup while restoring your account.
- Reduce repeated network requests while Iris is open.

## v2026.10.22.1

### GitHub

- Keep sign-in hidden while a saved account is still restoring, including restores longer than eight seconds. Preserve genuine restore errors and background storage protection.
- Fade in the loading label after two seconds, respecting reduced-motion settings.
- Add slow-restore transition checks on iOS and macOS, including background storage acknowledgement and macOS screen captures.
- Reduce repeated discovery requests for unchanged contacts and devices, while refreshing promptly when an invite or live connection needs them. Preserve separate anonymous lookup and publication connections.
- Check sustained discovery work with a large synthetic database containing multiple devices for the account and its contacts.
- Show the hand cursor over clickable Mac controls and message links, preserving text selection, link handling and image zoom cursors.

### Apple

- Prevent a brief sign-in screen while your saved account loads.
- Gently show the loading label only when startup takes more than two seconds.
- Reduce repeated network requests while Iris is open.
- Show the hand cursor on clickable controls and message links on Mac.

### Zapstore

- Reduce repeated network startup while restoring your account.
- Reduce repeated network requests while Iris is open.

## v2026.10.22

### GitHub

- Protect iOS background startup through account restoration and a storage suspension acknowledgement, including delayed restoration after an earlier suspension.
- Reconcile FIPS once after restoring saved authorizations, preserving the complete device roster without opening attachment storage for every intermediate authorization.
- Require exact TestFlight evidence for both background launch and backgrounding during account restoration before public beta or App Store promotion.

### Apple

- Fix a crash when Iris starts or restores your account in the background.

### Zapstore

- Reduce repeated network startup while restoring your account.

## v2026.10.21.2

### GitHub

- Fix the startup allocation failure in anonymous peer discovery. Bound concurrent reads and connection buffers without allocating a full relay-client cache for every contact or sharing authentication between contacts.
- Add populated-account discovery regressions under a process memory limit, alongside serialized authentication and query privacy checks.
- Require attested physical upgrade and lifecycle evidence for the exact TestFlight build before public beta or App Store promotion.
- Update TLS certificate verification and trusted roots to include the latest trust-store security fixes.

### Apple

- Fix a startup crash caused by high memory use.

### Zapstore

- Fix a startup crash caused by high memory use.

## v2026.10.21.1

### GitHub

- Publish private messages and call wake-ups on separate send-only connections, using fresh anonymous relay authentication and no account-authenticated fallback. Keep peer discovery separate from the account's receiving connection.
- Isolate FIPS message-provider publishing and each peer lookup from live device identities. Deliver private nearby traffic only to its intended recipient, and keep unsent copies queued when a provider has not accepted them.
- Request client mode for temporary message-provider and update-reader connections to avoid unnecessary routing work.
- Restore nearby Bluetooth delivery when a phone changes accounts and reuses its Bluetooth address. Accept traffic only from the current authenticated connection.
- Use private per-session responder keys so public invitations cannot identify the sender of encrypted replies.
- Sign call wakeups with fresh anonymous keys and authenticate callers only after decrypting the double-ratchet offer. Remove caller identity lists from notification subscriptions.
- Deliver every group message and membership update through separate double-ratchet envelopes for each recipient. Remove public group rosters, shared group broadcasts and shared-key subscriptions from all network carriers.
- Stop publishing private-group member lists, administrators and names. Previously published records cannot be made private retroactively.
- Require the new private messaging protocol, retire old sessions and invitations, and retain old unsent ciphertext locally without republishing it. Keep chat history.
- Add serialized-event and upgrade regressions for identity correlation, first replies, calls, notification subscriptions and queued-message replay.
- Keep Windows text, controls and attachments readable in light and dark themes, and follow Windows theme changes while the app is open.
- Give Windows settings a full-window layout with keyboard navigation, compact forms and switches. Preserve profile edits when switching settings pages.
- Preserve the ongoing drag position on iPhone when the rendered history window changes.
- Retry queued group keys as soon as group information arrives, including through linked-device sync.
- Scope shared-mesh release-test subscriptions to public file metadata while preserving receiver privacy rules and resource checks.

### Apple

- Improve privacy for messages, invitations and incoming calls.
- Keep caller approval and blocking on your device.
- Improve nearby messaging after changing accounts.
- Keep your place when scrolling through long chats on iPhone.
- Improve group message delivery after linking devices.
- Update both sides of a conversation to keep messaging and calling. Existing chat history is preserved; old invitation links need to be replaced.

### Zapstore

- Improve privacy for messages, invitations and incoming calls.
- Keep caller approval and blocking on your device.
- Improve nearby messaging after changing accounts.
- Improve group message delivery after linking devices.
- Update both sides of a conversation to keep messaging and calling. Existing chat history is preserved; old invitation links need to be replaced.

## v2026.10.21

### GitHub

- Publish private messages and call wake-ups on separate send-only connections, using fresh anonymous relay authentication and no account-authenticated fallback. Keep peer discovery separate from the account's receiving connection.
- Isolate FIPS message-provider publishing and each peer lookup from live device identities. Deliver private nearby traffic only to its intended recipient, and keep unsent copies queued when a provider has not accepted them.
- Request client mode for temporary message-provider and update-reader connections to avoid unnecessary routing work.
- Restore nearby Bluetooth delivery when a phone changes accounts and reuses its Bluetooth address. Accept traffic only from the current authenticated connection.
- Use private per-session responder keys so public invitations cannot identify the sender of encrypted replies.
- Sign call wakeups with fresh anonymous keys and authenticate callers only after decrypting the double-ratchet offer. Remove caller identity lists from notification subscriptions.
- Deliver every group message and membership update through separate double-ratchet envelopes for each recipient. Remove public group rosters, shared group broadcasts and shared-key subscriptions from all network carriers.
- Stop publishing private-group member lists, administrators and names. Previously published records cannot be made private retroactively.
- Require the new private messaging protocol, retire old sessions and invitations, and retain old unsent ciphertext locally without republishing it. Keep chat history.
- Add serialized-event and upgrade regressions for identity correlation, first replies, calls, notification subscriptions and queued-message replay.
- Keep Windows text, controls and attachments readable in light and dark themes, and follow Windows theme changes while the app is open.
- Give Windows settings a full-window layout with keyboard navigation, compact forms and switches. Preserve profile edits when switching settings pages.
- Preserve the ongoing drag position on iPhone when the rendered history window changes.
- Retry queued group keys as soon as group information arrives, including through linked-device sync.

### Apple

- Improve privacy for messages, invitations and incoming calls.
- Keep caller approval and blocking on your device.
- Improve nearby messaging after changing accounts.
- Keep your place when scrolling through long chats on iPhone.
- Improve group message delivery after linking devices.
- Update both sides of a conversation to keep messaging and calling. Existing chat history is preserved; old invitation links need to be replaced.

### Zapstore

- Improve privacy for messages, invitations and incoming calls.
- Keep caller approval and blocking on your device.
- Improve nearby messaging after changing accounts.
- Improve group message delivery after linking devices.
- Update both sides of a conversation to keep messaging and calling. Existing chat history is preserved; old invitation links need to be replaced.

## v2026.10.20

### GitHub

- Keep chat-list rows and following conversations in place when previews change between empty, short and wrapped text, drafts or typing indicators across the native apps.
- Resolve identity-backed avatars and names through the shared core profile directory on iOS, macOS, Android, Windows and Linux. Refresh profile changes without relying on stale row metadata.
- Keep mention suggestions tied to the matching person as the query changes and display their current avatar and name.
- Reuse verified core build inputs for focused Apple interface tests, while retaining the source and cache provenance of the original build.

### Apple

- Keep the chat list steady when message previews wrap or change.
- Show the right person, name and avatar in mention suggestions.
- Refresh people's names and photos consistently throughout the app.

### Zapstore

- Keep the chat list steady when message previews wrap or change.
- Show current names and avatars in mention suggestions.
- Refresh people's names and photos consistently throughout the app.

## v2026.10.19

### GitHub

- Disable colon emoji completion on iOS and Android; retain it on desktop. Mobile web follows the same behavior.
- Float mention suggestions above the composer without moving the editor or conversation. Keep suggestions scrollable, readable, and selectable by touch, mouse, or keyboard as appropriate.
- Show readable mention names in desktop draft previews while preserving the selected person's identity.
- Release iOS storage transactions before suspension to address the reported background termination, and resume with the draft intact.
- Update compatible networking dependencies, including FIPS 0.4.106 with connection retry fixes and the refreshed WebRTC dependency set.
- Reuse unchanged validation products and focused test coverage, and cache Linux release build inputs to reduce repeated compilation.
- Preserve group messages in web storage before showing them as sent; keep the draft available if saving fails.

### Apple

- Keep mention suggestions above your message without shifting the conversation.
- Use the iPhone keyboard for emoji; typing a colon now stays as text.
- Show people's names in Mac draft previews when you mention them.
- Fix a crash when the iPhone app moves into the background and keep your draft when you return.
- Improve connection retry handling and update networking dependencies.

### Zapstore

- Keep mention suggestions above your message without shifting the conversation.
- Use your keyboard for emoji; typing a colon now stays as text.
- Keep mention suggestions readable and easy to select.
- Improve connection retry handling and update networking dependencies.

## v2026.10.18

### GitHub

- Add group mention menus opened by `@` and emoji completion opened by `:` across the native apps. Preserve the selected person's identity through edits, deletion, and undo, including people with the same display name.
- Allow mentions to notify in muted groups, with a separate preference synced privately between authorized devices through Negentropy. Keep quiet preferences when concurrent changes have the same timestamp.
- Clear desktop notifications when their exact messages are read on another device, while retaining notifications for messages that remain unread. Preserve authenticated message identities through group-message recovery.
- Replace valid attachment hashes in notifications and chat-list previews with readable attachment labels while preserving captions and message bodies.
- Preserve new drafts and navigation when an earlier attachment upload finishes. Improve link cursors and link contrast in message bubbles.
- Create and manage encrypted private personal and group invitations, including expiry, use limits and revocation. Preserve durable admission decisions across retries and restarts.
- Sync chat acceptance through private request-decision events, including authenticated replies and reactions from linked devices, while preserving newer rejection and blocking decisions.
- Keep encrypted attachments in a bounded local store so a temporary upload-server outage does not prevent sending. Retry replication in the background and serve available ciphertext to peers.
- Recover recent accepted group messages from a restarted nearby member using their original signed, encrypted envelopes.
- Show confirmed encrypted upload URLs in message details, with full clickable links and copying.
- Expose mention-notification settings in the CLI and deliver complete service errors even when they exceed one socket write.
- Recover device sync after linked browsers reload or connect simultaneously, preserving queued records and keeping one consistent connection direction.
- Keep Linux chat-list and composer shortcuts working during navigation and pane animations.
- Preserve complete microphone batches through the Apple call sender while keeping audio queues bounded and honoring mute, call-end and account changes.
- Apply the intended microphone mute state before starting or restarting Apple call audio.
- Keep desktop Nearby rows and the following chat list in place as peers appear and disappear.
- Retain the 64 MiB limit for ordinary native attachments; use Send directly for larger one-to-one transfers, with a 100 GiB per-file limit.
- Add image zoom and bounded panning on Mac.
- Show Mac focus outlines during keyboard navigation while keeping mouse interactions clear.
- Update sync, TLS and QUIC dependencies to fix published security advisories.

### Apple

- Type `@` to mention someone in a group, or `:` to find an emoji.
- Choose whether mentions notify you in a muted group, and keep that choice in sync across devices.
- Keep new text you write while an attachment is uploading.
- Show readable attachment labels in notifications and the chat list.
- Create and open private invitations to people and groups.
- Keep accepted chats in sync across your devices.
- Recover recent group messages from a nearby member after a restart.
- Send attachments when the upload server is unavailable, and view encrypted upload links in message details.
- Improve message editing and notification cleanup across linked devices.
- Keep chat settings in sync when linked browsers reconnect.
- Reduce dropped audio during calls and preserve your mute setting when audio starts or resumes.
- Keep the Mac chat list steady as Nearby people appear and disappear.
- Zoom in on images and move around them on Mac.
- Keep Mac controls clear of focus outlines when using a mouse.
- Improve security when syncing and connecting to message servers.

### Zapstore

- Type `@` to mention someone in a group, or `:` to find an emoji.
- Choose whether mentions notify you in a muted group, and keep that choice in sync across devices.
- Keep new text you write while an attachment is uploading.
- Show readable attachment labels in notifications and the chat list.
- Create and open private invitations to people and groups.
- Keep accepted chats in sync across your devices.
- Recover recent group messages from a nearby member after a restart.
- Send attachments when the upload server is unavailable, and view encrypted upload links in message details.
- Improve message editing and notification cleanup across linked devices.
- Keep chat settings in sync when linked browsers reconnect.
- Improve security when syncing and connecting to message servers.

## v2026.10.15.1

### GitHub

- Use ordinary unsigned Nostr events for private contact facts, blocks, and name approvals, admitted only through authenticated own-device channels.
- Reconcile pins, timed mutes, device names, chat deletions, and message-request decisions through Negentropy. Preserve original signatures on device and group authorization proofs.
- Merge chat deletions independently, keep the latest deletion cutoff for each chat, and retain newer messages after offline reconciliation.
- Align web and native private sync with negotiated capabilities, durable restart recovery, and protection against stale device lists removing an authorized device.
- Use the approver's history window during device linking so chosen older messages are transferred.
- Retry temporary Apple ticket-service failures while stapling and validating the signed Mac app and installer.

### Apple

- Keep contact names, notes, favorites, blocks, and chat settings in sync across your devices.
- Recover changes made while a device was offline, including pinned chats, muted chats, and deleted conversations.
- Improve message-history transfer when linking a device and reliability after restarting the app.

### Zapstore

- Keep contact names, notes, favorites, blocks, and chat settings in sync across your devices.
- Recover changes made while a device was offline, including pinned chats, muted chats, and deleted conversations.
- Improve message-history transfer when linking a device and reliability after restarting the app.

## v2026.10.15

### GitHub

- Use ordinary unsigned Nostr events for private contact facts, blocks, and name approvals, admitted only through authenticated own-device channels.
- Reconcile pins, timed mutes, device names, chat deletions, and message-request decisions through Negentropy. Preserve original signatures on device and group authorization proofs.
- Merge chat deletions independently, keep the latest deletion cutoff for each chat, and retain newer messages after offline reconciliation.
- Align web and native private sync with negotiated capabilities, durable restart recovery, and protection against stale device lists removing an authorized device.
- Use the approver's history window during device linking so chosen older messages are transferred.

### Apple

- Keep contact names, notes, favorites, blocks, and chat settings in sync across your devices.
- Recover changes made while a device was offline, including pinned chats, muted chats, and deleted conversations.
- Improve message-history transfer when linking a device and reliability after restarting the app.

### Zapstore

- Keep contact names, notes, favorites, blocks, and chat settings in sync across your devices.
- Recover changes made while a device was offline, including pinned chats, muted chats, and deleted conversations.
- Improve message-history transfer when linking a device and reliability after restarting the app.

## v2026.10.13

### GitHub

- Retain the iOS CoreBluetooth bridge in the background, restore central/peripheral state, and bound callback processing with storage flushes before releasing background time.
- Sync signed contact-name approvals privately between authorized native devices, including offline recovery through Negentropy and deterministic resolution of concurrent approvals.
- Sync exact seen-message IDs between linked devices without marking unseen messages as read through timestamp cutoffs.
- Preserve direct conversations and history when blocking someone while stopping new messages and calls.
- Request Android notification permission after account setup and allow account creation when restored app data has lost its secret key.

### Apple

- Keep nearby Bluetooth connections available when switching apps, within iOS background limits.
- Keep approved contact names and seen messages in sync between linked native apps.
- Keep your conversation history when blocking someone.

### Zapstore

- Keep approved contact names and seen messages in sync between linked native apps.
- Keep your conversation history when blocking someone.
- Ask for notification permission after account setup.
- Recover account setup when restored app data no longer has its secret key.

## v2026.10.9.1

### GitHub

- Preserve unread notifications when opening the app on iOS and macOS. Clear notifications using message read state, including reads synced from linked devices.
- Enable typing indicators and received/seen receipts by default for new settings while preserving existing preferences. Deliver stopped-typing updates instead of discarding them as expired messages.
- Install already verified desktop updates even when discovery becomes unavailable. Show installation failures with a retry option and prevent background checks from interrupting installation.
- Include iOS builds with the latest private block sync and blocked-people settings.

### Apple

- Keep unread notifications when opening the app, and clear them as messages are read on your devices.
- Turn on typing indicators and received/seen status by default for new installs. Existing settings stay unchanged.
- Clear typing indicators promptly when someone stops typing.
- Manage blocked people in Settings and keep blocks in sync with your devices.
- Improve desktop update installation and show a retry option if installation fails.

### Zapstore

- Turn on typing indicators and received/seen status by default for new installs. Existing settings stay unchanged.
- Clear typing indicators promptly when someone stops typing.

## v2026.10.9

### GitHub

- Sync signed block and unblock events privately between currently authorized devices on the same account. Recheck device membership before importing or exporting private sync data.
- Delete direct conversations when blocking someone, stop queued sends across message servers and nearby connections, and keep blocked-period messages from returning after sync or restart. Preserve conversations reopened after unblocking.
- Keep existing shared-group history visible by default, with an option to hide past messages from blocked people. Apply visibility consistently to search, older-message pages, previews, and cached views.
- Add blocked-people settings with Unblock controls across native apps, and suppress new blocked messages and notification previews while preserving shared-group membership controls.

### Apple

- Manage blocked people in Settings and choose whether to hide their past group messages.
- Keep blocks in sync with your devices and remove direct chats when blocking.
- Stop queued messages and unwanted notifications from returning after syncing or restarting.

### Zapstore

- Manage blocked people in Settings and choose whether to hide their past group messages.
- Keep blocks in sync with your devices and remove direct chats when blocking.
- Stop queued messages and unwanted notifications from returning after syncing or restarting.

## v2026.10.8.3

### GitHub

- Find signed updates through either connected peers or the app's message servers. An unavailable source no longer prevents another source from finding the update.
- Keep signature, fresh-observation, and rollback checks intact across sources. Use the same message-server defaults for standalone update checks.
- Show concise update errors on Mac and clear previously saved technical error dumps. Keep full diagnostics in support logs and preserve an already verified update after a failed refresh.

### Apple

- Find updates even when a peer connection is unavailable.
- Show clearer update errors and keep verified updates available after a failed check.

### Zapstore

- Improve signed update discovery when a network source is unavailable.

## v2026.10.8.2

### GitHub

- Give Linux a responsive desktop chat layout, keeping the chat list visible in wide windows and adapting to smaller screens. Harmonize headers, round controls, message previews, reactions, and hover actions.
- Keep Linux typing and search responsive during background updates, show linked-device message sync progress, and allow informational popups to close when clicking outside.
- Restore encrypted image attachments on Linux. Add visible text-size controls and persistent image-proxy settings, and move disappearing-message options into chat details.
- Make edit history easy to find in message menus and Message Details across native apps. Show the latest revision first, follow incoming edits, preserve selected text on Linux, and close history when its message or account is no longer available.
- Avoid repeated address allocations while comparing ICE candidates.
- Preserve existing Hashtree release history during publication and publish signed Haps announcements directly to public message servers.

### Apple

- Find edit history from the message menu or Message Details on Mac.
- Read the latest version first and follow new edits while history is open.

### Zapstore

- Find edit history from the message menu or Message Details.
- Read the latest version first and follow new edits while history is open.
- Close history when a message is deleted or expires, or when leaving the chat.

## v2026.10.8.1

### GitHub

- Run strict Rust lint over test targets before tagging and correct a redundant test reference caught by the release gate.

- Suppress non-message iOS pushes using Apple-approved notification filtering, including decryption failures and extension timeouts. Verify the entitlement in signed release binaries.
- Keep iOS background handling alive until notification processing and persistence finish, and coalesce private-contact recovery after message-server reconnects.
- Receive Android messages and calls without Google push through the existing encrypted connections. Preserve saved accounts and nearby discovery when Android restarts the receiver.
- Dismiss ended incoming-call screens and return background receivers to the background. Pause hidden Nearby polling, reduce idle direct-file listener wakeups, and avoid full chat snapshots for hidden network diagnostics.
- Keep Apple startup loading visible until saved-session restoration settles. Present iPhone debug exports directly in the system share sheet.
- Refresh linked-device history when foregrounding, including interrupted reconciliation, so missed reactions catch up promptly.
- Keep queued nearby messages available for their intended recipient when another peer acknowledges them. Suppress retries only to the acknowledging peer while preserving queue limits.
- Stop rejected device approvals promptly and preserve safe, specific failure reasons across the phone and browser.
- Update FIPS core to 0.4.95, TCP to 0.2.5, TCP endpoint to 0.2.30, pubsub FIPS to 0.5.22, and Hashtree FIPS transport to 0.4.28. Bound idle control work and failed-service retries, retain authenticated peer identities, and correct TCP acknowledgment recovery and retry limits.
- Stop redundant ICE checks for candidate pairs that have already connected, while preserving consent checks and new connection validation.
- Keep new direct connections visible while their setup is starting so an early status check does not discard them. Cancel closed attempts during setup so they cannot recreate background connections.
- Batch queued Android UDP packets without copying payloads or delaying sends, and keep packet accounting correct when a send fails.
- Reuse network output buffers during empty completion turns.
- Activate public TestFlight builds after approval and verify external testing status. Publish signed desktop and command-line packages through Haps using the same attested release binaries.

### Apple

- Filter unnecessary notifications on iPhone.
- Improve background message handling and recovery after connection interruptions.
- Keep startup smooth while restoring your account.
- Improve device linking and message-history transfer, including accounts used with older app versions.
- Keep sign-in codes available and show clearer linking errors without waiting for a timeout.
- Catch up on missed reactions sooner when reopening the app.
- Fix missed messages when a nearby device connects later.
- Share iPhone debug information directly through the system share sheet.

### Zapstore

- Receive messages and calls in the background on Android devices without Google Play services.
- Keep nearby delivery working after Android restarts the app.
- Return to the background when an incoming call ends.
- Improve device linking and show clearer errors.
- Catch up on missed reactions sooner when reopening the app.
- Fix missed messages when a nearby device connects later.


## v2026.10.8

### GitHub

- Suppress non-message iOS pushes using Apple-approved notification filtering, including decryption failures and extension timeouts. Verify the entitlement in signed release binaries.
- Keep iOS background handling alive until notification processing and persistence finish, and coalesce private-contact recovery after message-server reconnects.
- Receive Android messages and calls without Google push through the existing encrypted connections. Preserve saved accounts and nearby discovery when Android restarts the receiver.
- Dismiss ended incoming-call screens and return background receivers to the background. Pause hidden Nearby polling, reduce idle direct-file listener wakeups, and avoid full chat snapshots for hidden network diagnostics.
- Keep Apple startup loading visible until saved-session restoration settles. Present iPhone debug exports directly in the system share sheet.
- Refresh linked-device history when foregrounding, including interrupted reconciliation, so missed reactions catch up promptly.
- Keep queued nearby messages available for their intended recipient when another peer acknowledges them. Suppress retries only to the acknowledging peer while preserving queue limits.
- Stop rejected device approvals promptly and preserve safe, specific failure reasons across the phone and browser.
- Update FIPS core to 0.4.95, TCP to 0.2.5, TCP endpoint to 0.2.30, pubsub FIPS to 0.5.22, and Hashtree FIPS transport to 0.4.28. Bound idle control work and failed-service retries, retain authenticated peer identities, and correct TCP acknowledgment recovery and retry limits.
- Stop redundant ICE checks for candidate pairs that have already connected, while preserving consent checks and new connection validation.
- Keep new direct connections visible while their setup is starting so an early status check does not discard them. Cancel closed attempts during setup so they cannot recreate background connections.
- Batch queued Android UDP packets without copying payloads or delaying sends, and keep packet accounting correct when a send fails.
- Reuse network output buffers during empty completion turns.
- Activate public TestFlight builds after approval and verify external testing status. Publish signed desktop and command-line packages through Haps using the same attested release binaries.

### Apple

- Filter unnecessary notifications on iPhone.
- Improve background message handling and recovery after connection interruptions.
- Keep startup smooth while restoring your account.
- Improve device linking and message-history transfer, including accounts used with older app versions.
- Keep sign-in codes available and show clearer linking errors without waiting for a timeout.
- Catch up on missed reactions sooner when reopening the app.
- Fix missed messages when a nearby device connects later.
- Share iPhone debug information directly through the system share sheet.

### Zapstore

- Receive messages and calls in the background on Android devices without Google Play services.
- Keep nearby delivery working after Android restarts the app.
- Return to the background when an incoming call ends.
- Improve device linking and show clearer errors.
- Catch up on missed reactions sooner when reopening the app.
- Fix missed messages when a nearby device connects later.

## v2026.10.7

### GitHub

- Fix device approval for existing accounts with equivalent signed device lists that retain older encrypted device labels. Repair preserves device authorization and removes retired labels from the replacement publication.
- Preserve recent device-link diagnostics during background receipt traffic and distinguish failed approvals from timeouts.
- Cover mixed legacy device lists, unchanged membership, conflicting authorizations, and approval with message history in regression tests.

### Apple

- Fix device linking for accounts used with older app versions.
- Improve diagnostics when device linking fails.

### Zapstore

- Fix device linking for accounts used with older app versions.
- Improve diagnostics when device linking fails.

## v2026.10.6.1

### GitHub

- Continue device approval after the first message-server acknowledgement, without waiting for an unresponsive server. Report completion only after the authorized device list is observed.
- Start native history transfer immediately after verified local device approval, without waiting for the phone to receive its own device-list event.
- Transfer linked-device history through routed FIPS connections, and refresh history authorization while preserving the mobile transport. Keep unused sign-in codes available until canceled.
- Make device-link errors visible in iOS settings. Add routed native/browser regression tests and a physical iPhone public-network release gate that checks history after reload.
- Accelerate desktop video color conversion on ARM with NEON while preserving encoded output.
- Update Hashtree Core to 0.2.91 for accelerated, verified hashing.

### Apple

- Improve device linking and message-history transfer.
- Keep sign-in codes available while you prepare your other device.
- Show device-link errors in settings.
- Improve video-call performance on Apple silicon Macs.

### Zapstore

- Improve device linking and message-history transfer.
- Keep sign-in codes available while you prepare your other device.

## v2026.10.6

### GitHub

- Fix device linking when a message server is unavailable, including approval with message history. Keep verified conflicting and newer device lists visible, and reject incomplete empty discovery.
- Show linking progress on Apple devices. Make command-line linking wait for the new device to appear and support including message history.
- Cover server outages during linking and conflicting device lists from interrupted responses with regression tests.

### Apple

- Fix linking another device when a message server is unavailable.
- Show progress while linking a device, including when sharing message history.

### Zapstore

- Fix linking another device when a message server is unavailable, including when sharing message history.

## v2026.10.5.2

### GitHub

- Show a pointing-hand cursor on clickable desktop avatars, attachment previews, and other controls.
- Show unsent drafts in the Mac and Windows chat lists. Save Windows drafts when switching chats or closing the window, keep message edits separate, and clear drafts after sending.
- Keep the Mac chat list steady during Tab navigation, scrolling only enough to reveal the focused chat.
- Save command-line sign-in data with an atomic file replacement so an interrupted update cannot truncate the existing file.

### Apple

- Show unsent drafts in the Mac chat list.
- Keep the Mac chat list steady when navigating with Tab.
- Show a pointing-hand cursor on clickable Mac avatars, attachment previews, and other controls.

### Zapstore

- Desktop interface improvements.

## v2026.10.5.1

### GitHub

- Keep release source downloads working when a message server is unavailable.
- Show current group sender profile pictures, including photo removal, without changing message snapshots. Align group avatars and both desktop action docks with the bubble while reserving reaction space.
- Preserve iOS history position using stable content geometry during rendering-window transitions.
- Recover closed or lagged mesh subscriptions during background liveness checks, drain admitted events before replacement, and pin the verified public delivery stack.
- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Compare large protocol checkpoints in fixed-size pages without allocating another copy of the stored checkpoint; preserve transactional rollback on read, write, or close failures.
- Keep signed update roots when discovery is interrupted before a complete observation.
- Measure chat opening and sending through the existing Debug logging setting, with timing records limited to durations and message counts.
- Update iris-chat-protocol to 0.3.1, hashtree-updater to 0.2.88, nostr-pubsub to 0.1.16, nostr-pubsub-fips to 0.5.19, and nostr-pubsub-relay to 0.1.13.

### Apple

- Show profile pictures beside group messages and keep avatars and message actions aligned when reactions appear.
- Keep your place while scrolling through older messages on iPhone.
- Improve message delivery after a connection interruption.
- Reduce memory used while saving large message histories and improve update checks.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Improve message delivery after a connection interruption.
- Reduce memory used while saving large message histories and improve update checks.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.5

### GitHub

- Show current group sender profile pictures, including photo removal, without changing message snapshots. Align group avatars and both desktop action docks with the bubble while reserving reaction space.
- Preserve iOS history position using stable content geometry during rendering-window transitions.
- Recover closed or lagged mesh subscriptions during background liveness checks, drain admitted events before replacement, and pin the verified public delivery stack.
- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Compare large protocol checkpoints in fixed-size pages without allocating another copy of the stored checkpoint; preserve transactional rollback on read, write, or close failures.
- Keep signed update roots when discovery is interrupted before a complete observation.
- Measure chat opening and sending through the existing Debug logging setting, with timing records limited to durations and message counts.
- Update iris-chat-protocol to 0.3.1, hashtree-updater to 0.2.88, nostr-pubsub to 0.1.16, nostr-pubsub-fips to 0.5.19, and nostr-pubsub-relay to 0.1.13.

### Apple

- Show profile pictures beside group messages and keep avatars and message actions aligned when reactions appear.
- Keep your place while scrolling through older messages on iPhone.
- Improve message delivery after a connection interruption.
- Reduce memory used while saving large message histories and improve update checks.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Improve message delivery after a connection interruption.
- Reduce memory used while saving large message histories and improve update checks.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.4.11

### GitHub

- Show current group sender profile pictures, including photo removal, without changing message snapshots. Align group avatars and both desktop action docks with the bubble while reserving reaction space.
- Preserve iOS history position using stable content geometry during rendering-window transitions.
- Recover closed or lagged mesh subscriptions during background liveness checks, drain admitted events before replacement, and pin the verified public delivery stack.
- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Show profile pictures beside group messages and keep avatars and message actions aligned when reactions appear.
- Keep your place while scrolling through older messages on iPhone.
- Improve message delivery after a connection interruption.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Improve message delivery after a connection interruption.
- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.4.10

### GitHub

- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.4.9

### GitHub

- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.4.8

### GitHub

- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.



## v2026.10.4.7

### GitHub

- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.


## v2026.10.4.6

### GitHub

- Add message editing, edit history, deletion for this device, and deletion for everyone across native platforms. Authenticate changes against the original author and retain deletion tombstones during reconciliation.
- Keep linked-device edits and deletions on the private history-aware reconciliation path. Preserve the chats-and-groups-only choice through live delivery, queued messages, interrupted transfers, and failed database writes.
- Bound group decryption work per turn, preserve newly arriving sender-key wakeups, and retire revoked search results without leaving pending work stuck.
- Read recipient device lists without copying unrelated encrypted sessions when sending and retrying messages.
- Remove the eight-person member-picker cutoff. Scroll through all eligible contacts, refresh after each addition, and preserve current group permissions across Apple, Android, Linux, and Windows.
- Keep People search scoped to recognized contacts and preserve current results during refresh. Restore favorite badges on additional avatar surfaces.
- Match group avatar placeholder colors to sender names, anchor desktop message actions beside their bubble, and align Mac composer controls.
- Confirm group administrator promotions, restore Mac composer focus after window attachment, and compare locally installed debug versions correctly when checking for updates.
- Update iris-chat-protocol to 0.3.0 and nostr-pubsub-relay to 0.1.13. The protocol library's batch completion now requires a mutable engine.

### Apple

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results and favorite indicators.
- Fix message-action placement, group avatar colors, and Mac message-bar alignment and focus.

### Zapstore

- Edit your messages, view their edit history, and delete messages for yourself or everyone.
- Respect your history choice when linking another device.
- Keep chats responsive while catching up on group messages.
- Scroll through everyone you can add to a group, with the list updating after each addition.
- Improve search results, favorite indicators, and group avatar colors.

## v2026.10.4.5

### GitHub

- Restore explicit startup announcements for an existing device identity without resetting queued message retries or repeating database writes.
- Reuse authenticated handshake responses during retries and preserve existing outbox attempts and errors, avoiding repeated database writes for unchanged queued events.
- Send direct files as the first message to a new contact, preserving an unregistered offer while the authenticated peer connection starts.
- Choose a destination before accepting direct files and stream bounded chunks into new files there. Verify every file before completing the transfer, remove incomplete outputs, and preserve existing destination files.
- Add the default-checked “Open chat when someone joins” invitation option. Open only the current invitation's authenticated chat, without interrupting another screen.
- Verify saved physical-device CPU and storage-write evidence with strict delivery, network-isolation, and restoration checks.

### Apple

- Keep nearby device discovery working after restarting the app.
- Reduce repeated background work and storage writes when retrying messages.
- Send files directly when starting a new chat.
- Choose where to save received files before the transfer starts.
- Choose whether to open the chat when someone accepts your invitation.

### Zapstore

- Keep nearby device discovery working after restarting the app.
- Reduce repeated background work and storage writes when retrying messages.
- Send files directly when starting a new chat.
- Choose where to save received files before the transfer starts.
- Choose whether to open the chat when someone accepts your invitation.

## v2026.10.4.4

### GitHub

- Reuse authenticated handshake responses during retries and preserve existing outbox attempts and errors, avoiding repeated database writes for unchanged queued events.
- Send direct files as the first message to a new contact, preserving an unregistered offer while the authenticated peer connection starts.
- Choose a destination before accepting direct files and stream bounded chunks into new files there. Verify every file before completing the transfer, remove incomplete outputs, and preserve existing destination files.
- Add the default-checked “Open chat when someone joins” invitation option. Open only the current invitation's authenticated chat, without interrupting another screen.
- Verify saved physical-device CPU and storage-write evidence with strict delivery, network-isolation, and restoration checks.

### Apple

- Reduce repeated background work and storage writes when retrying messages.
- Send files directly when starting a new chat.
- Choose where to save received files before the transfer starts.
- Choose whether to open the chat when someone accepts your invitation.

### Zapstore

- Reduce repeated background work and storage writes when retrying messages.
- Send files directly when starting a new chat.
- Choose where to save received files before the transfer starts.
- Choose whether to open the chat when someone accepts your invitation.

## v2026.10.4.3

### GitHub

- Restore the compact circular attachment button on macOS while preserving the attachment menu's icons and keyboard actions.
- Show private favorite stars on avatars across native platforms, independently of public follow badges, including offline contacts and live favorite changes.
- Avoid copying every encrypted session when resolving a message sender; keep the existing sender and device authorization checks.
- Add distinct iPhone feedback when voice recording starts, locks, finishes, or is cancelled.
- Add an opt-in physical iPhone/Android Bluetooth regression check for foreground CPU and storage writes.

### Apple

- Restore the round attachment button on Mac.
- Show a star on favorite contacts' avatars.
- Reduce background work when receiving messages and catching up on chats.
- Improve vibration feedback when recording voice messages on iPhone.

### Zapstore

- Show a star on favorite contacts' avatars.
- Reduce background work when receiving messages and catching up on chats.

## v2026.10.4.2

### GitHub

- Route signed update discovery to the configured trusted publisher in the CLI and shared native runtime, preserving a connection slot for the network seed and keeping update peers separate from linked-device permissions.
- Enable same-machine update-provider discovery when the standalone updater also uses WebSocket seeds.
- Open a sender's profile from their avatar in a group chat on Mac and iPhone.
- Avoid starting an iOS background task when there are no delivered notifications to clean up, while protecting notification database reads during suspension.
- Publish the same attested iOS build to internal and public TestFlight groups, reusing existing uploads and submitting only beta review when required.

### Apple

- Improve app update discovery while keeping existing chats and linked devices connected.
- Open a sender's profile by tapping their avatar in a group chat.
- Reduce unnecessary background work when checking read notifications.
- Make this build available through public TestFlight after beta review.

### Zapstore

- Improve app update discovery while keeping existing chats and linked devices connected.

## v2026.10.4

### GitHub

- Align desktop attachment menus with icons for Photos and videos and File choices, while retaining direct file sending.
- Show stable italic animal names for unnamed people instead of full or shortened user IDs, while preserving profile names and nicknames.
- Unify device linking around NostrConnect, retaining existing Iris link input compatibility and existing account sessions.
- Reuse unchanged signed device lists and repair equivalent duplicate snapshots during approved linking, so existing accounts can link without weakening conflict checks.
- Choose between chats and groups only or message history on the approving device. Keep the initial history transfer private to that device pair, resume interrupted transfers, and show progress on the receiving device.
- Reconcile messages and reaction changes over encrypted FIPS using the shared Negentropy codec. Sync current group details, settings, and verified contact profiles separately from old message history. Preserve link-time boundaries, local deletions, expiry, and device revocation during replay and recovery.
- Close interrupted reconciliation rounds when metadata refreshes, so linked devices can resume missing messages and reaction changes without waiting on a discarded inventory.
- Notify linked devices when queued messages become sent, and retry unanswered WebRTC offers after authenticated session restarts without waiting for the old dial timeout.
- Remove legacy paginated history and message-only reconciliation fallbacks. Keep large histories bounded by splitting Negentropy inventories, without changing the private history choice.
- Keep unchanged protocol checkpoints in place to reduce message-receive writes. Update iris-chat-protocol to 0.2.3 with durable exact group-ciphertext tracking and remove unsafe timestamp-only acknowledgements, preserving distinct same-second messages and repair backoff.
- Preserve group encryption settings when copying groups between web and native, rejecting copies that conflict with authenticated group history.
- Respect message-server retry backoff when a batch reports failures before its completion event, avoiding repeated failed drains of a large queue.
- Batch private read receipts into one protocol save and show durable local chat state before saving the protocol backlog on every native platform.
- Retry queued group ciphertext only when its decryption inputs change, with bounded background batches that yield to user actions. Reuse a session snapshot when waking queued direct messages.
- Open saved desktop history independently of checkpoint writes, and avoid mistaking a busy database for an empty chat. Reuse authenticated sessions without showing a messaging-availability check.
- Reuse unchanged encrypted backlog checkpoints and avoid global identity refreshes when opening existing chats. Verify offline sends and recovery with synthetic gigabyte histories and large encrypted queues.
- Build desktop message controls when needed and position initial chat history from measured layout, avoiding hidden control work and premature scrolling.
- Limit iOS message layout to a moving window of 160 rows and preserve measured positions while browsing older history. Keep Mac rows lazy, and measure messages, call entries, and group notices before revealing a newly opened chat.
- Load older chat history in bounded pages and preserve the visible message position as pages arrive. Keep current edits, reactions, expiry, and deletions authoritative across paging and navigation. Keep historical search pending during concurrent checkpoint writes instead of losing the requested message.
- Navigate desktop controls and chat rows with Tab and Shift-Tab, scroll the chat list with arrow keys without changing the active chat, and open the focused chat with Enter or Space. Keep focus when rows scroll out of view, and preserve Tab insertion in message drafts.
- Group consecutive messages from the same person for gaps under three minutes, with compact spacing and reaction, day, and sender boundaries. Keep delivery errors, pending sends, disappearing-message timers, and changed timestamps visible.
- Reuse bounded event-signature validation proofs across transports and suppress unchanged Nearby updates, preserving content validation, delivery receipts, and device-list recovery.
- Preserve an already recovered encrypted session when a direct connection catches up after a peer restarts, using the verified FIPS 0.4.93 dependency set.
- Focus the composer when replying, follow the iOS keyboard animation, and emit pending messages before protocol checkpoint work.
- Wait for audio replay to rewind before starting playback on Apple platforms, and preserve pause, message switching, and seeking while the rewind completes.
- Paste photos and multiple files into drafts without sending automatically, and improve long-draft editing across native platforms.
- Keep private contacts and device labels in ratcheted messages and direct pairing snapshots. Authenticate background call notifications and retain them for retry.
- Serialize Nearby transport replacement and completion-confirmed shutdown, and keep group-member profile navigation consistent.
- Update other devices and calling contacts too. History sync requires updated apps. Calls to older apps may not ring while closed; existing accounts, chats, and linked devices are preserved.

### Apple

- Choose photos and videos or any file from the Mac attachment menu, with icons for each choice.
- Reduce background work while catching up on group messages.
- Open saved chats while messages sync, and avoid unnecessary messaging checks in existing conversations.
- Give unnamed people a friendly animal name in italics instead of showing their user ID.
- Choose whether to copy message history when linking a device, and see transfer progress.
- Resume interrupted history transfers and keep deleted messages removed.
- Keep reactions, group details, and contact profiles in sync across devices.
- Open the keyboard when replying and keep your place as it appears.
- Open chats and show outgoing messages faster, and improve typing after long pastes.
- Keep your place while loading older messages and preserve recent edits and deletions.
- Navigate the Mac app with the keyboard and keep focus while chats update.
- Keep consecutive short messages together, and scroll the Mac chat list with arrow keys without switching chats.
- Paste photos and files into a chat before sending.
- Reliably replay finished voice messages and keep playback controls responsive while seeking.
- Improve private contact sync and Nearby connection recovery.
- Update your other devices and calling contacts too. Calls to an older app may not ring while it is closed. Existing chats and linked devices stay connected.

### Zapstore

- Reduce background work while catching up on group messages.
- Avoid unnecessary messaging checks in existing conversations.
- Give unnamed people a friendly animal name in italics instead of showing their user ID.
- Choose whether to copy message history when linking a device, and see transfer progress.
- Resume interrupted history transfers and keep deleted messages removed.
- Keep reactions, group details, and contact profiles in sync across devices.
- Open chats and show outgoing messages faster, and improve typing after long pastes.
- Keep consecutive messages together with clearer spacing and delivery status.
- Paste photos and files into a chat before sending.
- Improve private contact sync and Nearby connection recovery.
- Update your other devices and calling contacts too. Calls to an older app may not ring while it is closed. Existing chats and linked devices stay connected.

## v2026.10.2

### GitHub

- Paste images and multiple files into native chat drafts, preserving captions and direct-send mode without sending automatically.
- Reduce repeated layout of long drafts on Apple and Windows, keep the cursor visible, and avoid updating the whole Android chat screen on each edit.
- Keep Android message-size changes from racing the initial preference read.
- Move private contacts and device labels to ratcheted device messages, direct transfer, and pairing snapshots. Retire the retained static-encryption sync formats while preserving existing accounts, sessions, and linked devices.
- Publish iris-chat-protocol 0.1.12 with selective retirement of obsolete queued device-sync messages, and verify native-to-web private controls through the production receive path.
- Authenticate background call notifications through ratcheted messages and signed session setup, and retain notifications for retry when a cold lookup fails.
- Update all linked apps for private contact and device-name sync. Both calling endpoints must update for background call notifications; calls to an older app may not ring while it is closed.

### Apple

- Paste photos and files into a chat before sending.
- Improve typing after long text pastes and keep the cursor visible.
- Improve privacy when syncing favorites, nicknames, notes, and device names.
- Update your other devices and calling contacts too. Calls to an older app may not ring while it is closed. Existing chats and linked devices stay connected.

### Zapstore

- Paste photos and files into a chat before sending.
- Improve typing after long text pastes.
- Reliably save message-size changes made just after opening the app.
- Improve privacy when syncing favorites, nicknames, notes, and device names.
- Update your other devices and calling contacts too. Calls to an older app may not ring while it is closed. Existing chats and linked devices stay connected.

## v2026.10.1.8

### GitHub

- Sync private favorites, nicknames, and notes across devices and Iris apps using encrypted contact records, durable offline retries, and independent field merges. Add nickname and note commands to the CLI while keeping public name-change approval separate.
- Preserve direct file transfers in conversation history and align attachment actions on mobile.
- Show Nearby status on user avatars across the native apps.
- Keep chat messages anchored as the keyboard opens and closes on iPhone and Android.
- Keep internal TestFlight delivery restricted to internal tester groups without submitting the build for public beta review.
- Check CLI and app update discovery with separate empty data directories and shipped transport settings.
- Preserve atomic state updates on both the Linux packaging compiler and current Rust toolchains.

### Apple

- Keep favorite stars, nicknames, and private notes in sync across your devices.
- Keep direct file transfers in your chat history.
- See Nearby status on people's avatars.
- Keep your place in a chat when the keyboard opens or closes on iPhone.

### Zapstore

- Keep favorite stars, nicknames, and private notes in sync across your devices.
- Keep direct file transfers in your chat history.
- See Nearby status on people's avatars.
- Keep your place in a chat when the keyboard opens or closes.

## v2026.10.1.6

### GitHub

- Add blue, gold, and gray social connection badges at the top right of list and chat avatars, and beside follow-distance explanations on profiles. Warn about accounts with more mutes than follows in your social network.
- Preserve first-interaction and accepted contact names privately in SQLite. Require approval for a changed public name and record approved changes in the chat history.
- Add private favorite stars and public follow controls that preserve existing contacts, tag hints, and contact-list content.
- Add read-only contact inspection, favorites, exact-name approval, and public follow commands to the CLI; drain queued public updates before a one-shot command exits.
- Add Note to self shortcuts to profiles and search across native apps.
- Send multiple files directly to an accepting device over FIPS, including between your own linked devices, without uploading the files to Blossom or Hashtree.
- Recover device linking after suspension by replacing terminated pairing connections before reconnecting, preserving subscriptions and the existing recovery deadline.
- Use the shared signed update transport and published dependencies while retaining protection against older release announcements.
- Run desktop update checks on Rust worker threads to avoid exhausting the calling app's thread stack.

### Apple

- Remember people's names and choose when to accept a name change.
- See social connections on avatars and profiles, with a warning for accounts often muted in your network.
- Add private favorite stars and follow people publicly from their profile.
- Find Note to self from your profile or search.
- Send files directly to someone who accepts them, including another device of your own.
- Fix device linking after leaving and returning to the app.
- Fix a startup crash on Mac during update checks.

### Zapstore

- Remember people's names and choose when to accept a name change.
- See social connections on avatars and profiles, with a warning for accounts often muted in your network.
- Add private favorite stars and follow people publicly from their profile.
- Find Note to self from your profile or search.
- Send files directly to someone who accepts them, including another device of your own.
- Fix device linking after leaving and returning to the app.

## v2026.9.30.3

### GitHub

- Move iOS notification preview decryption and cache retries off the UI thread, ingest incoming events immediately, and keep notification navigation scoped to the current account and latest tap.
- Preserve authenticated control-event previews after foreground decryption. Distinguish delivered, seen, typing, and stopped-typing updates; use a quiet Chat updated fallback while iOS notification filtering is unavailable.
- Sync pinned chats and timed mutes privately between linked devices, including newly linked devices and explicit unpin or unmute changes.
- Share device names between linked devices and give unnamed devices stable friendly names without redundant Linked device labels.
- Simplify chat headers with consistent call controls, move existing chat search into chat details, and add pin controls to chat details where supported.
- Remove raw user ID placeholders from search results and fix a macOS Settings accessibility crash.

### Apple

- Fix pauses when receiving or opening notifications on iPhone.
- Show accurate delivery, read, and typing updates instead of misleading New message alerts.
- Keep pinned chats and mute durations in sync across your devices.
- Make linked devices easier to recognize with clearer names.
- Refine call buttons and make chat search available from chat details.
- Fix a crash in Mac settings.

### Zapstore

- Keep pinned chats and mute durations in sync across your devices.
- Make linked devices easier to recognize with clearer names.
- Refine call buttons and make chat search available from chat details.
- Improve search results and linked-device setup.

## v2026.9.30.2

### GitHub

- Discover same-machine Hashtree providers by default and serve cached encrypted attachment blocks through Chat's existing FIPS endpoint.
- Reuse files cached by Drive, htree, or another Chat instance without requiring a standalone daemon.
- Preserve the explicit local-sharing opt-out and keep Nearby's LAN preferences independent.
- Verify remote transport authorization separately from same-machine discovery, including contact-session preservation and relayless transit recovery.

### Apple

- Improve access to files already cached by other Iris apps on the same device.
- Keep local file sharing available when Nearby is turned off.

### Zapstore

- Improve access to files already cached by other Iris apps on the same device.
- Keep local file sharing available when Nearby is turned off.

## v2026.9.30.1

### GitHub

- Publish iris-chat-protocol 0.1.11 with the current delivery, group membership, and retry fixes, and update native consumers.
- Retain group history after removal, show a clear notice, and block sending and queued retries across native apps and linked devices.
- Fix Linux screen-sharing picker cancellation on older supported desktops.
- Deliver direct messages to available linked devices while durably retrying unavailable devices, and try all matching receive sessions before rejecting a message.
- Use published nostr-double-ratchet 0.0.168 and remove the temporary vendored library.
- Preserve delivered and seen receipts for unloaded messages without creating gaps in history pagination; keep linked-device unread counts accurate.
- Keep an undecryptable queued message from blocking other senders, and preserve pending decrypted messages across storage failures and restarts.
- Improve large, multi-device group delivery, sender-key recovery, and mesh catch-up while reducing repeated subscription and snapshot work.
- Add desktop screen sharing, call audio-device selection, restored outgoing call tones, and clearer call controls.
- Keep incoming call audio playing when a peer's mute status arrives out of order, and clarify whose microphone is off.
- Add timed chat mute, image copying, and file-drop attachment staging across supported native interfaces.
- Open the correct account and chat from notifications, enforce call blocking immediately, and preserve Android push-token updates.
- Clear local data and caches on logout and end sessions when a signed device list removes the current device.
- Refresh the Mac update banner when a release is found and retain automatic update-check outcomes in Settings.

### Apple

- Keep your group history and clearly show when you can no longer send because you were removed.
- Improve message delivery between linked devices and in large groups.
- Make delivered and read indicators more reliable, with accurate unread counts.
- Choose call audio devices and hear outgoing call tones on iPhone.
- Prevent stale microphone status from silencing incoming call audio.
- Mute chats for a chosen duration, copy images, and attach dropped files where supported.
- Open the right chat from notifications and make signing out clearer.
- Improve responsiveness when accepting chat requests.

### Zapstore

- Keep your group history and clearly show when you can no longer send because you were removed.
- Improve message delivery between linked devices and in large groups.
- Make delivered and read indicators more reliable, with accurate unread counts.
- Choose available call audio devices and restore outgoing call tones.
- Prevent stale microphone status from silencing incoming call audio.
- Mute chats for a chosen duration, copy images, and attach dropped files.
- Open the right chat from notifications and improve push registration reliability.
- Clear local caches when signing out and end sessions on removed devices.

## v2026.9.30

### GitHub

- Deliver direct messages to available linked devices while durably retrying unavailable devices, and try all matching receive sessions before rejecting a message.
- Use published nostr-double-ratchet 0.0.168 and remove the temporary vendored library.
- Preserve delivered and seen receipts for unloaded messages without creating gaps in history pagination; keep linked-device unread counts accurate.
- Keep an undecryptable queued message from blocking other senders, and preserve pending decrypted messages across storage failures and restarts.
- Improve large, multi-device group delivery, sender-key recovery, and mesh catch-up while reducing repeated subscription and snapshot work.
- Add desktop screen sharing, call audio-device selection, restored outgoing call tones, and clearer call controls.
- Keep incoming call audio playing when a peer's mute status arrives out of order, and clarify whose microphone is off.
- Add timed chat mute, image copying, and file-drop attachment staging across supported native interfaces.
- Open the correct account and chat from notifications, enforce call blocking immediately, and preserve Android push-token updates.
- Clear local data and caches on logout and end sessions when a signed device list removes the current device.
- Refresh the Mac update banner when a release is found and retain automatic update-check outcomes in Settings.

### Apple

- Improve message delivery between linked devices and in large groups.
- Make delivered and read indicators more reliable, with accurate unread counts.
- Choose call audio devices and hear outgoing call tones on iPhone.
- Prevent stale microphone status from silencing incoming call audio.
- Mute chats for a chosen duration, copy images, and attach dropped files where supported.
- Open the right chat from notifications and make signing out clearer.
- Improve responsiveness when accepting chat requests.

### Zapstore

- Improve message delivery between linked devices and in large groups.
- Make delivered and read indicators more reliable, with accurate unread counts.
- Choose available call audio devices and restore outgoing call tones.
- Prevent stale microphone status from silencing incoming call audio.
- Mute chats for a chosen duration, copy images, and attach dropped files.
- Open the right chat from notifications and improve push registration reliability.
- Clear local caches when signing out and end sessions on removed devices.

## v2026.9.29.2

### GitHub

- Correct iOS demo guidance to connect review devices through New chat, scanning a code or sharing a chat link.
- Clarify App Store demo setup, sample content, and live call review instructions.
- Use the shared Rust test runner in CI so device-approval tests use a local test server.

### Apple

- Clearer demo instructions for starting chats and testing calls between devices.

### Zapstore

- No Android changes; this release updates iOS demo instructions.

## v2026.9.29.1

### GitHub

- Correct iOS demo guidance to connect review devices through New chat, scanning a code or sharing a chat link.
- Clarify App Store demo setup, sample content, and live call review instructions.
- Keep Rust device-approval tests on a local test server instead of depending on an external service.

### Apple

- Clearer demo instructions for starting chats and testing calls between devices.

### Zapstore

- No Android changes; this release updates iOS demo instructions.

## v2026.9.29

### GitHub

- Keep Apple chat interactions responsive by bounding chat snapshots and navigation caches, reusing unchanged message history, and avoiding repeated GIF reloads.
- Move Apple file staging, share-inbox scans, camera startup, and Bluetooth bridge lifecycle work away from the UI thread.
- Wait for Apple core shutdown and active database readers before resetting local storage, and cancel stale attachment work safely.
- Support explicit browser-to-native chat links on iOS, macOS, and Android, preserving the destination until account setup and device approval finish.
- Simplify attachment preparation and keep sending controls consistent while files are prepared.

### Apple

- Improve responsiveness when typing and opening chats.
- Make photo sharing, camera startup, and nearby connections smoother.
- Keep animated images playing during chat updates.
- Open chat links from the browser and keep the destination while you finish setup.
- Improve reliability when signing out or preparing attachments.

### Zapstore

- Open chat links directly from the web app.
- Keep your chat link ready while you finish setup or approve a linked device.
- Reduce unnecessary work when opening chats.

## v2026.9.28.3

### GitHub

- Validate native desktop call-tone samples with checked access, and simplify the cross-platform call-audio test fixture for older Swift compilers.
- Preserve complete Apple microphone capture batches and drive Apple/Android call playback from device consumption to prevent dropped speech and scheduling gaps.
- Fix iOS suspension crashes by waiting for database shutdown and avoiding SQLite analysis during suspension.
- Add per-device audio auto-download settings, bounded attachment downloads, and cached waveforms before playback on Apple platforms.
- Improve quoted-message bubble sizing and separate clearing chat search from closing it on iOS.
- Keep CLI message reads passive until explicitly marked seen.
- Propagate attachment HTTP client errors instead of panicking, and update FIPS dependencies to 0.4.83.

### Apple

- Improve call audio to reduce broken speech and playback stutters.
- Fix a crash when the app moves into the background.
- Choose when voice messages download automatically.
- Show audio waveforms before playback and reduce repeated audio processing.
- Improve quoted-message layout and make chat search easier to clear or close.
- More reliable attachment downloads.

### Zapstore

- Improve call playback timing to reduce stutters.
- Improve attachment download limits and error handling.
- Update networking components.

## v2026.9.28.2

### GitHub

- Preserve complete Apple microphone capture batches and drive Apple/Android call playback from device consumption to prevent dropped speech and scheduling gaps.
- Fix iOS suspension crashes by waiting for database shutdown and avoiding SQLite analysis during suspension.
- Add per-device audio auto-download settings, bounded attachment downloads, and cached waveforms before playback on Apple platforms.
- Improve quoted-message bubble sizing and separate clearing chat search from closing it on iOS.
- Keep CLI message reads passive until explicitly marked seen.
- Propagate attachment HTTP client errors instead of panicking, and update FIPS dependencies to 0.4.83.

### Apple

- Improve call audio to reduce broken speech and playback stutters.
- Fix a crash when the app moves into the background.
- Choose when voice messages download automatically.
- Show audio waveforms before playback and reduce repeated audio processing.
- Improve quoted-message layout and make chat search easier to clear or close.
- More reliable attachment downloads.

### Zapstore

- Improve call playback timing to reduce stutters.
- Improve attachment download limits and error handling.
- Update networking components.

## v2026.9.28.1

### GitHub

- Fix iOS suspension crashes by waiting for database shutdown and avoiding SQLite analysis during suspension.
- Add per-device audio auto-download settings, bounded attachment downloads, and cached waveforms before playback on Apple platforms.
- Improve quoted-message bubble sizing and separate clearing chat search from closing it on iOS.
- Keep CLI message reads passive until explicitly marked seen.
- Propagate attachment HTTP client errors instead of panicking, and update FIPS dependencies to 0.4.83.

### Apple

- Fix a crash when the app moves into the background.
- Choose when voice messages download automatically.
- Show audio waveforms before playback and reduce repeated audio processing.
- Improve quoted-message layout and make chat search easier to clear or close.
- More reliable attachment downloads.

### Zapstore

- Improve attachment download limits and error handling.
- Update networking components.

## v2026.9.28

### GitHub

- Add iOS review onboarding with fresh per-device identities, real sample chat history, a sample group, and bundled playable audio. Include review instructions in Apple submissions.
- Add inline voice-message playback, seeking, waveforms, and playback speed controls across native apps.
- Add private contact nicknames and notes, improve name search, and label forwarded messages consistently.
- Preserve direct-message IDs from queued sending through delivery.
- Add a single-owner CLI service and improve Android invite layouts with large text.

### Apple

- Play voice messages inside chats, seek through audio, and adjust playback speed.
- Add private nicknames and notes for contacts.
- Improved name search and clearer forwarded messages.
- More reliable queued-message delivery.
- Add a demo profile with sample conversations on iPhone and iPad.

### Zapstore

- Play voice messages inside chats and seek through audio.
- Add private nicknames and notes for contacts.
- Improved name search and clearer forwarded messages.
- More reliable queued-message delivery.
- Easier-to-use invite screens with large text.

## v2026.9.26

### GitHub

- Make iPhone voice and video call buttons easier to tap and keep progress visible while granting permissions and starting a call.
- Avoid repeated permission requests and main-thread camera discovery when starting Apple calls.
- Play outgoing connecting and ringing tones on iOS, Android, macOS, Windows, and Linux. Ringing begins when the other device responds, and tones stop when the call is answered or ends.
- Show call-start errors on every attempt, including repeated failures and disabled calling.

### Apple

- Easier-to-tap voice and video call buttons on iPhone.
- Clearer call progress while granting microphone and camera access.
- Hear connecting and ringing sounds when calling someone.
- More reliable feedback when a call cannot start.

### Zapstore

- Hear connecting and ringing sounds when calling someone.
- See when the other device is ringing.
- More reliable feedback when a call cannot start.

## v2026.9.24.7

### GitHub

- Add native voice and video calls on Windows and Linux with echo cancellation, adaptive video quality and bounded media queues.
- Stop sending captured audio and video promptly when muting, turning off the camera or ending a desktop call, including when a stale call update arrives.
- Pace Android camera frames when adapting to lower video frame rates.
- Add desktop codec, audio, privacy and call-screen regression checks to platform verification and include the desktop media licenses in release packages.
- Includes the idle CPU, queued-message timestamp, nearby-delivery and incoming-call fixes from v2026.9.24.5.

### Apple

- Make voice and video calls, adjust video quality, and answer a video call with voice only.
- Improved incoming call alerts and call recovery.
- Reduced idle CPU use and improved message delivery.
- Queued messages keep their original timestamps.
- Record voice messages on iPhone and play audio inside chats.
- Keep typing while connection checks run, and adjust message text size.

### Zapstore

- Smoother video when calls adjust to slower connections.
- Improved incoming call alerts and message delivery.
- Reduced repeated background work.
- Queued messages keep their original timestamps.

## v2026.9.24.5

### GitHub

- Reduce idle CPU use by reusing unchanged nearby identity announcements and avoiding repeated processing of messages waiting for device verification.
- Retry unacknowledged nearby deliveries with bounded backoff instead of depending on another connection change.
- Preserve original message timestamps when sending queued messages and recovering encrypted or legacy messages after a restart.
- Add iPhone voice-message recording and inline audio playback.
- Keep the message composer available during connection checks, preserve input focus on Linux, and fix the Windows send shortcut.
- Add adjustable message text size and desktop zoom controls.
- Improve video keyframe recovery and adapt call traffic to constrained connections.
- Wake native incoming calls through encrypted push notifications, retry temporary Android push-registration failures, and improve Apple call dismissal and background recovery.

### Apple

- Reduced idle CPU use and improved message processing.
- Improved nearby delivery when a packet is lost during reconnection.
- Messages keep their original timestamps when sent after a delay or recovered after restarting.
- Record voice messages on iPhone and play audio inside chats.
- Keep typing while connection checks run, and adjust message text size.
- Improved video calls, incoming call alerts, and call dismissal.

### Zapstore

- Reduced repeated background work and improved message processing.
- Improved nearby delivery when a packet is lost during reconnection.
- Messages keep their original timestamps when sent after a delay or recovered after restarting.
- Keep typing while connection checks run, and adjust message text size.
- Improved video calls and incoming call alerts.
- Retry push registration after temporary network or notification-service failures.

## v2026.9.24.4

### GitHub

- Add authenticated voice and video calls over the shared FIPS transport on Android, iOS and macOS, using Opus audio, hardware H.264 video, adaptive bitrate and adjustable quality limits.
- Integrate iOS CallKit, Android Telecom and desktop call alerts. Coordinate answering, declining and cancellation across linked devices, and stop ringing when the caller disappears.
- Persist missed, answered, declined and canceled call summaries in chat history. Allow voice-only answers to video calls and independently disable voice or video calls.
- Preserve local FIPS routes without internet access and use optional STUN discovery for direct transport upgrades without a separate calling server.
- Add external signer login and device authorization across native clients, including Android signer apps and NIP-46 signers.
- Show live linked-device connection status and improve Apple composer responsiveness and draft stability.
- Make profile names optional across native platforms and fix iPhone keyboard submission after accepting the terms.
- Add an iPhone attachment sheet with recent photos, Camera, Photos and Files, preserving drafts and preparing selected media without blocking typing.

### Apple

- Make voice and video calls with compatible Iris contacts, including over an existing local connection without internet access.
- Adjust video quality and data use during calls, or answer a video call with voice only.
- See missed and answered calls in your chats. Answering or declining stops ringing on your other devices.
- Choose whether to enable voice and video calls in Settings.
- Sign in with a connected signing app and see which linked devices are connected.
- Create a profile without entering a name.
- Pick recent photos, use the camera or browse files from the new iPhone attachment sheet.
- Improved typing responsiveness, draft stability and call audio reliability.

### Zapstore

- Make voice and video calls with compatible Iris contacts, including over an existing local connection without internet access.
- Adjust video quality and data use during calls, or answer a video call with voice only.
- Answer through Android's call interface and see missed and answered calls in your chats.
- Answering or declining stops ringing on your other devices. Choose whether to enable voice and video calls in Settings.
- Sign in with a signing app and see which linked devices are connected.
- Create a profile without entering a name.

## v2026.9.23

### GitHub

- Deliver own-device encrypted traffic as silent background pushes and dismiss only notifications covered by authenticated read progress, preserving newer unread alerts.
- Register push notifications on linked devices using their device key when the account secret key is absent.
- Sync read progress and unread counts between linked devices, including after reconnecting or restoring history. Marking a chat unread remains local, and read-receipt privacy settings still apply to other people.
- Keep pending read updates for offline linked devices while delivering immediately to devices that are ready.
- Avoid repeated Apple Bluetooth discovery and reopening links that already have a working network path.
- Keep nearby message delivery running after a temporary connection-status timeout.
- Defer speculative Bluetooth discovery on macOS while Bluetooth audio is active, then resume after playback or calls stop.

### Apple

- Updates from your own devices no longer create message alerts.
- Read updates can clear matching notifications on your other devices.
- Reading a chat now updates its unread count on your other devices.
- Improved syncing when a linked device comes back online.
- Reduced repeated Bluetooth connections and interruptions to Bluetooth audio on Mac.

### Zapstore

- Read updates clear matching notifications on your other devices without creating new alerts.
- Reading a chat now updates its unread count on your other devices.
- Improved syncing when a linked device comes back online.

## v2026.9.22.11

### GitHub

- Carry account-signed device approval inside encrypted chat handshakes, retaining compatibility with existing handshakes and message encryption. Linked devices do not need the account secret key.
- Recover device registration after account restoration and wait for local approval before sending, preserving existing devices and known revocations.
- Preserve undelivered messages for retry and recover exact signed registration evidence during upgrades.
- Sync chat deletions between linked devices and prevent deleted chats from returning through older history.
- Keep chat navigation, search, image decoding, and composer edits from blocking or repeatedly rebuilding the message timeline.
- Exclude internet connections from Nearby and size reply quotes to their message bubbles.
- Respect system autocorrection and spelling preferences in Apple text composers.

### Apple

- Improved message delivery when using an existing account or a linked device.
- Improved recovery of messages waiting for device verification.
- Chat deletions now sync between your devices.
- Improved chat opening, typing, search, and image responsiveness.
- Fixed incorrect Nearby listings and reply quote widths.
- Iris now respects your system autocorrect settings.

### Zapstore

- Improved message delivery and device verification for existing accounts and linked devices.
- Improved recovery of pending messages and syncing of chat deletions.
- Fixed incorrect Nearby listings.

## v2026.9.22.1

### GitHub

- Fix signed update checks for installed calendar revision versions such as `2026.9.10.1`.
- Generate update metadata with supported installation kinds and retain compatibility with older readers when publishing same-day revisions.
- Require the exact released executable to resolve both CLI and native-app updates before Hashtree promotion succeeds, and preserve the underlying error when a check fails.

### Apple

- Fixed update checks for releases with a same-day revision number.
- Improved error details when an update cannot be checked.

### Zapstore

- Improved compatibility and verification of release metadata used by desktop and command-line updates.

## v2026.9.22

### GitHub

- Bind local chat storage to its account and reject attempts to open it with a different identity, protecting existing history from mixed-account writes.
- Restore people search using the active Iris Social index, improve social filtering and ranking, and keep repeated searches responsive.
- Hide your own linked devices from Nearby and forward signed identities between linked devices so nearby contacts can be identified reliably.
- Keep wrapped messages readable on iOS when expanding and collapsing message text.
- Add CLI profile inspection and partial profile updates that preserve fields you did not edit.
- Build Linux CLI releases for Debian 12 compatibility and test the installer as both a regular user and root before publishing.

### Apple

- Improved protection against opening one account's chat history with a different account.
- Restored people search and improved search results and responsiveness.
- Improved nearby contact identification and hidden your own linked devices from Nearby.
- Fixed readability of wrapped messages on iOS.

### Zapstore

- Improved protection against opening one account's chat history with a different account.
- Restored people search and improved search results and responsiveness.
- Improved nearby contact identification and hidden your own linked devices from Nearby.

## v2026.9.10.1

### GitHub

- Limit repeated pubsub connection attempts to unavailable services while preserving immediate initial connections, established traffic, and queued-event recovery.
- Recover session key rotation in Tree routing mode after topology changes clear routing coordinates, using bounded discovery while existing traffic continues.
- Size FIPS crypto work buffers for the active packet batch and avoid copying peer identities during mesh selection while keeping trust decisions fresh.
- Adopt the corrected FIPS and Hashtree dependency closure across every native platform, with release gates for relayless recovery, CPU use, and bandwidth.

### Apple

- Reduced unnecessary background connection work when another device is temporarily unavailable.
- Reduced temporary memory use during small network updates.
- Improved connection reliability as the network changes.

### Zapstore

- Reduced unnecessary background connection work when another device is temporarily unavailable.
- Reduced temporary memory use during small network updates.
- Improved connection reliability as the network changes.

## v2026.9.10

### GitHub

- Reduce idle FIPS session-report traffic while preserving live traffic measurements and recovery after an interrupted route.
- Apply one bounded reputation projection to mesh peer preference and event admission. Optional configured rating entrypoints default to empty; good service does not automatically authorize a peer to rate others.
- Keep local peer observations useful when rating publication fails, and manage the shared rating exchange inside the pubsub adapter.
- Stop shared pubsub tasks when the app stops device connections, even when another component retains the client.
- Require relayless signed-event and verified-blob recovery, CPU limits, and bandwidth limits against the exact tagged source before release publication.
- Preserve profile update freshness across restart and exclude mesh servers from nearby user previews.

### Apple

- Reduced background network traffic between connected devices.
- Stopped background connection work reliably when device connections are turned off.
- Improved how the app chooses available device connections.
- Fixed stale profile updates after restarting the app and improved Nearby discovery.

### Zapstore

- Reduced background network traffic between connected devices.
- Stopped background connection work reliably when device connections are turned off.
- Improved how the app chooses available device connections.
- Fixed stale profile updates after restarting the app and improved Nearby discovery.

## v2026.9.9

### GitHub

- Carry signed Chat protocol events and attachment reads across known native FIPS peers, including routes through peers outside the conversation.
- Keep mesh subscriptions active across peer changes and retain the existing message encryption, signature checks, and device authorization.
- Keep linked-device sync available with no message servers configured and recover interrupted shared file reads through updated Hashtree dependencies.
- Refresh nearby identities and recover missing peer device lists; avoid showing an offline warning before login.

### Apple

- Improved direct device connections and recovery after a connection drops.
- Improved shared file downloads and nearby contact discovery.
- Fixed an offline warning appearing before sign-in.

### Zapstore

- Improved direct device connections and recovery after a connection drops.
- Improved shared file downloads and nearby contact discovery.
- Fixed an offline warning appearing before sign-in.

## v2026.9.8.4

### GitHub

- Update the shared file transport to try additional peers on repeated requests, so a file held beyond the first four peers can be found.
- Preserve the existing per-request attempt limit and deadlines, and report an incomplete search when peers remain untried.
- Update iris-chat to 0.1.45 with hashtree-fips-transport 0.4.15.
- Publish the matching iris-chat-protocol 0.1.10 package and require it from the CLI, keeping registry builds on the same protocol API and ratchet dependency as the apps.

### Apple

- Improved reliability when retrieving shared files from other devices.

### Zapstore

- Improved reliability when retrieving shared files from other devices.

## v2026.9.8.3

### GitHub

- Increase Android release packaging memory and bound worker concurrency to prevent dex merger heap exhaustion.
- Back off persistent UDP receive failures so networking workers cannot spin continuously after a socket error.
- Verify mobile push events before buffering them during account restore, preventing malformed pushes from blocking genuine message delivery.
- Make APNs token waits finish on timeout, cancellation, and registration failure.
- Add physical iPhone coverage for decrypted push previews and opening the received message.
- Advance App Store versions for same-day corrective releases.

### Apple

- Fixed a networking issue that could cause excessive battery use.
- Improved message delivery when opening a notification.

### Zapstore

- Fixed a networking issue that could cause excessive battery use.
- Improved message delivery when opening a notification.

## v2026.9.8.2

### GitHub

- Back off persistent UDP receive failures so networking workers cannot spin continuously after a socket error.
- Verify mobile push events before buffering them during account restore, preventing malformed pushes from blocking genuine message delivery.
- Make APNs token waits finish on timeout, cancellation, and registration failure.
- Add physical iPhone coverage for decrypted push previews and opening the received message.
- Advance App Store versions for same-day corrective releases.

### Apple

- Fixed a networking issue that could cause excessive battery use.
- Improved message delivery when opening a notification.

### Zapstore

- Fixed a networking issue that could cause excessive battery use.
- Improved message delivery when opening a notification.

## v2026.9.8

### GitHub

- Updated encrypted-message dependencies to reject malformed and oversized
  payloads before they can disrupt message processing.
- Updated relay event verification and authentication queue limits, and
  included dependency soundness fixes.

### Apple

- Improved protection against malformed messages and unreliable message servers.

### Zapstore

- Improved protection against malformed messages and unreliable message servers.

## v2026.9.7

### GitHub

- Hardened device linking, attachment imports, incoming event validation, and
  private key storage.
- Updated Hashtree dependencies, including secure update verification and
  installation protections against unsafe paths and temporary-file links.
- People search now shows only users with compatible messaging devices.
- Added a default-off setting to load original images when the image proxy
  fails, with a clear warning before enabling it.

### Apple

- Improved security when linking devices, opening attachments, and installing
  updates.
- People search now shows users who can receive messages.
- Added an optional image-loading fallback when the image service is unavailable.

### Zapstore

- Improved security when linking devices and opening shared attachments.
- People search now shows users who can receive messages.
- Added an optional image-loading fallback when the image service is unavailable.

## v2026.9.5

### GitHub

- Fixed duplicate rows and unstable previews in the protocol library's
  direct-message listing when messages share a timestamp.
- Moved Start at login into General settings on macOS, Windows, and Linux.
- Expanded the fast Rust gate to cover protocol and FFI tests, reused build
  artifacts across crates, and removed redundant CLI test builds.

### Apple

- Start at login now appears under General settings on Mac.

### Zapstore

- This release has no Android-facing changes.

## v2026.8.28

### GitHub

- People search now finds Iris users beyond existing chats and follows, with
  bounded remote profile discovery and friend-supported ranking.
- Search results remain useful offline, exclude blocked people, and refresh
  safely when account, connection, or profile information changes.
- New direct chats now wait for a verified compatible device before messaging
  is enabled, with clear unavailable and retry states on every platform.

### Apple

- People search can now find more Iris users and prioritizes people connected
  to your friends.
- New chats check for a compatible device before messaging is enabled and offer
  a clear retry when the check cannot complete.

### Zapstore

- People search can now find more Iris users and prioritizes people connected
  to your friends.
- New chats check for a compatible device before messaging is enabled and offer
  a clear retry when the check cannot complete.

## v2026.8.23.1

### GitHub

- Updated image-proxy hex decoding for Rust 1.98 release validation without
  changing its behavior.
- Added an iOS App Store update controller with regional lookups, verified Apple
  links, cached checks, and per-version dismissal.
- Added an adaptive update banner across iOS screens, with status-banner layout
  that keeps navigation and chat content unobscured.
- App Store distribution retries now safely resume the exact ready-for-review
  submission without recreating its version or re-uploading its build.

### Apple

- Iris Chat now lets you know when a newer version is available and opens the
  App Store when you choose Update.
- Update and connection notices now stay neatly above your chats without
  covering content.

### Zapstore

- This release has no Android-facing changes.

## v2026.8.23

### GitHub

- Added an iOS App Store update controller with regional lookups, verified Apple
  links, cached checks, and per-version dismissal.
- Added an adaptive update banner across iOS screens, with status-banner layout
  that keeps navigation and chat content unobscured.
- App Store distribution retries now safely resume the exact ready-for-review
  submission without recreating its version or re-uploading its build.

### Apple

- Iris Chat now lets you know when a newer version is available and opens the
  App Store when you choose Update.
- Update and connection notices now stay neatly above your chats without
  covering content.

### Zapstore

- This release has no Android-facing changes.

## v2026.8.19

### GitHub

- iOS notification taps received during profile restoration now wait for
  authorization and open the intended chat exactly once.
- Android chat image previews and full-screen images now honor embedded EXIF
  rotation and reflection metadata while decoding at an appropriate size.
- The macOS message composer now preserves native edits and cursor state during
  SwiftUI reconciliation.
- The macOS message composer now grows and scrolls reliably for multiline text.

### Apple

- Opening a notification while a profile is being restored now reliably opens the intended chat.
- The Mac message box no longer moves the cursor unexpectedly while editing.
- The Mac message box now grows and scrolls reliably for longer messages.

### Zapstore

- Photos with embedded camera orientation now display correctly in chats and the full-screen viewer.

## v2026.8.12

### GitHub

- New Double Ratchet invite responses prove control of their claimed session
  key before the session is installed, preventing another identity from
  claiming an observed session key.
- Zapstore releases now publish the app icon reliably.

### Apple

- Secure chat invites now verify that the person accepting the invite controls
  the new encryption key before the chat starts.

### Zapstore

- Secure chat invites now verify that the person accepting the invite controls
  the new encryption key before the chat starts.
- Fixed the app icon in Zapstore releases.

## v2026.8.1

### GitHub

- Attachment messages now wait for a confirmed upload and report failures
  instead of sending an empty message.
- Queued attachment sends retain their uploaded file details until delivery,
  including across connection interruptions.
- Restoring an existing profile now preserves its identity and profile details
  reliably across iOS and Android.
- Opening a chat no longer sends a typing indicator before text is entered.
- iOS and macOS show message hover actions for only one message at a time.
- Release artifacts are now built once by GitHub and promoted unchanged through
  the supported distribution channels.

### Apple

- Fixed photo and file messages that could appear empty or fail to reach other devices.
- Made queued attachments more reliable during connection interruptions.
- Restoring an existing profile now keeps its identity and profile details reliably.
- Typing indicators now appear only after someone starts typing.
- Message actions no longer appear on multiple messages at once on iPhone and Mac.

### Zapstore

- Fixed photo and file messages that could appear empty or fail to reach other devices.
- Made queued attachments more reliable during connection interruptions.
- Restoring an existing profile now keeps its identity and profile details reliably.
- Typing indicators now appear only after someone starts typing.

## v2026.7.27.1

### GitHub

- Rebuilt the iOS release with Xcode 26 for current App Store compatibility.

### Apple

- Updated Apple compatibility for App Store delivery.

### Zapstore

- Updated Apple compatibility for App Store delivery.

## v2026.7.27

### GitHub

- Messages you send to yourself now reach every linked device instead of
  waiting on the sending device, and previously stuck messages recover after
  updating.
- Linking after deleting local data now reliably shows a roomy, centered code
  or a clear retry action.
- Link codes include the requesting device name so linked devices are
  recognizable in Devices.
- Creating a profile and linking a device now show failures clearly without
  changing their button labels while work completes.
- Release checks now verify real iPhone and Android notifications through the
  production notification server.

### Apple

- Messages you send to yourself now reach every linked device instead of waiting on the sending device.
- Messages stuck in that old queued state recover automatically after updating.
- Linking after deleting local data now reliably shows a roomy, centered scan code or a clear retry action.
- New link codes include the requesting device name so it is recognizable in Devices.
- Creating a profile and linking a device now keep stable button labels while work completes.
- macOS test runs no longer terminate an installed Iris Chat app.

### Zapstore

- Messages you send to yourself now reach every linked device instead of waiting on the sending device.
- Messages stuck in that old queued state recover automatically after updating.
- Linking after deleting local data now reliably shows a roomy, centered scan code or a clear retry action.
- New link codes include the requesting device name so it is recognizable in Devices.
- Creating a profile and linking a device now keep stable button labels while work completes.
- macOS test runs no longer terminate an installed Iris Chat app.

## v2026.7.23

### GitHub

- A linked device now goes directly from its link code to the chat list; the
  redundant "Finish linking" screen has been removed on every platform.
- Device approval now waits only for the exact owner-signed device entry and
  the response authenticated by that link code, while the optional receipt no
  longer delays login.
- Interrupted, disconnected, or reordered approvals retry automatically within
  two seconds and survive app relaunches and updates.
- Messages waiting for local device keys now retry normally instead of staying
  stuck in `MissingLocalAppKeys`.

### Apple

- Linking a device now goes straight from the code to your chats, with no extra finishing screen.
- Interrupted or out-of-order approvals recover automatically, even after reopening or updating the app.
- Linking still verifies both the exact signed device entry and the secret in that specific code.
- Messages waiting for device details retry automatically instead of getting stuck.

### Zapstore

- Linking a device now goes straight from the code to your chats, with no extra finishing screen.
- Interrupted or out-of-order approvals recover automatically, even after reopening or updating the app.
- Linking still verifies both the exact signed device entry and the secret in that specific code.
- Messages waiting for device details retry automatically instead of getting stuck.

## v2026.7.22

### GitHub

- Restoring a profile with a valid secret key now continues without requiring
  another tap or click.
- Secret-key restores can send immediately while the linked-device roster is
  recovered, and messages already waiting for that roster retry automatically.
- Message delivery status uses familiar checkmarks instead of a paper-plane
  icon, including when delivered and seen receipts are disabled.
- Fresh installs default typing indicators and read receipts off while keeping
  notifications enabled on every supported platform.

### Apple

- Restoring a profile with a valid secret key now continues automatically.
- Messages no longer remain stuck while device details are restored; queued messages recover automatically.
- Sent-message status now uses familiar checkmarks instead of a paper-plane icon.
- Typing indicators and read receipts are off by default; notifications are on by default.

### Zapstore

- Restoring a profile with a valid secret key now continues automatically.
- Messages no longer remain stuck while device details are restored; queued messages recover automatically.
- Sent-message status now uses familiar checkmarks instead of a paper-plane icon.
- Typing indicators and read receipts are off by default; notifications are on by default.

## v2026.7.21

### GitHub

- Linked devices recover direct connections more reliably after changing
  networks or briefly losing connectivity.
- Connection retries now wait for validated payload traffic instead of being
  cancelled by control-only heartbeats.
- Linked-device routing uses FIPS 0.4.34 and the newest shared
  `nostr-pubsub` peer adapter.

### Apple

- Linked devices reconnect more reliably after changing networks.
- Interrupted secure connections and key refreshes recover more quickly.
- Peer messaging now waits for real delivered data before considering a link healthy.

### Zapstore

- Linked devices reconnect more reliably after changing networks.
- Interrupted secure connections and key refreshes recover more quickly.
- Peer messaging now waits for real delivered data before considering a link healthy.

## v2026.7.19

### GitHub

- Linked-device connections now use the independent Osiris and LNVPS FIPS
  WebSocket entry points by default.
- Signed update announcements now use the newest shared `nostr-pubsub` stack
  across configured Nostr relays and connected FIPS peers.
- Linked-device traffic now uses authenticated FIPS WebSocket seed peers;
  message servers remain ordinary Nostr event and discovery relays rather than
  carrying FIPS packets.
- Linked devices now recover ordered chat, group, key, and recent-message snapshots over TCP/FIPS.
- Message delivery and seen indicators still reflect recipient application receipts, not transport acknowledgements.
- Attachment downloads can opt into reusing a Hashtree provider running under the same user, then continue through the existing storage path if it has no result or exits.

### Apple

- Linked devices now use two independent secure connection points by default.
- Update notices can arrive through message servers or directly from linked devices.

### Zapstore

- Linked devices now use two independent secure connection points by default.
- Update notices can arrive through message servers or directly from linked devices.

## v2026.6.30

### GitHub

- Group membership now syncs through shared roster fact snapshots across web and native apps.
- Linked-device authorization now comes from owner-signed kind 37368 AppKeys snapshots.
- Web and native interop checks now cover direct chats, linked devices, and groups before release.

### Apple

- Group membership now syncs through shared roster fact snapshots across web and native apps.
- Linked-device authorization now comes from owner-signed kind 37368 AppKeys snapshots.
- Release checks cover direct chats, linked devices, and groups across web and native apps.

### Zapstore

- Group membership now syncs through shared roster fact snapshots across web and native apps.
- Linked-device authorization now comes from owner-signed kind 37368 AppKeys snapshots.
- Release checks cover direct chats, linked devices, and groups across web and native apps.

## v2026.6.29

### GitHub

- Linked-device approval writes the shared AppKeys device roster directly for new manual adds.
- Message requests now show Accept, Block, and Block and report actions directly, with Delete chat and Unblock available from the safety flow.

### Apple

- Linking a device is more reliable across app restarts and fresh installs.
- Device approvals now use the shared AppKeys roster, keeping new linked devices in sync.
- Internal roster handling was split into smaller pieces so release checks catch regressions cleanly.

### Zapstore

- Linking a device is more reliable across app restarts and fresh installs.
- Device approvals now use the shared AppKeys roster, keeping new linked devices in sync.
- Internal roster handling was split into smaller pieces so release checks catch regressions cleanly.

## v2026.6.9

### GitHub

- Messages recover more reliably after restart, offline use, or message-server reconnects.
- Startup no longer waits on message-server status before chat recovery can continue.
- Linked devices and queued messages retry missing chat state more reliably.

### Apple

- Messages recover more reliably after restart, offline use, or message-server reconnects.
- Startup no longer waits on message-server status before chat recovery can continue.
- Linked devices and queued messages retry missing chat state more reliably.

### Zapstore

- Messages recover more reliably after restart, offline use, or message-server reconnects.
- Startup no longer waits on message-server status before chat recovery can continue.
- Linked devices and queued messages retry missing chat state more reliably.

## v2026.6.3

### GitHub

- Messages recover more reliably after the app was closed, restarted, or offline.
- Group messages and linked devices retry missing keys instead of getting stuck.
- Restoring an existing profile with a secret key is covered by broader phone and simulator tests.
- Desktop builds and release tests now cover more real app journeys.

### Apple

- Messages recover more reliably after the app was closed, restarted, or offline.
- Group messages and linked devices retry missing keys instead of getting stuck.
- Restoring an existing profile with a secret key is covered by broader phone and simulator tests.
- Desktop builds and release tests now cover more real app journeys.

### Zapstore

- Messages recover more reliably after the app was closed, restarted, or offline.
- Group messages and linked devices retry missing keys instead of getting stuck.
- Restoring an existing profile with a secret key is covered by broader phone and simulator tests.
- Desktop builds and release tests now cover more real app journeys.

## v2026.5.29

### GitHub

- iOS notifications stay off by default until turned on in Settings.
- Blocked message requests stay open for review and disappear from the chat list after you leave.
- Typing indicators are on by default.

### Apple

- iOS notifications stay off by default until turned on in Settings.
- Blocked message requests stay open for review and disappear from the chat list after you leave.
- Typing indicators are on by default.

### Zapstore

- iOS notifications stay off by default until turned on in Settings.
- Blocked message requests stay open for review and disappear from the chat list after you leave.
- Typing indicators are on by default.

## v2026.5.27

### GitHub

- Onboarding now asks people to agree to Terms before creating, restoring, or linking a profile.
- Welcome screens, app icons, splash art, and notification icons are cleaner and more consistent.
- Pending outgoing messages now use a send icon, keeping the clock/timer icon for disappearing messages.
- Linux chats now include link actions.
- Split oversized iOS Swift UI files and added a repo-wide source file size ratchet.

### Apple

- Onboarding now asks people to agree to Terms before creating, restoring, or linking a profile.
- Welcome screens, app icons, splash art, and notification icons are cleaner and more consistent.
- Pending outgoing messages now use a send icon, keeping the clock/timer icon for disappearing messages.
- Linux chats now include link actions.
- Split oversized iOS Swift UI files and added a repo-wide source file size ratchet.

### Zapstore

- Onboarding now asks people to agree to Terms before creating, restoring, or linking a profile.
- Welcome screens, app icons, splash art, and notification icons are cleaner and more consistent.
- Pending outgoing messages now use a send icon, keeping the clock/timer icon for disappearing messages.
- Linux chats now include link actions.
- Split oversized iOS Swift UI files and added a repo-wide source file size ratchet.

## v2026.5.23.1

### GitHub

- Messages reveal less delivery metadata to message servers.
- Group message recovery still works with older app versions.
- Message repair requests avoid sharing hidden delivery counters.

### Apple

- Messages reveal less delivery metadata to message servers.
- Group message recovery still works with older app versions.
- Message repair requests avoid sharing hidden delivery counters.

### Zapstore

- Messages reveal less delivery metadata to message servers.
- Group message recovery still works with older app versions.
- Message repair requests avoid sharing hidden delivery counters.

## v2026.5.20.2

### GitHub

- Chats now fetch missing profile details when needed, so names and photos appear more reliably.
- Desktop notifications now work more consistently after switching away from Iris.
- Settings no longer show secret device-key copy/export actions.
- Chat-list profile avatars feel cleaner when tapped.

### Apple

- Chats now fetch missing profile details when needed, so names and photos appear more reliably.
- Desktop notifications now work more consistently after switching away from Iris.
- Settings no longer show secret device-key copy/export actions.
- Chat-list profile avatars feel cleaner when tapped.

### Zapstore

- Chats now fetch missing profile details when needed, so names and photos appear more reliably.
- Desktop notifications now work more consistently after switching away from Iris.
- Settings no longer show secret device-key copy/export actions.
- Chat-list profile avatars feel cleaner when tapped.

## v2026.5.20.1

### GitHub

- Group messages recover more reliably after app restarts and missed key updates.
- New chats with known linked devices get unstuck more often.
- Recovery retries are quieter and survive restart.

### Apple

- Group messages recover more reliably after app restarts and missed key updates.
- New chats with known linked devices get unstuck more often.
- Recovery retries are quieter and survive restart.

### Zapstore

- Group messages recover more reliably after app restarts and missed key updates.
- New chats with known linked devices get unstuck more often.
- Recovery retries are quieter and survive restart.

## v2026.5.18.6

### GitHub

- Foreground stays responsive during catch-up bursts and large group metadata updates.

### Apple

- Foreground stays responsive during catch-up bursts and large group metadata updates.

### Zapstore

- Foreground stays responsive during catch-up bursts and large group metadata updates.

## v2026.5.18.5

### GitHub

- Linked devices now learn remote-created groups after restart.
- Group messages recover more reliably after app restore.
- Android release checks now rebuild Rust path dependencies when shared protocol code changes.
- Android storage avoids a native SQLite crash seen during relay publishing.

### Apple

- Linked devices now learn remote-created groups after restart.
- Group messages recover more reliably after app restore.
- Android release checks now rebuild Rust path dependencies when shared protocol code changes.
- Android storage avoids a native SQLite crash seen during relay publishing.

### Zapstore

- Linked devices now learn remote-created groups after restart.
- Group messages recover more reliably after app restore.
- Android release checks now rebuild Rust path dependencies when shared protocol code changes.
- Android storage avoids a native SQLite crash seen during relay publishing.

## v2026.5.18.4

### GitHub

- Nearby profiles now open as profiles instead of being mistaken for chats.
- Profile nickname editing no longer shows a placeholder nickname as saved data.
- Desktop message actions sit beside bubbles more neatly.
- Idle sync retries use less CPU.
- macOS release builds find the shared Cargo build directory more reliably.

### Apple

- Nearby profiles now open as profiles instead of being mistaken for chats.
- Profile nickname editing no longer shows a placeholder nickname as saved data.
- Desktop message actions sit beside bubbles more neatly.
- Idle sync retries use less CPU.
- macOS release builds find the shared Cargo build directory more reliably.

### Zapstore

- Nearby profiles now open as profiles instead of being mistaken for chats.
- Profile nickname editing no longer shows a placeholder nickname as saved data.
- Desktop message actions sit beside bubbles more neatly.
- Idle sync retries use less CPU.
- macOS release builds find the shared Cargo build directory more reliably.

## v2026.5.18.2

### GitHub

- Adding people to groups now asks for confirmation before sending invites.
- Linked device names can be renamed from Devices.
- Nearby now shows cleaner chat-list shortcuts, opens chats from nearby avatars, and appears in mobile sharing.
- Removed linked devices now stay removed more reliably.

### Apple

- Adding people to groups now asks for confirmation before sending invites.
- Linked device names can be renamed from Devices.
- Nearby now shows cleaner chat-list shortcuts, opens chats from nearby avatars, and appears in mobile sharing.
- Removed linked devices now stay removed more reliably.

### Zapstore

- Adding people to groups now asks for confirmation before sending invites.
- Linked device names can be renamed from Devices.
- Nearby now shows cleaner chat-list shortcuts, opens chats from nearby avatars, and appears in mobile sharing.
- Removed linked devices now stay removed more reliably.

## v2026.5.18.1

### GitHub

- Group photos now persist and appear in chats, chat lists, and group details.

### Apple

- Group photos now persist and appear in chats, chat lists, and group details.

### Zapstore

- Group photos now persist and appear in chats, chat lists, and group details.

## v2026.5.17.1

### GitHub

- Linked devices now show clearer app, OS, and device labels where available.
- Messages to a newly restored linked device now wait for its device keys and retry automatically.

### Apple

- Linked devices now show clearer app, OS, and device labels where available.
- Messages to a newly restored linked device now wait for its device keys and retry automatically.

### Zapstore

- Linked devices now show clearer app, OS, and device labels where available.
- Messages to a newly restored linked device now wait for its device keys and retry automatically.

## v2026.5.16.3

### GitHub

- Restoring with a secret key after Delete all local data no longer gets stuck on a storage error.
- Logout and Delete all local data now make sure secret keys are cleared before app data is removed.
- Old messages are no longer skipped just because they are old or far back in history.
- Linked devices are less likely to receive messages for a stale phone session after logout or reset.

### Apple

- Restoring with a secret key after Delete all local data no longer gets stuck on a storage error.
- Logout and Delete all local data now make sure secret keys are cleared before app data is removed.
- Old messages are no longer skipped just because they are old or far back in history.
- Linked devices are less likely to receive messages for a stale phone session after logout or reset.

### Zapstore

- Restoring with a secret key after Delete all local data no longer gets stuck on a storage error.
- Logout and Delete all local data now make sure secret keys are cleared before app data is removed.
- Old messages are no longer skipped just because they are old or far back in history.
- Linked devices are less likely to receive messages for a stale phone session after logout or reset.

## v2026.5.16.2

### GitHub

- Iris can now check for updates automatically on desktop, and self-installed Android APKs can download and install updates from Settings.
- New Chat now uses the same clean code sheet for showing and scanning codes.
- Group creation is simpler: paste or type a user ID and it is added to the member list automatically.
- Nearby rows now show fresher mailbag status and open the right peer flow when tapped.
- iOS image albums now keep the fourth tile and + count aligned when a message has more than four images.

### Apple

- Iris can now check for updates automatically on desktop, and self-installed Android APKs can download and install updates from Settings.
- New Chat now uses the same clean code sheet for showing and scanning codes.
- Group creation is simpler: paste or type a user ID and it is added to the member list automatically.
- Nearby rows now show fresher mailbag status and open the right peer flow when tapped.
- iOS image albums now keep the fourth tile and + count aligned when a message has more than four images.

### Zapstore

- Iris can now check for updates automatically on desktop, and self-installed Android APKs can download and install updates from Settings.
- New Chat now uses the same clean code sheet for showing and scanning codes.
- Group creation is simpler: paste or type a user ID and it is added to the member list automatically.
- Nearby rows now show fresher mailbag status and open the right peer flow when tapped.
- iOS image albums now keep the fourth tile and + count aligned when a message has more than four images.

## v2026.5.16.1

### GitHub

- Messages with multiple images now use Signal-style album layouts: a side-by-side pair, a 1+2 mosaic for three, a 2x2 grid for four, and a +N overlay for albums larger than four.
- Tapping any image opens a swipe-through carousel with the sender name, date, share, and forward actions; swipe down or up to dismiss, and adjacent images preload so navigation stays smooth.
- The composer's staged attachment row now shows a small thumbnail for image attachments instead of a generic filename chip.
- The "Uploading attachment" bar now fills in real time as chunks land on the network instead of running as an indeterminate stripe.

### Apple

- Messages with multiple images now use Signal-style album layouts: a side-by-side pair, a 1+2 mosaic for three, a 2×2 grid for four, and a +N overlay for albums larger than four.
- Tapping any image opens a swipe-through carousel with the sender name, date, share, and forward actions; swipe down or up to dismiss, and adjacent images preload so navigation stays smooth.
- The composer's staged attachment row now shows a small thumbnail for image attachments instead of a generic filename chip.
- The "Uploading attachment" bar now fills in real time as chunks land on the network instead of running as an indeterminate stripe.

### Zapstore

- Messages with multiple images now use Signal-style album layouts: a side-by-side pair, a 1+2 mosaic for three, a 2×2 grid for four, and a +N overlay for albums larger than four.
- Tapping any image opens a swipe-through carousel with the sender name, date, share, and forward actions; swipe down or up to dismiss, and adjacent images preload so navigation stays smooth.
- The composer's staged attachment row now shows a small thumbnail for image attachments instead of a generic filename chip.
- The "Uploading attachment" bar now fills in real time as chunks land on the network instead of running as an indeterminate stripe.

## v2026.5.15.3

### GitHub

- Settings now have a single "Nearby" toggle that hides the chat-list shortcut and turns Bluetooth and Wi-Fi off in one move; turn it back on to keep using nearby messaging.
- Settings now have an "Accept chat requests" toggle on Android, Linux, and Windows; turning it off drops messages and invite responses from people you have not chatted with before.
- Group member rows are now tappable on every platform and open a 1:1 chat with that member.
- macOS message bubbles hug their side of the chat instead of drifting into the middle, and the in-bubble timestamp + delivery glyph trail-align consistently for incoming and outgoing messages on iOS, macOS, and Windows.
- macOS message hover dock is less crowded: Forward moved into the three-dot menu next to Copy, Info, and Delete.
- Nearby modal now shows a small "Mailbag: N yours, M from others" line under each Bluetooth and Wi-Fi row so you can see what is queued for nearby relay.
- Message info "Transport" rows now name the nearby peer that relayed the event, for example "bluetooth: Alice".
- Windows message info now matches the other platforms: per-recipient delivery, transport channels, queued device targets, network event ids.
- Local development builds finally show the real app version on the About screen instead of "0.1.0".

### Apple

- Settings now have a single "Nearby" toggle that hides the chat-list shortcut and turns Bluetooth and Wi-Fi off in one move; turn it back on to keep using nearby messaging.
- Settings now have an "Accept chat requests" toggle on Android, Linux, and Windows; turning it off drops messages and invite responses from people you have not chatted with before.
- Group member rows are now tappable on every platform and open a 1:1 chat with that member.
- macOS message bubbles hug their side of the chat instead of drifting into the middle, and the in-bubble timestamp + delivery glyph trail-align consistently for incoming and outgoing messages on iOS, macOS, and Windows.
- macOS message hover dock is less crowded — Forward moved into the three-dot menu next to Copy, Info, and Delete.
- Nearby modal now shows a small "Mailbag · N yours · M from others" line under each Bluetooth and Wi-Fi row so you can see what is queued for nearby relay.
- Message info "Transport" rows now name the nearby peer that relayed the event (for example "bluetooth · Alice").
- Windows message info now matches the other platforms: per-recipient delivery, transport channels, queued device targets, network event ids.
- Local development builds finally show the real app version on the About screen instead of "0.1.0".

### Zapstore

- Settings now have a single "Nearby" toggle that hides the chat-list shortcut and turns Bluetooth and Wi-Fi off in one move; turn it back on to keep using nearby messaging.
- Settings now have an "Accept chat requests" toggle on Android, Linux, and Windows; turning it off drops messages and invite responses from people you have not chatted with before.
- Group member rows are now tappable on every platform and open a 1:1 chat with that member.
- macOS message bubbles hug their side of the chat instead of drifting into the middle, and the in-bubble timestamp + delivery glyph trail-align consistently for incoming and outgoing messages on iOS, macOS, and Windows.
- macOS message hover dock is less crowded — Forward moved into the three-dot menu next to Copy, Info, and Delete.
- Nearby modal now shows a small "Mailbag · N yours · M from others" line under each Bluetooth and Wi-Fi row so you can see what is queued for nearby relay.
- Message info "Transport" rows now name the nearby peer that relayed the event (for example "bluetooth · Alice").
- Windows message info now matches the other platforms: per-recipient delivery, transport channels, queued device targets, network event ids.
- Local development builds finally show the real app version on the About screen instead of "0.1.0".

## v2026.5.15.2

### GitHub

- Invite and profile QR links now open through chat.iris.to so they work in the web app when the native app is not installed.
- iOS and Android now handle chat.iris.to links directly when installed.
- The web privacy, terms, and child safety pages now open as plain pages instead of redirecting into the app.

### Apple

- Invite and profile QR links now open through chat.iris.to so they work in the web app when the native app is not installed.
- iOS and Android now handle chat.iris.to links directly when installed.
- The web privacy, terms, and child safety pages now open as plain pages instead of redirecting into the app.

### Zapstore

- Invite and profile QR links now open through chat.iris.to so they work in the web app when the native app is not installed.
- iOS and Android now handle chat.iris.to links directly when installed.
- The web privacy, terms, and child safety pages now open as plain pages instead of redirecting into the app.

## v2026.5.15.1

### GitHub

- iOS Settings now includes Privacy, Terms, Child Safety, and Contact links for App Store review.
- Direct chat profiles now include a report action alongside block.
- Account data now separates Delete profile from Delete all local data. Delete profile clears the public profile first.

### Apple

- iOS Settings now includes Privacy, Terms, Child Safety, and Contact links for App Store review.
- Direct chat profiles now include a report action alongside block.
- Account data now separates Delete profile from Delete all local data. Delete profile clears the public profile first.

### Zapstore

- iOS Settings now includes Privacy, Terms, Child Safety, and Contact links for App Store review.
- Direct chat profiles now include a report action alongside block.
- Account data now separates Delete profile from Delete all local data. Delete profile clears the public profile first.

## v2026.5.15

### GitHub

- Settings now has Devices as its own page, and profile QR codes only open when you tap for them.
- Chat screens are closer to Signal, with better headers, message spacing, day labels, reactions, drafts, and composer behavior.
- The iOS new chat button is easier to tap reliably.
- The iOS chat search field now keeps the right dark color without custom rounded styling.
- Profile photos, QR sharing, image previews, and share sheets now feel cleaner across mobile.
- Blocking users, linked devices, and group chats are steadier, with more crash and error recovery fixes.

### Apple

- Settings now has Devices as its own page, and profile QR codes only open when you tap for them.
- Chat screens are closer to Signal, with better headers, message spacing, day labels, reactions, drafts, and composer behavior.
- The iOS new chat button is easier to tap reliably.
- The iOS chat search field now keeps the right dark color without custom rounded styling.
- Profile photos, QR sharing, image previews, and share sheets now feel cleaner across mobile.
- Blocking users, linked devices, and group chats are steadier, with more crash and error recovery fixes.

### Zapstore

- Settings now has Devices as its own page, and profile QR codes only open when you tap for them.
- Chat screens are closer to Signal, with better headers, message spacing, day labels, reactions, drafts, and composer behavior.
- The iOS new chat button is easier to tap reliably.
- The iOS chat search field now keeps the right dark color without custom rounded styling.
- Profile photos, QR sharing, image previews, and share sheets now feel cleaner across mobile.
- Blocking users, linked devices, and group chats are steadier, with more crash and error recovery fixes.

## v2026.5.14.1

### GitHub

- New chats now appear when a new sender messages you for the first time, without needing to search for that user first.
- This device can now block new chats from unknown users.

### Apple

- New chats now appear when a new sender messages you for the first time, without needing to search for that user first.
- This device can now block new chats from unknown users.

### Zapstore

- New chats now appear when a new sender messages you for the first time, without needing to search for that user first.
- This device can now block new chats from unknown users.

## v2026.5.13.6

### GitHub

- iOS message bubbles no longer steal fast vertical flicks from the chat timeline.
- iOS message swipe gestures still open reply and message info, and chat-list row swipes still show row actions.
- Jump to latest now stops in-flight timeline momentum before scrolling, avoiding temporary scroll lock near the bottom.
- The jump-to-latest caret now responds on first touch even while the timeline is still coasting.

### Apple

- iOS message bubbles no longer steal fast vertical flicks from the chat timeline.
- iOS message swipe gestures still open reply and message info, and chat-list row swipes still show row actions.
- Jump to latest now stops in-flight timeline momentum before scrolling, avoiding temporary scroll lock near the bottom.
- The jump-to-latest caret now responds on first touch even while the timeline is still coasting.

### Zapstore

- iOS message bubbles no longer steal fast vertical flicks from the chat timeline.
- iOS message swipe gestures still open reply and message info, and chat-list row swipes still show row actions.
- Jump to latest now stops in-flight timeline momentum before scrolling, avoiding temporary scroll lock near the bottom.
- The jump-to-latest caret now responds on first touch even while the timeline is still coasting.

## v2026.5.13.5

### GitHub

- Long chats no longer flicker on open or briefly lock scrolling after you scroll away from the latest message.
- Opening or paging long chats no longer waits on slow message-server work before the UI can respond.
- Live message subscriptions now finish reliably after reconnects, fixing missed group and linked-device updates.
- Bluetooth nearby presence now stays visible even when the same device is also reachable over Wi-Fi.
- Wi-Fi and Bluetooth nearby handshakes now keep liveness traffic small while avoiding duplicate bulk sync work.
- Release checks now include a local core LAN discovery smoke test.
- Navigation now updates immediately across shells while Rust remains the source of truth, so protocol backlog cannot make chat taps look dead.
- Rust now services user actions ahead of relay/nearby backlog and chunks catch-up processing to keep the app responsive.
- Nearby frame work moved off the iOS main thread and repeated peer updates are deduplicated more aggressively.
- iOS protocol catch-up now coalesces repeated fetches, reducing relay CPU churn and phone heating.
- Duplicate invite events no longer rebuild expensive debug snapshots while replaying queued sends.
- Queued protocol fetches now run single-flight with bounded retry timing instead of overlapping relay requests.
- Group and linked-device recovery still subscribes to your own keys while avoiding useless repeated backfill.

### Apple

- Long chats no longer flicker on open or briefly lock scrolling after you scroll away from the latest message.
- Opening or paging long chats no longer waits on slow message-server work before the UI can respond.
- Live message subscriptions now finish reliably after reconnects, fixing missed group and linked-device updates.
- Bluetooth nearby presence now stays visible even when the same device is also reachable over Wi-Fi.
- Wi-Fi and Bluetooth nearby handshakes now keep liveness traffic small while avoiding duplicate bulk sync work.
- Release checks now include a local core LAN discovery smoke test.
- Navigation now updates immediately across shells while Rust remains the source of truth, so protocol backlog cannot make chat taps look dead.
- Rust now services user actions ahead of relay/nearby backlog and chunks catch-up processing to keep the app responsive.
- Nearby frame work moved off the iOS main thread and repeated peer updates are deduplicated more aggressively.
- iOS protocol catch-up now coalesces repeated fetches, reducing relay CPU churn and phone heating.
- Duplicate invite events no longer rebuild expensive debug snapshots while replaying queued sends.
- Queued protocol fetches now run single-flight with bounded retry timing instead of overlapping relay requests.
- Group and linked-device recovery still subscribes to your own keys while avoiding useless repeated backfill.

### Zapstore

- Long chats no longer flicker on open or briefly lock scrolling after you scroll away from the latest message.
- Opening or paging long chats no longer waits on slow message-server work before the UI can respond.
- Live message subscriptions now finish reliably after reconnects, fixing missed group and linked-device updates.
- Bluetooth nearby presence now stays visible even when the same device is also reachable over Wi-Fi.
- Wi-Fi and Bluetooth nearby handshakes now keep liveness traffic small while avoiding duplicate bulk sync work.
- Release checks now include a local core LAN discovery smoke test.
- Navigation now updates immediately across shells while Rust remains the source of truth, so protocol backlog cannot make chat taps look dead.
- Rust now services user actions ahead of relay/nearby backlog and chunks catch-up processing to keep the app responsive.
- Nearby frame work moved off the iOS main thread and repeated peer updates are deduplicated more aggressively.
- iOS protocol catch-up now coalesces repeated fetches, reducing relay CPU churn and phone heating.
- Duplicate invite events no longer rebuild expensive debug snapshots while replaying queued sends.
- Queued protocol fetches now run single-flight with bounded retry timing instead of overlapping relay requests.
- Group and linked-device recovery still subscribes to your own keys while avoiding useless repeated backfill.

## v2026.5.13.4

### GitHub

- iOS share sends now queue from the share sheet instead of depending on Iris opening right away.
- iOS shared files are copied into Iris before sending, fixing missing attachments after sharing.
- iOS back navigation now stays on the chat list without briefly reopening the previous chat.
- Android chat navigation now ignores stale chat snapshots while the app catches up.
- Linked devices now restore correctly after restart instead of getting stuck waiting for approval.
- iOS composer taps now focus reliably on the first tap.
- iOS composer send button now aligns with the message input.
- macOS composer no longer shows a send button; Return sends and Shift-Return keeps multiline drafting.
- Nearby permission checks no longer poll from render paths, reducing CPU waste.
- Nearby sync now avoids repeated request/response broadcasts, reducing iOS idle CPU while Bluetooth and Wi-Fi discovery are active.
- Debug logging is off by default in release builds and can be enabled from Settings when exporting a debug dump.

### Apple

- iOS share sends now queue from the share sheet instead of depending on Iris opening right away.
- iOS shared files are copied into Iris before sending, fixing missing attachments after sharing.
- iOS back navigation now stays on the chat list without briefly reopening the previous chat.
- Android chat navigation now ignores stale chat snapshots while the app catches up.

- Linked devices now restore correctly after restart instead of getting stuck waiting for approval.
- iOS composer taps now focus reliably on the first tap.
- iOS composer send button now aligns with the message input.
- macOS composer no longer shows a send button; Return sends and Shift-Return keeps multiline drafting.
- Nearby permission checks no longer poll from render paths, reducing CPU waste.
- Nearby sync now avoids repeated request/response broadcasts, reducing iOS idle CPU while Bluetooth and Wi-Fi discovery are active.
- Debug logging is off by default in release builds and can be enabled from Settings when exporting a debug dump.

### Zapstore

- iOS share sends now queue from the share sheet instead of depending on Iris opening right away.
- iOS shared files are copied into Iris before sending, fixing missing attachments after sharing.
- iOS back navigation now stays on the chat list without briefly reopening the previous chat.
- Android chat navigation now ignores stale chat snapshots while the app catches up.

- Linked devices now restore correctly after restart instead of getting stuck waiting for approval.
- iOS composer taps now focus reliably on the first tap.
- iOS composer send button now aligns with the message input.
- macOS composer no longer shows a send button; Return sends and Shift-Return keeps multiline drafting.
- Nearby permission checks no longer poll from render paths, reducing CPU waste.
- Nearby sync now avoids repeated request/response broadcasts, reducing iOS idle CPU while Bluetooth and Wi-Fi discovery are active.
- Debug logging is off by default in release builds and can be enabled from Settings when exporting a debug dump.

## v2026.7.15

### GitHub

- Linked devices now recover chats and recent messages reliably across packet loss and reconnects.
- Delivery and seen indicators continue to reflect what recipient apps actually received and opened.

### Apple

- Linked devices now recover chats and recent messages reliably across packet loss and reconnects.
- Delivery and seen indicators continue to reflect what recipient apps actually received and opened.

### Zapstore

- Linked devices now recover chats and recent messages reliably across packet loss and reconnects.
- Delivery and seen indicators continue to reflect what recipient apps actually received and opened.

## v2026.7.14

### GitHub

- A newly linked device now receives your chat list and group details automatically.
- Linked devices share known device keys for direct chats and group members.
- Recent messages after the latest device change can follow to the newly linked device.

### Apple

- A newly linked device now receives your chat list and group details automatically.
- Linked devices share known device keys for direct chats and group members.
- Recent messages after the latest device change can follow to the newly linked device.

### Zapstore

- A newly linked device now receives your chat list and group details automatically.
- Linked devices share known device keys for direct chats and group members.
- Recent messages after the latest device change can follow to the newly linked device.

## v2026.7.13

### GitHub

- App updates now come from a dedicated Iris release channel.
- Linked devices now keep receiving group messages after another member restores the app.
- Desktop release checks now catch more idle resource-use and Windows startup problems before shipping.

### Apple

- App updates now come from a dedicated Iris release channel.
- Linked devices now keep receiving group messages after another member restores the app.
- Desktop release checks now catch more idle resource-use and Windows startup problems before shipping.

### Zapstore

- App updates now come from a dedicated Iris release channel.
- Linked devices now keep receiving group messages after another member restores the app.
- Desktop release checks now catch more idle resource-use and Windows startup problems before shipping.

## v2026.7.12

### GitHub

- Device linking now uses signed approval requests for more reliable setup across relays.
- Direct and group chats recover secure messaging readiness more reliably after reconnecting.
- Group key recovery avoids redundant responses while preserving delayed message delivery.

### Apple

- Device linking now uses signed approval requests for more reliable setup across relays.
- Direct and group chats recover secure messaging readiness more reliably after reconnecting.
- Group key recovery avoids redundant responses while preserving delayed message delivery.

### Zapstore

- Device linking now uses signed approval requests for more reliable setup across relays.
- Direct and group chats recover secure messaging readiness more reliably after reconnecting.
- Group key recovery avoids redundant responses while preserving delayed message delivery.

## v2026.7.6

### GitHub

- One-to-one messages now queue cleanly while secure chat setup finishes, then send once the conversation is ready.
- Updated the encrypted messaging library to the latest release.
- Release checks now cover the current Android APK and internal iOS TestFlight upload flow.

### Apple

- One-to-one messages now queue cleanly while secure chat setup finishes, then send once the conversation is ready.
- Updated the encrypted messaging library to the latest release.
- Release checks now cover the current Android APK and internal iOS TestFlight upload flow.

### Zapstore

- One-to-one messages now queue cleanly while secure chat setup finishes, then send once the conversation is ready.
- Updated the encrypted messaging library to the latest release.
- Release checks now cover the current Android APK and internal iOS TestFlight upload flow.

## v2026.7.1

### GitHub

- Newly created groups now exchange messages reliably between web and native linked devices.
- Device linking now keeps browser, OS, and app labels visible across web, iOS, and desktop.
- Restored profiles open faster while message recovery continues in the background.

### Apple

- Newly created groups now exchange messages reliably between web and native linked devices.
- Device linking now keeps browser, OS, and app labels visible across web, iOS, and desktop.
- Restored profiles open faster while message recovery continues in the background.

### Zapstore

- Newly created groups now exchange messages reliably between web and native linked devices.
- Device linking now keeps browser, OS, and app labels visible across web, iOS, and desktop.
- Restored profiles open faster while message recovery continues in the background.

## v2026.6.5

### GitHub

- Message requests now show Accept, Block, and Block and report actions directly, with Delete chat and Unblock available from the safety flow.
- Messages recover more reliably after phones or linked devices were closed, restarted, or offline.
- Direct and group chats now have broader real-device coverage across Android phones and iOS simulators.
- Restoring an existing profile with a secret key is tested across iOS and Android.
- Multi-device accounts now sync direct and group messages more reliably.
### Apple

- Message requests now show Accept, Block, and Block and report actions directly, with Delete chat and Unblock available from the safety flow.
- Messages recover more reliably after phones or linked devices were closed, restarted, or offline.
- Direct and group chats now have broader real-device coverage across Android phones and iOS simulators.
- Restoring an existing profile with a secret key is tested across iOS and Android.
- Multi-device accounts now sync direct and group messages more reliably.

### Zapstore

- Message requests now show Accept, Block, and Block and report actions directly, with Delete chat and Unblock available from the safety flow.
- Messages recover more reliably after phones or linked devices were closed, restarted, or offline.
- Direct and group chats now have broader real-device coverage across Android phones and iOS simulators.
- Restoring an existing profile with a secret key is tested across iOS and Android.
- Multi-device accounts now sync direct and group messages more reliably.
