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

Set `IRIS_CHAT_SAME_HOST_HASHTREE=1` to let the logged-in Chat FIPS endpoint
discover authenticated `hashtree.blob/1` providers over fixed loopback UDP.
This is an optional read optimization: provider misses or failures continue
through Chat's configured Blossom sources, and attachment writes are unchanged.

Primary development is on hashtree:
https://git.iris.to/#/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/iris-chat-rs
