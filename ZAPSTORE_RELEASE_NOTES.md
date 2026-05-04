# Iris Chat 3.0.13

- Long-press a message on Android and iOS to open a redesigned action sheet with quick reactions, message preview (including existing reactions), and iconified Reply / Copy / Message info / Delete actions; the tap-to-show three-dot toolbar is gone on mobile.
- Message info now shows participant avatars and display names instead of hex pubkeys, drops the redundant direction subtitle, renames the header copy button to "Copy info", strips the "message server:" prefix from transport entries, and labels nearby transport as "wifi" or "bluetooth" instead of "nearby offered".
- Adds message info views for inspecting message status, recipients, transport IDs, attachments, and reactions.
- Shows device add dates in the linked-device roster.
- Uses nostr-double-ratchet 0.0.136 with private one-use invite links.
