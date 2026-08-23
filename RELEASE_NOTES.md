# Iris Chat Release Notes

Each release has channel-specific notes. The release tag must match the `##` heading exactly.

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
