# Iris Chat

Iris Chat is an encrypted chat client built on the Iris shared Rust core.
This crate publishes the `iris` command line tool and the `iris_chat_core`
library used by the native apps.

## Install

```sh
cargo install iris-chat
```

## CLI

```sh
iris account create --name Alice
iris whoami
iris chat create <user-id>
iris send <chat-id> "hello"
iris read <chat-id>
iris listen
```

Use `--json` for scripts and agents.

Use a separate `--data-dir` for each account, including bots. A data folder is
bound to one account; restoring another key there is rejected without deleting
its history or replacing its saved credentials. Devices of the same account
can continue using that account's folder. Older folders are adopted only when
their stored identity is unambiguous. If a folder contains multiple identities
or history with no identifiable account, keep it for recovery and use a new
folder for the bot:

```sh
iris --data-dir /path/to/bot-chat restore <bot-secret-key>
iris --data-dir /path/to/bot-chat listen
```

### Persistent CLI service

On macOS, Linux and Windows, run one foreground service for a profile:

```sh
iris --data-dir /path/to/bot-chat service run
```

Other CLI invocations with that same data directory automatically use its private
local endpoint. `listen` can stay connected while `send`, `read`, `sync` and other
commands use the same encryption runtime. The existing profile lock remains held
by the service. Different profiles remain independent. Use `service status` for
connection health and `service stop` for a graceful shutdown. A process supervisor
may restart `service run` after a crash; stale endpoints are reclaimed only after
acquiring the exclusive profile lock.

The transport uses a private Unix socket on macOS/Linux and an owner-only named
pipe on Windows, with same-user peer verification in both directions. It does not
open a TCP port. Set `IRIS_REQUIRE_SERVICE=1` in supervised scripts to fail closed
when the service is unavailable. A failed or disconnected send is never retried
automatically; inspect message/delivery state first. Stream consumers must retain
handled message IDs and reconcile history after reconnecting. `ready` means the
local subscription is established; `service status` reports network readiness.

Without a running service, standalone commands continue to work as before.
The service is a desktop CLI feature; iOS and Android apps continue to own their
existing in-process core and platform push/background lifecycle.

Set `IRIS_CHAT_SAME_HOST_HASHTREE=1` to let the logged-in Chat FIPS endpoint
discover authenticated `hashtree.blob/1` providers over fixed loopback UDP.
Chat's local cache, one composite FIPS provider route, and its configured
Blossom sources share the canonical hash-verifying `BlobRouter`. Provider
misses, failures, or exit leave the other routes available, while attachment
writes remain application-owned and unchanged.

Process-level integration tests may set `IRIS_CHAT_FIPS_LOCAL_RENDEZVOUS_ADDR`
to an isolated non-zero IPv4 loopback address. Normal application runs leave
it unset and use the fixed FIPS same-host rendezvous address.

Multi-device sync uses the independent Osiris and LNVPS FIPS WebSocket seeds by
default. Set `IRIS_FIPS_WEBSOCKET_SEED_URLS` to replace them with a
comma-separated list of explicit `wss://.../fips` URLs, or set it to an empty
value for an isolated run. These are authenticated FIPS physical peers;
ordinary Nostr message servers remain Nostr event and discovery/signaling
relays and do not carry FIPS packets. Signed update notices use the shared
`nostr-pubsub` path over both connected FIPS peers and configured message
servers.

Primary development is on hashtree:
https://git.iris.to/#/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/iris-chat-rs
