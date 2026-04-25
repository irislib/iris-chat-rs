# Iris Chat 2.6.22

Interop reliability release for the native Rust chat app.

- Fixed bidirectional NDR interop with chat.iris.to.
- Fetches recent protocol state immediately after opening or accepting chats.
- Keeps queued setup messages moving to sent after runtime relay ACKs.
- Uses live-updating relative timestamps and shows fresh messages as "now".
- Adds live Android/browser interop coverage against public relays.
