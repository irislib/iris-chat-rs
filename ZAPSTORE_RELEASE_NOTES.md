# Iris Chat 2026.5.11.6

- Notification tap now opens the chat instantly instead of sitting on "Loading chat…" — the running core stubs the thread record + loads its persisted page inline, so `current_chat` is populated on the same render that flips the screen.
- iOS message bubbles get their breathing room back. Consecutive same-author messages had collapsed to a ~4pt gap once the dark theme moved to pure-black panels; bumped to ~8pt so each bubble still reads as its own message without breaking the cluster grouping.
- Drafts now stick. The composer's unsent text is saved per-chat in the local database (Signal-style), so leaving the chat, backgrounding the app, or relaunching all preserve what you were typing. The chat list shows "Draft: …" for any thread with an unsent message.
