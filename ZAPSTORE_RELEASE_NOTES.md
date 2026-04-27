# Iris Chat 2.6.26

Performance release. Opening a chat is now near-instant on Android.

- Cut chat-open core-thread time on Android from ~5.5 s to ~50 ms by deduping `setup_user` calls and dropping a dead-code mobile-push session walk that was running `serde_json::to_string` on every ratchet state of every tracked owner on every emit.
- Persist split into per-slice files under `core/`. Tapping a chat now writes ~170 bytes (a small `meta.json`) instead of rewriting a 74 KB monolithic JSON. Per-chat `threads/<id>.json` mean a relay event for one chat only touches that one file.
- Coalesce queued state updates: a flurry of relay events flushed by a relay produces a single UI update, not one per event.
- Skip pushing `FullState` to the UI when nothing user-visible changed.
- Per-slice `StateFlow`s on Android: `ChatScreen` only recomposes when its specific slice (currentChat / preferences / busy / router / chatList) actually changed.
- Drain the FFI listener queue and deliver only the latest snapshot when the core emits a tight burst.
- Throttle DM-subscription resyncs in nostr-double-ratchet (1.5 s trailing) so ratchet steps stop slamming relays with redundant REQs.
