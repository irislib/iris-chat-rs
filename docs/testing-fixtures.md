# Testing Fixtures

Use the core fixture builders when a test needs lots of chats, groups, or
messages but does not need relay/protocol behavior:

- `build_large_test_app_state(direct_chat_count, group_chat_count, messages_in_current_chat)`
- `build_large_test_search_result(query, contact_count, group_count, message_count)`

These are deterministic and capped so UI/perf tests can safely ask for large
datasets. They are exposed through UniFFI for shell tests and also usable from
Rust integration tests.

Use the local relay for protocol and persistence behavior. The in-process Rust
fixture is `iris_chat_core::local_relay::TestRelay`; the standalone binary is:

```sh
cargo run --manifest-path core/Cargo.toml --features local-relay-bin --bin local_nostr_relay -- 127.0.0.1:4848
```

Use production relays only for smoke tests that must prove interoperability with
the live network. Keep performance/rendering tests deterministic so failures are
not caused by relay availability or account history.
