UI copy: concise, normie/granny-friendly. Prefer "Link this device", "secret key", "user ID", "message server"; avoid filler explainers and protocol jargon (`npub`, `nsec`, owner, roster, relay, hex, QR) unless essential.

Private communication must preserve metadata privacy as well as forward secrecy.
User-to-user relay traffic must use double-ratchet sessions and their invite
handshakes. Never reveal the identities of both participants through event
signers, tags, notification filters, authentication, or correlations between them.
Prefer invite/session keys over identity keys. A recipient device tag is allowed
when necessary for delivery only if the sender uses a session key unlinkable to
its identity; invite responses target the invite's ephemeral key. Keep caller
and contact authorization on device, after authenticated decryption. FIPS remains
available for live traffic. Delivery and cold-start fixes must not weaken this
boundary; do not retain an identity-leaking compatibility fallback.
Changes to these paths require wire-level tests that inspect serialized events
and subscription requests for identity leakage, including first contact,
restarts, retries, calls, and group key distribution. Successful encryption and
decryption alone do not establish metadata privacy.

For app releases, add the exact tagged GitHub, Apple, and Zapstore sections to
`RELEASE_NOTES.md`. Follow `RELEASE.md`; release binaries are built only by the
tagged GitHub workflow and are promoted unchanged to every channel.
