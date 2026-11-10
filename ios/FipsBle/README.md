# FIPS BLE Apple adapter

This package is the native Apple half of the portable FIPS host BLE transport.
It was vendored from `platform/apple` in FIPS commit `29e784207` so Iris builds
without a sibling checkout. Keep its protocol types in sync with the pinned
`fips-core` revision in `core/Cargo.toml`.

Each GATT bootstrap endpoint is reported once per scan, with a fresh discovery after service
invalidation or a rejected L2CAP open. Repeated advertisements must not reopen
a Bluetooth link FIPS closed because it already has a healthy network path.
FIPS retains the bootstrap parameters and owns connection retries.

Discovery reads all matching service instances, keeping apps on the same phone
as separate candidates. Starting a scan or returning to the foreground also
checks peripherals already connected by another app; these may no longer
advertise. A retrieved peripheral is connected locally before its services are
read. The service UUID, bootstrap bytes and FIPS wire protocol remain unchanged.

Failed GATT discovery retries back off from five seconds to one minute. These
failures occur before FIPS receives a peer candidate, so its connection backoff
cannot handle them. A timer retries
discovery even when CoreBluetooth coalesces further advertisements.
Connect/service/read attempts that never receive a callback expire after 15
seconds and enter the same backoff. Closing one L2CAP channel preserves the
underlying peripheral connection while discovery, another channel, or a new
open still uses it; stopping discovery releases otherwise idle connections.

On macOS, speculative bootstrap reads from Apple's tentative overflow UUID
matches wait while a Bluetooth audio device is active. Explicit FIPS service
advertisements and previously identified peers can still trigger discovery.
Scanning, advertising, new peer discovery, and established L2CAP connections
remain available. CoreAudio activity notifications resume deferred
discovery after five seconds of idle audio, without opening or recording an
audio stream. Brief pauses and route changes do not restart speculative reads.
Previously unknown peers advertising only through Apple's background overflow
area can therefore take longer to appear during Bluetooth playback or calls.

Run the adapter tests with `swift test --package-path ios/FipsBle`. For changes
to radio behavior, also check Bluetooth audio during app startup and across
playback stop/start transitions; passing state tests alone does not establish
that discovery coexists with audio on real hardware.

Cancellation is asynchronous. Expired discovery and stopped scans prevent new
reads or L2CAP opens on that peripheral until CoreBluetooth delivers its terminal
disconnect/failure callback. An existing open or channel is allowed to finish
before cancellation. The retry backoff alone never authorizes reusing the
peripheral. If that terminal callback is missing, the peripheral stays unavailable
until the adapter restarts; commands are rejected immediately, not queued, and no
retry timer spins for that peripheral. A scan restart does not clear this boundary.

On iOS, the host can opt into CoreBluetooth state restoration with a stable
`restorationIdentifier`. Restored peripherals are rediscovered; the new process
publishes a fresh L2CAP channel and bootstrap instead of reusing the old PSM.
Background scans keep the service filter and disable duplicate advertisements.
Foreground discovery timers stop in the background, preserving pending connects;
up to 16 peers seen within the last 15 minutes can have a pending reconnect.
CoreBluetooth decides when to deliver callbacks. Force-quitting the app normally
prevents relaunch, and background discovery may take longer than foreground scans.

Iris retains this bridge while backgrounded. Inbound Bluetooth activity grants
five seconds of core processing, with storage flushed before its background task
ends. Polling and write completions cannot renew the allowance. The physical gate
`scripts/physical_ble_idle.py --background` additionally requires an Android peer
with no IP route to receive a delivery receipt before the iPhone returns to the
foreground; unit tests do not establish that L2CAP wakes a suspended phone.

For foreground adapter isolation, `PhysicalTests/project.yml` defines a separate
development app and UI test; generate its project with XcodeGen and build it for
testing. Pair it with Android's `:fips-ble:assembleDebugAndroidTest` output. Run
both tests concurrently with the same fresh 32-character hexadecimal token:
`IRIS_BLE_PROBE_TOKEN` in the Apple test's `.xctestrun` environment, and Android
instrumentation argument `-e probeToken`. Reserve both physical devices first.
The dedicated bundles are `to.iris.bleprobe`, its UI-test runner, and
`org.fips.ble.test`; no Iris account is needed. Require an actual passing test on
each phone, not a skipped test or just a successful instrumentation command.
