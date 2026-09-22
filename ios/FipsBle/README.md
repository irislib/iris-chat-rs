# FIPS BLE Apple adapter

This package is the native Apple half of the portable FIPS host BLE transport.
It was vendored from `platform/apple` in FIPS commit `29e784207` so Iris builds
without a sibling checkout. Keep its protocol types in sync with the pinned
`fips-core` revision in `core/Cargo.toml`.

Bootstrap metadata is cached for each scan. Failed GATT discovery retries back
off from five seconds to one minute; these failures occur before FIPS receives
a peer candidate, so its connection backoff cannot handle them. A timer retries
discovery even when CoreBluetooth coalesces further advertisements.

On macOS, speculative bootstrap reads from Apple's tentative overflow UUID
matches wait while a Bluetooth audio device is active. Explicit FIPS service
advertisements and previously identified peers can still trigger discovery.
Scanning, advertising, cached discovery observations, and established L2CAP
connections remain available. CoreAudio activity notifications resume deferred
discovery after five seconds of idle audio, without opening or recording an
audio stream. Brief pauses and route changes do not restart speculative reads.
Previously unknown peers advertising only through Apple's background overflow
area can therefore take longer to appear during Bluetooth playback or calls.

Run the adapter tests with `swift test --package-path ios/FipsBle`. For changes
to radio behavior, also check Bluetooth audio during app startup and across
playback stop/start transitions; passing state tests alone does not establish
that discovery coexists with audio on real hardware.
