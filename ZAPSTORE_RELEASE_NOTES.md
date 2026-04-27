# Iris Chat 3.0.0

- Storage rewrite: durable state now lives in SQLite. Faster cold starts,
  smaller backups, and a proper foundation for upcoming features.
- Add members to a group by searching your known users — group creation
  and group management both let you find existing direct-chat partners by
  name or user ID instead of pasting a raw npub.
- Full categorized emoji picker on Android message reactions, matching the
  iOS / macOS picker.
- Chat list "typing" no longer sticks after a newer message arrives,
  including the same-second tie that caused "always typing" rows on
  freshly-active chats.
- "Show" label on the invite-QR button so it no longer reads as a second
  scanner next to the actual scan button.
- macOS invite-QR modal closes on click outside (was Escape-only).
- iOS push: typing-indicator previews decode correctly again.
- Faster runtime publish path; typing indicators are now opt-in.
- Composer attachment previews on Linux and Windows.

Carries forward 2.6.33's relay catchup + Android push preview fixes,
2.6.32's icon polish, and the 2.6.31 reliability work.
