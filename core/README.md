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

Primary development is on hashtree:
https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs
