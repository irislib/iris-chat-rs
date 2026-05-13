# Iris Chat 2026.5.13.5

- Long chats no longer flicker on open or briefly lock scrolling after you scroll away from the latest message.
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
