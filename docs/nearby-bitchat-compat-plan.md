# Nearby BitChat Compatibility Plan

Date: 2026-04-28

## Current status

As of 2026-07-20, the mobile implementation uses portable FIPS BLE v2 between
Iris clients. It is not BitChat wire-compatible. The unused BitChat codec and
the custom BLE/mDNS/TCP transports have been removed; FIPS owns discovery,
authentication, framing, BLE, and LAN transport.

The shared application scope is `iris-chat-nearby-v1`. FIPS carries that scope
in its generic `_fips._udp` LAN advertisement and in its signed Nostr overlay
advertisement, and ignores advertisements for other scopes. The LAN advert is
only a routing hint; FIPS authenticates the advertised device identity with
Noise before accepting traffic. Platform code only supplies OS integration
required by FIPS: the Apple Bonjour declaration, Android's multicast lock, and
the mobile BLE command adapters.

FIPS BLE advertises a small GATT bootstrap characteristic containing the
platform-assigned L2CAP channel number. GATT is BLE's service-and-characteristic
API; Iris uses it only for discovery/bootstrap. Message data then travels as
length-framed packets over an authenticated FIPS L2CAP connection. Android
support requires Android 10 / API 29 or newer; the app itself still supports
older Android versions with Bluetooth nearby unavailable.

Physical-device proof completed on 2026-07-20 with an iPhone 12 Pro and a
Pixel 10 Pro. The Android phone had airplane mode enabled, Bluetooth restored,
and no message servers configured. The iPhone harness reported a native FIPS
connection with successful reads and writes and zero message servers; the
Android harness received the exact probe message and marked it Seen. Network
settings were restored after the test.

The remainder of this document is historical design context, not the current
implementation plan.

## Goal

Add a first-row "Nearby" entry to the Iris chat list that can discover and
message people on the same local network or over BLE, while staying compatible
with the existing BitChat iOS and Android protocol where possible.

The important product behavior is:

- show nearby people in one predictable place
- deliver messages locally when possible
- establish a nostr-double-ratchet session while nearby
- send the same private chat events directly while nearby, including in
  airplane mode
- continue the same private conversation over Nostr message servers when nearby
  transport or local reachability disappears
- keep unsolicited/random nearby data separate from social-graph data

## MVP Shape

The MVP is a virtual "Nearby" row pinned to the top of the chat list.

Opening it shows:

- a visibility toggle
- nearby people with avatars and names from their Nostr profile
- a concise fallback label when no profile is known yet
- a "Start chat" action for each person

Visibility off:

- do not advertise local presence
- stop accepting new nearby chat starts
- keep existing regular chats available over message servers

Visibility on:

- advertise a small signed nearby presence card
- listen for nearby presence cards from others
- show nearby people in the Nearby screen
- allow other visible nearby clients to start a chat

"Start chat" is the handshake moment. We should not run Noise/NDR handshakes
against every visible person just to render the list. After the handshake, the
nearby connection stays attached to the normal direct chat so new messages are
sent directly to the peer whenever they are reachable locally.

## Recommendation

Use BitChat's packet protocol as the nearby compatibility layer:

- `BitchatPacket` as the outer frame
- `ANNOUNCE` (`0x01`) for nearby presence
- `NOISE_HANDSHAKE` (`0x10`) for local secure-channel setup
- `NOISE_ENCRYPTED` (`0x11`) for encrypted local payloads
- `NoisePayloadType.NDR_EVENT` (`0x12`) for nostr-double-ratchet out-of-band
  invite/response event JSON

Do not invent a second local private-message protocol for Iris. The BitChat
repos already use this shape on iOS and Android, and both already have NDR
out-of-band bootstrap paths.

## Transport

For same-Wi-Fi messaging, start with:

1. mDNS/Bonjour discovery using a service such as `_bitchat._tcp.local`.
2. A length-prefixed TCP stream or WebSocket stream for packet exchange.
3. Exact `BitchatPacket` binary frames over that stream.
4. The same packet processing rules used by BLE.

QUIC is optional later, not required for compatibility. QUIC would add another
crypto/session layer and more mobile dependency work. If QUIC is added later,
it should still carry exact `BitchatPacket` frames and still keep BitChat Noise
at the app layer so existing clients can interoperate.

BLE remains useful for existing BitChat mesh compatibility and for local
discovery. Wi-Fi should be preferred for bulk sync and mailbag transfer because
it has better throughput and simpler reliability.

## Development Strategy

Build the MVP on the macOS app first. This Mac can exercise real BLE hardware,
run the normal Iris UI, and run local test peers without needing phone deploys
for every iteration.

The fastest external reference loop is:

1. Run BitChat Android on a physical Android device first, using a local
   checkout of `bitchat-android` and `./gradlew installDebug`.
2. Use BitChat iOS instead when a real iPhone is already set up for local Xcode
   signing; otherwise Android is usually less friction.
3. Implement Iris macOS as the compatibility peer.
4. First make standard BitChat `ANNOUNCE` discovery work.
5. Then make Noise handshake and `NDR_EVENT` round trips work.
6. During this stage, render raw BitChat nickname/peer ID rows if needed.
7. Add Nostr profile/avatar extensions only after the BitChat wire behavior is
   compatible enough.

Do not base Iris on `bitchat-terminal`. It is useful background for BLE
experiments, but it tracks an older BitChat crypto/message shape. Use the
current BitChat iOS and Android repos as the compatibility source of truth.

Keep the reusable parts in shared Rust:

- BitChat packet encode/decode
- Noise compatibility wrapper and test vectors
- nearby peer/session state
- local outbox fanout and inbound event dedupe
- NDR event routing between nearby transport and the existing chat flow

Keep platform adapters thin:

- macOS: first BLE and mDNS/TCP implementation target
- iOS: native BLE adapter and the same shared Rust protocol code
- Android: native BLE adapter and the same shared Rust protocol code

Compatibility should be verified against real clients:

- Iris macOS to Iris macOS over LAN, with message servers disabled
- Iris macOS to Iris iOS over BLE and/or LAN
- Iris macOS to Iris Android over BLE and/or LAN
- Iris macOS to BitChat iOS for `ANNOUNCE`, Noise, and `NDR_EVENT`
- Iris macOS to BitChat Android for `ANNOUNCE`, Noise, and `NDR_EVENT`

Simulators are useful for UI and core routing tests, but BLE compatibility needs
real devices.

## Presence Card

Presence should be cleartext only when the user has made themselves visible.
That is the privacy boundary: showing an avatar and name requires revealing a
stable profile key to nearby people.

Use a phased BitChat-compatible `ANNOUNCE` packet.

Compatibility baseline:

- random rotating 8-byte BitChat peer ID as `senderID`
- BitChat identity TLV fields:
  - nickname
  - Noise public key
  - Ed25519 signing public key, if available

Iris profile extension, after baseline BitChat compatibility:

- Iris extension TLV fields in an unknown/ignored range:
  - Nostr public key
  - capability flags, such as `iris-nearby/1`, `ndr-oob/1`, `lan-stream/1`
  - optional cached Nostr kind 0 profile event
  - Nostr signature over the binding: Noise public key, peer ID, Nostr public
    key, timestamp, capabilities

Existing BitChat clients should still be able to parse the standard fields and
ignore unknown extension TLVs. Iris clients can use the Nostr public key or
profile event to render the nearby list, and can fetch fresher profile metadata
from message servers when available.

## Noise Compatibility

Iris does not need to use the exact same source library as BitChat, but it must
match the wire behavior exactly:

- Noise protocol name: `Noise_XX_25519_ChaChaPoly_SHA256`
- static Noise identity key: X25519
- transport ciphertext format: 4-byte big-endian nonce prefix followed by
  ChaCha20-Poly1305 ciphertext and tag
- BitChat packet framing, sender ID, recipient ID, TTL, signatures, and
  fragmentation behavior

The iOS and Android BitChat clients already use different implementations while
matching the protocol. Rust/Iris can do the same if the test vectors cover
handshake messages, transport ciphertext, and full packet round trips.

## Nostr Authentication

Do not replace BitChat Noise static keys with Nostr keys. Nostr identity is
secp256k1, while BitChat Noise uses X25519.

Instead, bind identities inside the established Noise channel:

1. Complete BitChat Noise XX.
2. Exchange an encrypted identity-binding payload containing:
   - BitChat Noise public key
   - current BitChat peer ID
   - Nostr public key
   - timestamp or nonce
   - supported capability flags
3. Sign that binding with the Nostr key.
4. Optionally include the BitChat Ed25519 signing key for native BitChat
   identity continuity.

Treat the Nostr binding as a claim until the signature verifies and local policy
accepts it. For Iris, this should map the nearby device to the user's local
social graph and to any existing direct chat.

## NDR Handoff

Once the Noise channel is established and the peer is accepted by policy:

1. Send the local NDR invite event JSON as `NoisePayloadType.NDR_EVENT`.
2. Process inbound `NDR_EVENT` JSON through `nostr-double-ratchet`.
3. Bounce any NDR invite response events back through `NDR_EVENT`.
4. When NDR reports an active session, route normal private messages through
   NDR.
5. Store resulting NDR message events in the normal local outbox.
6. Send those signed Nostr events directly over every currently bound nearby
   transport for that peer.
7. Also publish them to Nostr message servers when network is available.

This keeps nearby delivery and message-server delivery as carriers for the same
signed Nostr/NDR events rather than separate chat histories.

## Airplane-Mode Chat

Treat "publish message" as "create and route a signed Nostr/NDR event", not as
"must reach a message server first".

Outbound routing should fan out to available carriers:

- local database/outbox, always
- nearby direct transport, when the recipient has an active bound nearby session
- Nostr message servers, when internet is available
- later, trusted mailbag storage for opportunistic carry

Inbound events from nearby direct transport should enter the same ingestion path
as events from message servers. Dedupe by event ID before decrypting or updating
the chat projection.

This lets two nearby users start or resume a private chat with no internet. When
internet returns, the already-signed events can still be published to message
servers for backup, multi-device sync, and delivery to devices that were not
nearby.

## App Boundary

Keep `nostr-double-ratchet` transport-neutral. It already has the important
shape:

- feed signed inbound events into the runtime
- drain signed outbound events from the runtime
- surface decrypted messages back to the app

The nearby layer belongs in Iris app/core transport code, not inside NDR. The
app should adapt `BitchatPacket` traffic into the same event ingestion path used
for message servers.

## Nearby Row

The "Nearby" chat-list entry should be virtual UI state, not a persisted fake
chat thread.

It should summarize:

- count of nearby devices
- count of nearby people mapped to known users
- whether local transport is active
- unread nearby-only activity, if any

Opening it should show discovered people and allow starting or resuming a
normal direct chat. Once a Nostr/NDR identity is bound, messages should land in
the regular direct chat, not in a separate nearby-only history.

## Mailbags

Implement mailbags in phases.

### Phase 1: No Mailbag

Ship live nearby discovery plus live BitChat-compatible NDR bootstrap and local
event delivery. This phase should already support airplane-mode chat between
devices that are visible, handshaken, and currently reachable over nearby
transport.

### Phase 2: Trusted Mailbag

Devices may store a small rolling mailbag of signed Nostr/NDR events and serve
them to authenticated nearby devices.

Rules:

- only after Noise and Nostr identity binding
- only for peers in the user's social graph, such as followed users or known
  friend-of-friend style policy
- request by active NDR author filters or event IDs, not "send everything"
- bounded by count, bytes, and age
- dedupe by Nostr event ID before processing

### Phase 3: Random Mailbag

Random-device mailbags should be separate, tiny, and muted by default because
they are spam-prone.

Rules:

- separate storage bucket from trusted mailbag
- strict byte and count limits
- short TTL
- no notifications until an event decrypts or otherwise proves useful
- rate-limit by courier device and source event author

## Sync Protocol

For BitChat compatibility, reuse or extend request-sync semantics rather than
inventing a parallel mechanism:

- request sync is local-only
- responses are local-only
- use compact filters or event-ID sets to avoid blind dumps
- do not forward sync requests beyond direct nearby peers
- keep old and random data separated from trusted social-graph data

For Iris NDR mailbags, the requested set should be based on:

- current NDR message author pubkeys
- recent event IDs already known
- bounded timestamp windows
- optional trusted owner pubkeys

## Security Notes

- A nearby courier being trusted does not make every carried event trusted.
- A Nostr event's normal signature and NDR decryption outcome remain the source
  of message authenticity.
- Random mailbags must not create notification spam.
- NDR event authors are rotating ratchet keys, so pre-decryption classification
  by stable social identity is limited.
- Nearby packet parsing must be defensive: size caps, fragmentation caps,
  timestamp windows, and per-peer rate limits.

## Implementation Steps

1. Add the virtual "Nearby" chat-list row and Nearby screen.
2. Add visibility state and native/platform hooks to start or stop local
   discovery/listening.
3. Add a small BitChat packet codec and test vectors in Rust.
4. Add baseline BitChat `ANNOUNCE` encode/decode without Iris profile
   extensions.
5. Add the first macOS nearby adapter with mDNS/TCP packet exchange.
6. Add macOS BLE packet exchange for real BitChat compatibility testing.
7. Render nearby peers from BitChat nickname/peer ID so compatibility can be
   tested before profile extensions.
8. Add Noise XX compatibility tests against captured iOS and Android handshake
   and encrypted payload fixtures.
9. On "Start chat", run BitChat Noise handshake.
10. After Noise is established, verify the Nostr identity binding.
11. Implement `NDR_EVENT` send/receive over nearby transport.
12. Route established NDR messages through the existing Iris direct-chat flow.
13. Fan out outbound NDR events to the local outbox, active nearby transport,
    and message servers when available.
14. Add airplane-mode end-to-end tests for two nearby peers with no message
    server connectivity.
15. Add Iris `ANNOUNCE` extension TLVs for Nostr profile/avatar rendering.
16. Port the thin nearby adapter to iOS and Android.
17. Add trusted mailbag sync.
18. Add random mailbag sync only after trusted sync is stable.

## Test Plan

- Rust packet codec tests for v1/v2 `BitchatPacket`.
- Cross-language Noise vectors for iOS, Android, and Rust.
- End-to-end local TCP test: discover, Noise handshake, NDR invite exchange,
  NDR message decrypt.
- Airplane-mode local chat test: two peers with no message server connectivity
  exchange NDR messages directly, then later publish/dedupe the same events when
  connectivity returns.
- macOS real-hardware smoke: BLE visible, nearby list renders profile, start
  chat, direct local message delivery.
- BLE compatibility smoke with iOS and Android BitChat clients.
- Message-server fallback test: establish nearby, disconnect nearby, continue
  over Nostr message servers.
- Mailbag tests for size caps, TTL eviction, dedupe, and random/trusted
  separation.
