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
