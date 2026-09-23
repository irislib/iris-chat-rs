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
| `IRIS_CHAT_FIPS_WEBSOCKET_BIND_ADDR` | Optional WebSocket listener socket address for a browser-accessible local FIPS connection. Unset by default. |

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

## Voice and video calls

Calls use the same FIPS endpoint as chat. Both call control and media are
end-to-end encrypted FIPS datagrams on service port 39511; a WebSocket seed
can route a call without being a participant. Incoming calls require an
accepted contact and a device in that contact's verified device list. Calls
are ephemeral and are never replayed from the message outbox or stored as
attachments.

The native call interface is available on Android, iOS and macOS. Desktop
browsers use iris-chat; the Linux and Windows native shells do not yet have
capture or call controls. Phone integration uses iOS CallKit and Android
self-managed Telecom with a foreground call service. Active calls keep their
network connection while the app is in the background. An offline incoming
call cannot wake an app whose process or network connection has been suspended;
keep the apps open to establish an offline call.

Settings has independent Voice calls and Video calls switches. A disabled
switch hides that call button. Video calls can be answered with voice; with
video disabled and voice enabled, an incoming video offer rings as voice.
Voice answering negotiates an audio-only session and stops the caller's camera.
Mute and camera-off choices made while ringing remain in effect when answered.

The initial shared media format uses 20 ms mono PCM16 frames at 16 kHz and
independent JPEG video frames up to 320 by 240 pixels, eight frames per second.
This is a basic Wi-Fi/LAN format, not HD video or an optimized low-bandwidth
codec. Datagram fragments and decode/playback queues are bounded; incomplete
or late frames are dropped instead of accumulating latency. Ringing times out
after 30 seconds and an established call ends after 15 seconds without the peer.

### Offline verification

Native-to-native calls can use Nearby on the same Wi-Fi/LAN with no message
server. A browser needs an initial browser-compatible FIPS path. Configure a
local WebSocket seed under Call servers in iris-chat Settings; the native
listener above can provide it. A browser can upgrade that authenticated path
to WebRTC, but WebRTC discovery alone does not establish the initial path.
For a local HTTP test use localhost; an HTTPS web app may require a trusted
local WSS endpoint because browsers enforce secure-context and mixed-content
rules. Once contact identities and the FIPS route are established, calls do
not require a message server or internet access.

`scripts/test_calls_offline.sh` builds and runs the production Rust call
lifecycle over two authenticated local UDP endpoints. macOS sandbox rules
allow loopback traffic, explicitly reject a nonlocal network probe, and cover
bidirectional media, mute, camera, hangup, and voice answering. Other runners
return infrastructure-unavailable (75) instead of claiming network isolation.
The `iris-call-fixture` binary behind the `stack-fixture` feature drives the
same FFI actions and echoes received media for browser and Android end-to-end
tests; it requires a fresh data directory and never uses an installed account.

`android/scripts/native-call-e2e.py` pairs a fresh emulator test account with
that fixture, blocks non-loopback traffic on both sides, and stops the local
message server before calling. It checks real microphone/camera capture,
returned media bytes, audio playback, voice answering, and remote hangup.
It requires a root-capable emulator, leaves installed account data intact,
and removes its temporary network rules on exit. This exercises a local
WebSocket FIPS connection forwarded to the emulator; it does not prove a
physical Wi-Fi or Bluetooth link. Browser coverage lives in iris-chat's
`e2e/calls.spec.ts` and `e2e/calls-native.spec.ts`.
