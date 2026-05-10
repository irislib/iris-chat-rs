# Iris Chat 2026.5.10.1

- Stops every chat list row from re-evaluating on every state push (typing pings, relay events, update checks). On lists of any size this should noticeably reduce CPU/battery while the app is open.
