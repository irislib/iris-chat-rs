# Iris Chat 2026.7.12

- Device linking now uses signed approval requests for more reliable setup across relays.
- Direct and group chats recover secure messaging readiness more reliably after reconnecting.
- Group key recovery avoids redundant responses while preserving delayed message delivery.

# Iris Chat 2026.7.6

- One-to-one messages now queue cleanly while secure chat setup finishes, then send once the conversation is ready.
- Updated the encrypted messaging library to the latest release.
- Release checks now cover the current Android APK and internal iOS TestFlight upload flow.

# Iris Chat 2026.7.1

- Newly created groups now exchange messages reliably between web and native linked devices.
- Device linking now keeps browser, OS, and app labels visible across web, iOS, and desktop.
- Restored profiles open faster while message recovery continues in the background.

# Iris Chat 2026.6.30

- Group membership now syncs through shared roster fact snapshots across web and native apps.
- Linked-device authorization now comes from owner-signed kind 37368 AppKeys snapshots.
- Release checks cover direct chats, linked devices, and groups across web and native apps.

# Iris Chat 2026.6.29

- Linking a device is more reliable across app restarts and fresh installs.
- Device approvals now use the shared AppKeys roster, keeping new linked devices in sync.
- Internal roster handling was split into smaller pieces so release checks catch regressions cleanly.

# Iris Chat 2026.6.9

- Messages recover more reliably after restart, offline use, or message-server reconnects.
- Startup no longer waits on message-server status before chat recovery can continue.
- Linked devices and queued messages retry missing chat state more reliably.

# Iris Chat 2026.6.5

- Message requests now show Accept, Block, and Block and report actions directly, with Delete chat and Unblock available from the safety flow.
- Messages recover more reliably after phones or linked devices were closed, restarted, or offline.
- Direct and group chats now have broader real-device coverage across Android phones and iOS simulators.
- Restoring an existing profile with a secret key is tested across iOS and Android.
- Multi-device accounts now sync direct and group messages more reliably.

# Iris Chat 2026.6.3

- Messages recover more reliably after the app was closed, restarted, or offline.
- Group messages and linked devices retry missing keys instead of getting stuck.
- Restoring an existing profile with a secret key is covered by broader phone and simulator tests.
- Desktop builds and release tests now cover more real app journeys.

# Iris Chat 2026.5.29

- iOS notifications stay off by default until turned on in Settings.
- Blocked message requests stay open for review and disappear from the chat list after you leave.
- Typing indicators are on by default.

# Iris Chat 2026.5.27

- Onboarding now asks people to agree to Terms before creating, restoring, or linking a profile.
- Welcome screens, app icons, splash art, and notification icons are cleaner and more consistent.
- Pending outgoing messages now use a send icon, keeping the clock/timer icon for disappearing messages.
- Linux chats now include link actions.
- Split oversized iOS Swift UI files and added a repo-wide source file size ratchet.

# Iris Chat 2026.5.23.1

- Messages reveal less delivery metadata to message servers.
- Group message recovery still works with older app versions.
- Message repair requests avoid sharing hidden delivery counters.

# Iris Chat 2026.5.20.2

- Chats now fetch missing profile details when needed, so names and photos appear more reliably.
- Desktop notifications now work more consistently after switching away from Iris.
- Settings no longer show secret device-key copy/export actions.
- Chat-list profile avatars feel cleaner when tapped.

# Iris Chat 2026.5.20.1

- Group messages recover more reliably after app restarts and missed key updates.
- New chats with known linked devices get unstuck more often.
- Recovery retries are quieter and survive restart.

# Iris Chat 2026.5.18.6

- Foreground stays responsive during catch-up bursts and large group metadata updates.

# Iris Chat 2026.5.18.5

- Linked devices now learn remote-created groups after restart.
- Group messages recover more reliably after app restore.
- Android release checks now rebuild Rust path dependencies when shared protocol code changes.
- Android storage avoids a native SQLite crash seen during relay publishing.

# Iris Chat 2026.5.18.4

- Nearby profiles now open as profiles instead of being mistaken for chats.
- Profile nickname editing no longer shows a placeholder nickname as saved data.
- Desktop message actions sit beside bubbles more neatly.
- Idle sync retries use less CPU.
- macOS release builds find the shared Cargo build directory more reliably.

# Iris Chat 2026.5.18.2

- Adding people to groups now asks for confirmation before sending invites.
- Linked device names can be renamed from Devices.
- Nearby now shows cleaner chat-list shortcuts, opens chats from nearby avatars, and appears in mobile sharing.
- Removed linked devices now stay removed more reliably.

# Iris Chat 2026.5.18.1

- Group photos now persist and appear in chats, chat lists, and group details.

# Iris Chat 2026.5.17.1

- Linked devices now show clearer app, OS, and device labels where available.
- Messages to a newly restored linked device now wait for its device keys and retry automatically.

# Iris Chat 2026.5.16.3

- Restoring with a secret key after Delete all local data no longer gets stuck on a storage error.
- Logout and Delete all local data now make sure secret keys are cleared before app data is removed.
- Old messages are no longer skipped just because they are old or far back in history.
- Linked devices are less likely to receive messages for a stale phone session after logout or reset.

# Iris Chat 2026.5.16.2

- Iris can now check for updates automatically on desktop, and self-installed Android APKs can download and install updates from Settings.
- New Chat now uses the same clean code sheet for showing and scanning codes.
- Group creation is simpler: paste or type a user ID and it is added to the member list automatically.
- Nearby rows now show fresher mailbag status and open the right peer flow when tapped.
- iOS image albums now keep the fourth tile and + count aligned when a message has more than four images.

# Iris Chat 2026.5.16.1

- Messages with multiple images now use Signal-style album layouts: a side-by-side pair, a 1+2 mosaic for three, a 2×2 grid for four, and a +N overlay for albums larger than four.
- Tapping any image opens a swipe-through carousel with the sender name, date, share, and forward actions; swipe down or up to dismiss, and adjacent images preload so navigation stays smooth.
- The composer's staged attachment row now shows a small thumbnail for image attachments instead of a generic filename chip.
- The "Uploading attachment" bar now fills in real time as chunks land on the network instead of running as an indeterminate stripe.

# Iris Chat 2026.5.15.3

- Settings now have a single "Nearby" toggle that hides the chat-list shortcut and turns Bluetooth and Wi-Fi off in one move; turn it back on to keep using nearby messaging.
- Settings now have an "Accept chat requests" toggle on Android, Linux, and Windows; turning it off drops messages and invite responses from people you have not chatted with before.
- Group member rows are now tappable on every platform and open a 1:1 chat with that member.
- macOS message bubbles hug their side of the chat instead of drifting into the middle, and the in-bubble timestamp + delivery glyph trail-align consistently for incoming and outgoing messages on iOS, macOS, and Windows.
- macOS message hover dock is less crowded — Forward moved into the three-dot menu next to Copy, Info, and Delete.
- Nearby modal now shows a small "Mailbag · N yours · M from others" line under each Bluetooth and Wi-Fi row so you can see what is queued for nearby relay.
- Message info "Transport" rows now name the nearby peer that relayed the event (for example "bluetooth · Alice").
- Windows message info now matches the other platforms: per-recipient delivery, transport channels, queued device targets, network event ids.
- Local development builds finally show the real app version on the About screen instead of "0.1.0".

# Iris Chat 2026.5.15.2

- Invite and profile QR links now open through chat.iris.to so they work in the web app when the native app is not installed.
- iOS and Android now handle chat.iris.to links directly when installed.
- The web privacy, terms, and child safety pages now open as plain pages instead of redirecting into the app.

# Iris Chat 2026.5.15.1

- iOS Settings now includes Privacy, Terms, Child Safety, and Contact links for App Store review.
- Direct chat profiles now include a report action alongside block.
- Account data now separates Delete profile from Delete all local data. Delete profile clears the public profile first.

# Iris Chat 2026.5.15

- Settings now has Devices as its own page, and profile QR codes only open when you tap for them.
- Chat screens are closer to Signal, with better headers, message spacing, day labels, reactions, drafts, and composer behavior.
- The iOS new chat button is easier to tap reliably.
- The iOS chat search field now keeps the right dark color without custom rounded styling.
- Profile photos, QR sharing, image previews, and share sheets now feel cleaner across mobile.
- Blocking users, linked devices, and group chats are steadier, with more crash and error recovery fixes.

# Iris Chat 2026.5.14.1

- New chats now appear when a new sender messages you for the first time, without needing to search for that user first.
- This device can now block new chats from unknown users.

# Iris Chat 2026.5.13.6

- iOS message bubbles no longer steal fast vertical flicks from the chat timeline.
- iOS message swipe gestures still open reply and message info, and chat-list row swipes still show row actions.
- Jump to latest now stops in-flight timeline momentum before scrolling, avoiding temporary scroll lock near the bottom.
- The jump-to-latest caret now responds on first touch even while the timeline is still coasting.

# Iris Chat 2026.5.13.5

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

# Iris Chat 2026.5.13.4

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
