# Iris Chat 2026.5.13.3

- iOS share sends now queue from the share sheet instead of depending on Iris opening right away.
- iOS shared files are copied into Iris before sending, fixing missing attachments after sharing.
- iOS back navigation now stays on the chat list without briefly reopening the previous chat.
- Android chat navigation now ignores stale chat snapshots while the app catches up.

- Linked devices now restore correctly after restart instead of getting stuck waiting for approval.
- iOS composer taps now focus reliably on the first tap.
- Nearby permission checks no longer poll from render paths, reducing CPU waste.
- Debug logging is off by default in release builds and can be enabled from Settings when exporting a debug dump.
