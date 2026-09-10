# Native mesh connections

The native core uses one FIPS endpoint for chat events, linked-device sync,
nearby discovery, and attachment reads. Chat events use the existing signed,
encrypted message format and recipient authorization. An intermediate FIPS
peer can route traffic without joining the conversation or subscribing to
its events.

Known contact and sibling-device identities supply a bounded peer roster.
Delivery still requires a physical path into the mesh. Identity hints alone
do not discover unknown providers, establish a physical connection, or grant
conversation access. This integration covers native clients; browser clients
need a browser-supported transport and signaling path.

For an explicitly configured native mesh, set these variables before starting
the app:

| Variable | Purpose |
| --- | --- |
| `IRIS_CHAT_FIPS_STATIC_PEERS` | Comma- or semicolon-separated `npub=udp:host:port` physical peers; addresses must be numeric socket addresses. |
| `IRIS_CHAT_FIPS_ROUTED_PEERS` | Comma- or semicolon-separated peer npubs reachable through FIPS routing. |
| `IRIS_CHAT_FIPS_TRUSTED_RATERS` | Optional comma- or semicolon-separated public keys in npub or hex form whose signed machine ratings inform mesh preference. Empty by default. |
| `IRIS_CHAT_FIPS_UDP_BIND_ADDR` | Local UDP socket address, such as `127.0.0.1:0` for an isolated local test. |
| `IRIS_FIPS_WEBSOCKET_SEED_URLS` | WebSocket seed URLs; an explicitly empty value disables the default public seeds. |

Message servers and Nearby LAN discovery are separate account preferences.
A test without public bootstrap must also clear the account's message servers
and disable LAN discovery. The stack fixture reports the effective relay count,
LAN state, direct peers, and pubsub peers so tests can verify those conditions.

Mesh preference combines local FIPS observations with ratings from the explicitly
configured entrypoints. One shared adapter manages the bounded rating exchange
and uses the same projection for peer preference and event admission. A positive
service rating does not authorize its subject to rate other peers. No personal
identity is selected automatically, and personal follow lists are not imported.
These preferences confer no conversation or device-linking permission. Restart
the app after changing the environment configuration.

The durable chat outbox owns retries after a partition. The shared pubsub cache
is bounded; explicit retries restore evicted events in fair batches. Existing
authenticated Delivered or Seen receipts stop message retries while optional
message-server persistence retains its records.
