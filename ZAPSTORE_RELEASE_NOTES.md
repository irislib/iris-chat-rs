# Iris Chat 2.6.23

Interop reliability release for the native Rust chat app.

- Dedupe browser-origin NDR messages by inner rumor ID instead of relay fanout IDs.
- Keeps typing, receipt, seen, and reaction controls out of stored chat history.
- Keeps disappearing-message and group metadata changes visible as system history.
- Adds exact-count Android/browser interop coverage for duplicate regressions.
