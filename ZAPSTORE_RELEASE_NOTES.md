# Iris Chat 2026.5.11.8

- macOS chat bubbles no longer stretch the full pane on wide windows — capped at a comfortable column, like every other desktop messenger.
- Hover-revealed message actions no longer reflow the bubble width when they appear. The action dock floats next to the message instead of pushing it around.
- Links inside messages now use the same colour on outgoing and incoming bubbles, so the same URL doesn't toggle between two "this is clickable" cues when quoted back. On macOS clicking a link works again and you get the pointing-hand cursor over URLs.
- "Show more" / "Show less" toggle no longer renders in brand purple on Android, Windows, or Linux — caught the same no-purple-text rule iOS already followed.
- Expanded validation coverage for sender-key encrypted groups, picking up an upstream contribution from lauri000.
