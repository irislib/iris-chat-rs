# FIPS BLE Apple adapter

This package is the native Apple half of the portable FIPS host BLE transport.
It was vendored from `platform/apple` in FIPS commit `29e784207` so Iris builds
without a sibling checkout. Keep its protocol types in sync with the pinned
`fips-core` revision in `core/Cargo.toml`.
