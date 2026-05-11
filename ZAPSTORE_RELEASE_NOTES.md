# Iris Chat 2026.5.11.9

- Big speed-up for anyone with many active chats: the app would slowly chew CPU after extended use and the UI got sluggish when navigating between chats. Traced it to an internal debug snapshot rebuilding on every relay event — now throttled and disabled entirely in release builds.
- Links inside messages render in the regular text colour with a Signal-style underline. The previous orange tint was loud and clashed with both bubble colours; the quieter underline reads cleanly on outgoing and incoming bubbles alike.
- macOS chat bubbles no longer stretch the full pane on wide windows — capped at a comfortable column, like every other desktop messenger.
- Hover-revealed message actions no longer reflow the bubble width when they appear. The action dock floats next to the message instead of pushing it around.
- On macOS, clicking a link in a message works again and you get the pointing-hand cursor over URLs.
- "Show more" / "Show less" toggle no longer renders in brand purple on Android, Windows, or Linux — caught the same no-purple-text rule iOS already followed.
- Expanded validation coverage for sender-key encrypted groups, picking up an upstream contribution from lauri000.
