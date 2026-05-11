# Iris Chat 2026.5.11.4

- iOS: fix RUNNINGBOARD 0xdead10cc crash when the app is suspended mid relay event — every queued internal event now drops cleanly during suspend and the SQLite seen-events table updates incrementally instead of being rewritten on every message (one INSERT instead of 2048).
- Instant chat-screen flip: tapping a chat now flips the screen immediately and defers the message-page load to a follow-up event, so tap-to-back from a freshly opened chat no longer sits behind the cold-chat load.
- Back navigation goes through a single `NavigateBack` intent end-to-end: the rust core owns the screen-stack pop instead of every platform computing it. Cleaner and consistent across iOS / Android / Linux.
