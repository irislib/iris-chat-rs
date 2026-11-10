# FIPS BLE Android adapter

This module is the native Android half of the portable FIPS host BLE transport.
It was vendored from `platform/android/fips-ble` in FIPS commit `29e784207` so
Iris builds without a sibling checkout. Keep its protocol types in sync with
the pinned `fips-core` revision in `core/Cargo.toml`.

Discovery reads every matching GATT service, since multiple apps can expose the
same service UUID on one phone. Each bootstrap characteristic gets a separate
opaque peer token; connecting still uses the underlying Bluetooth address and
the bootstrap's PSM. Bootstrap bytes and the FIPS wire protocol are unchanged.

Unanswered GATT discovery expires after 15 seconds and retries with a five-second
to one-minute backoff, even without another advertisement. Late callbacks cannot
finish a replacement read. Completed discovery remains deduplicated until a scan
restart or a rejected L2CAP open requests fresh connection parameters.

Run the unit tests with `./gradlew :fips-ble:testDebugUnitTest` from `android`.
The opt-in `PhysicalBleDiscoveryTest` pairs with
`ios/FipsBle/PhysicalTests`. Both phones advertise an unrelated matching service
before the actual test service, then must discover each other and exchange
run-specific bytes. This tests the native adapters, not Iris account/session or
background-delivery behavior.
