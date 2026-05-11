# Iris Chat 2026.5.11.7

- Drafts now persist: leave a half-typed message in any chat and it'll still be there after backgrounding, navigating away, or relaunching the app. The chat list shows "Draft: …" for any thread with unsent text. Matches Signal's behaviour.
- Tapping a search result lands you on that message in the chat instead of just opening the conversation at the bottom.
- No more purple text. "Show more", swipe-actions, member-admin toggles, search shortcut icons, and the like all read in the regular text colour now — brand purple is reserved for surfaces (button backgrounds, your own bubble). A release-time lint blocks new purple text/icons from sneaking in.
- Heat fix on iOS: the chat-list search was re-running the message index on every incoming message / typing ping. Cache it per query change so a busy conversation doesn't warm the phone.
- Tap-and-hold to copy version numbers, npubs, hex pubkeys, and other identifiers on the settings / chat-info / message-details pages.
- Notification tap opens the chat immediately instead of sitting on "Loading chat…" (carried from the prior in-flight release).
- iOS message bubbles get their breathing room back between consecutive messages (carried from the prior in-flight release).
