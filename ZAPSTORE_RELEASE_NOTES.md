# Iris Chat 2026.5.12.1

- Searches no longer redo message-index work just because the app state refreshed, which should keep busy chat lists and in-chat search calmer on phones.
- In-chat search can now load more matching messages instead of stopping at the first batch.
- Android share checkboxes stay selected while the chat list refreshes, so sending shared content to multiple chats is less fragile.
