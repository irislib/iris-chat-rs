# FIPS BLE Android adapter

This module is the native Android half of the portable FIPS host BLE transport.
It was vendored from `platform/android/fips-ble` in FIPS commit `29e784207` so
Iris builds without a sibling checkout. Keep its protocol types in sync with
the pinned `fips-core` revision in `core/Cargo.toml`.
