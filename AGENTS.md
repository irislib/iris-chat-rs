UI copy: concise, normie/granny-friendly. Prefer "Link this device", "secret key", "user ID", "message server"; avoid filler explainers and protocol jargon (`npub`, `nsec`, owner, roster, relay, hex, QR) unless essential.

Private communication must preserve metadata privacy as well as forward secrecy.
Private relay publishing must use separate send-only connections with fresh
anonymous authentication. Never reuse account-authenticated or receiving
connections, including on retries or authentication failure.
The same boundary applies to FIPS providers and nearby forwarding: a provider
must not learn a public sender identity together with a recipient tag or a
peer-discovery filter. Payload encryption alone does not hide those fields.
User-to-user relay traffic must use double-ratchet sessions and their invite
handshakes. Never reveal the identities of both participants through event
signers, tags, notification filters, authentication, or correlations between them.
Prefer invite/session keys over identity keys. A recipient device tag is allowed
when necessary for delivery only if the sender uses a session key unlinkable to
its identity; invite responses target the invite's ephemeral key. Keep caller
and contact authorization on device, after authenticated decryption. FIPS remains
available for live traffic. Delivery and cold-start fixes must not weaken this
boundary; do not retain an identity-leaking compatibility fallback.
Shared group ciphertext and signed rosters are local intermediates, never public
network records. Every group message, membership update, retry and recovery
delivery must travel inside recipient-specific ratchet envelopes. Never open
shared group-author or group-roster subscriptions on any network carrier.
Changes to these paths require wire-level tests that inspect serialized events
and subscription requests for identity leakage, including first contact,
restarts, retries, calls, and group key distribution. Successful encryption and
decryption alone do not establish metadata privacy.

For app releases, add the exact tagged GitHub, Apple, and Zapstore sections to
`RELEASE_NOTES.md`. Follow `RELEASE.md`; release binaries are built only by the
tagged GitHub workflow and are promoted unchanged to every channel.
