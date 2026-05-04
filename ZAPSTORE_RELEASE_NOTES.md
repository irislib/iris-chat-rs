# Iris Chat 3.0.15

- Adds an `iris update {check, download, install}` subcommand for self-updating the CLI from hashtree releases.
- Routes legacy group sender-key events through the new wire format so older peers and snapshot consumers stay in sync.
- Caps Android chat bubble width at 300dp for cleaner long-message layout.
- Updates to nostr-double-ratchet 0.0.138 with snapshot-only group metadata.
