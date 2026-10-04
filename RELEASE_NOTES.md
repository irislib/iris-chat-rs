# Iris Chat Release Notes

Each release has channel-specific notes. The release tag must match the `##` heading exactly.

## v2026.10.4.1

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
