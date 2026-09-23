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
keep signaling and media ephemeral: neither is replayed from the message outbox
or stored as attachments. A local summary in chat history records direction,
missed/answered/canceled/declined outcome, voice or video, and answered duration.
Other ringing devices show “Answered on another device” without inventing a duration.
These summaries persist on that device and are not sent to peers or synced to
other devices. A video call answered with voice is recorded as a voice call.

The native call interface is available on Android, iOS and macOS. Desktop
browsers use iris-chat; the Linux and Windows native shells do not yet have
capture or call controls. Phone integration uses iOS CallKit and Android
self-managed Telecom with a foreground call service. Active calls keep their
network connection while the app is in the background. An offline incoming
call cannot wake an app whose process or network connection has been suspended;
keep the apps open to establish an offline call.

iOS uses the standard CallKit `voip` background mode and normal microphone,
camera and local-network permissions. Raw multicast discovery on physical iOS
requires the provisioned `com.apple.developer.networking.multicast` entitlement,
which this project does not currently include; simulator results do not establish
physical-device multicast support. Already-connected unicast FIPS calls are
unaffected.

Settings has independent Voice calls and Video calls switches. A disabled
switch hides that call button. Video calls can be answered with voice; with
video disabled and voice enabled, an incoming video offer rings as voice.
Voice answering negotiates an audio-only session and stops the caller's camera.
Mute and camera-off choices made while ringing remain in effect when answered.

Calls encode voice as 48 kHz mono Opus (32 kbps, 20 ms packets) and video as
H.264 access units with an adaptive bitrate. Camera capture targets 720p/30fps
when supported; High can capture 1080p on capable devices. Auto caps video at
2 Mbps, High at 4 Mbps, and Data saver at 400 kbps. The custom cap ranges from
100 kbps to 10 Mbps and can change during a call. These are ceilings, not promises
of a fixed bitrate or resolution. Audio and transport headers add overhead.

Browser clients use WebCodecs and a bundled, pinned libopus module. Android uses
MediaCodec and Apple uses VideoToolbox, with platform microphone echo/noise
processing. Native audio uses the same pinned libopus version. The browser probes
actual codec support before calling; no installed native helper is required.
Calls do not require a separate calling or TURN server. FIPS transport upgrades
use host candidates and optional STUN address discovery to find direct routes
through NAT. Native clients inherit the shared FIPS defaults:
`stun:stun.l.google.com:19302`, `stun:stun.cloudflare.com:3478`, and
`stun:global.stun.twilio.com:3478`. STUN discovers addresses; it does not carry
call media. Encoded audio/video and call controls travel inside the authenticated
FIPS call service.

Native ICE gathering is bounded to two seconds. With every STUN server
unreachable, the pinned native FIPS transport may reject a host-only WebRTC
upgrade. Calls can continue on their existing FIPS route, including local UDP
without Internet access.

The client media layer supplies bounded audio jitter buffering, Opus forward
error correction and packet-loss concealment, video reordering, keyframe requests,
and deadline-limited retransmission of missing video fragments. Receiver feedback
adjusts the sender's video bitrate below the user's cap. The codecs are standard
Opus/H.264; adaptation and packet delivery are the Iris client implementation.

Protocol version 3 (`opus-h264-v3`, `IC03`) replaces the earlier PCM/JPEG prototype.
Each frame has a per-kind sequence number, capture timestamp, keyframe flag and
bounded fragments. H.264 keyframes contain SPS/PPS and an IDR; no B frames are
used. Retransmission caches are bounded to eight frames/1 MiB and expire after
300 ms; incomplete receiving frames expire after 250 ms. Real-time queues discard
stale work instead of accumulating latency. Ringing times out after 30 seconds;
an established session ends after 15 seconds without its peer. Both clients must
support version 3 to call each other.

### Offline verification

Native-to-native calls can use Nearby on the same Wi-Fi/LAN with no message
server. A standalone browser needs an initial browser-compatible FIPS path.
Configure an existing local FIPS WebSocket seed under Call servers in browser
settings; a native FIPS listener can provide it. No additional media service is
needed. FIPS can subsequently use its WebRTC data transport while keeping media
inside FIPS packets. WebRTC discovery alone does not establish the initial path.
Use localhost for same-host HTTP tests; other devices need HTTPS/WSS with a
certificate the browser trusts for microphone/camera access and browser network
policy. Once contact identities and the FIPS route are established, calls do not
require a message server or Internet access.

`scripts/test_calls_offline.sh` runs production call control and compressed-frame
transport over authenticated local FIPS UDP with macOS sandbox rules denying all
nonlocal traffic, including an explicit denial probe. It covers bidirectional
codec packets, mute, camera, hangup and voice answering. Separate codec tests
exercise actual Opus encode/decode, reordering, FEC/PLC and recovery after mute.

`iris-call-fixture` (feature `stack-fixture`) uses a fresh native FFI account and
echoes the actual encoded Opus/H.264 frames for interoperability tests. It also
decodes received Opus to measure nonzero audio. It never uses an installed account.
Browser coverage lives in iris-chat's `e2e/calls.spec.ts` and
`e2e/calls-native.spec.ts`, with actual encoders, decoders, bitrate feedback and
bandwidth/loss tests.

`android/scripts/native-call-e2e.py` pairs a fresh emulator account with the native
fixture, blocks non-loopback traffic and stops the local setup message server
before calling. Installed account data is preserved and temporary network rules
are removed on exit. This exercises local FIPS transport, not a physical Wi-Fi or
Bluetooth link. Results distinguish simulated capture from physical hardware.
