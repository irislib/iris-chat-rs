# Iris Chat 2026.5.11.5

- Signal-style search across the chat list: tap the field at the top and grouped Contacts / Groups / Messages results appear, with the messages section backed by a real SQLite FTS5 index so it's fast even on long histories. Pasting an npub or invite URL into the search field auto-opens the chat or accepts the invite — no extra tap.
- Search-in-chat: the magnifying-glass icon in a chat or group header opens a sheet that searches only that conversation's messages.
- iOS: smoother send animation — the message log no longer flickers when you fire off a quick message. The chat timeline used to issue a dozen scroll-to-bottom calls per send; it now coalesces to one.
