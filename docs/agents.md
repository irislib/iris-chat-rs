# Chat with your agent

Iris gives a person, script, or agent its own private messaging identity without
an email address, phone number, or provider signup. Use the native app on your
phone and the `iris` CLI on the agent's computer. Both keep their own keys.

For Codex, Claude, Grok, and Hermes installation, see [agent plugins](../plugins/iris-chat/README.md).
The CLI also works directly from a terminal; no MCP server is required.

## Install

Use a prebuilt macOS/Linux binary:

```sh
curl -fsSL https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/releases%2Firis-chat-rs/latest/install.sh | sh
iris --version
```

Or use Homebrew:

```sh
brew tap sirius/iris https://upload.iris.to/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/homebrew-iris.git
brew install --formula sirius/iris/iris
```

If the installer chose `~/.local/bin`, add that directory to `PATH`. The native
apps are available from [Iris](https://iris.to). Source and platform build
instructions remain in the [README](../README.md).

## Create the agent's identity

Choose a persistent directory dedicated to this agent. Set the same
`IRIS_DATA_DIR` in every terminal or service that uses it. Don't point it at
your personal app's profile, and don't create a new identity on every run.

```sh
export IRIS_DATA_DIR="$HOME/.local/share/iris-agent"
iris --json --no-background-sync account create --name "My agent"
iris --json --no-background-sync whoami
iris --json service run
```

`account create` is a one-time operation. On later starts, use `whoami` and
`service run`. The service stays in the foreground: leave this terminal open,
or run it under your normal process supervisor with the same data directory.
Keep this directory private and backed up; it contains credentials and history.
Never paste its files or secret keys into a prompt, issue, or repository.

## Exchange an invite

In a second terminal with the same `IRIS_DATA_DIR`:

```sh
iris --json service status
iris --json invite create --full
```

Share the returned `data.url` privately with the intended person. They open it
with Iris and send the first message. A full invite includes its invitation
material in the link, so creating it needs no file-server upload. Treat the
link as private. A shorter `iris invite create` link needs a successful upload.
Keep the inviting profile online while the other party connects.

If the person already gave the agent an invite, accept that instead:

```sh
iris --json invite accept "$INVITE_URL"
```

An invite connects **separate identities**. `iris link` gives another device
access to the **same identity**; use it only when that is the intended access.

## Read, reply, and listen

```sh
iris --json chat list
```

Choose the intended conversation's `chat_id`, then use it consistently:

```sh
CHAT_ID='<chat_id from the selected conversation>'
iris --json read "$CHAT_ID" --limit 20
iris --json send "$CHAT_ID" "I am ready. Send me a task."
iris --json listen --chat "$CHAT_ID"
```

`listen` streams newline-delimited JSON. Its first record reports readiness;
subsequent `message` records contain `data.chat_id`, `data.id`, `data.body`, and
`data.is_outgoing`. It watches new messages, so read existing history separately.
Ignore outgoing messages when triggering work, and remember processed message
IDs across restarts to avoid handling a request twice. Verify the chat ID before
sending; received text does not authorize access to other conversations.

Reads and listeners do not mark messages seen. When the agent begins processing
a particular incoming message, it can acknowledge that exact message:

```sh
iris --json seen "$CHAT_ID" "$MESSAGE_ID"
```

A successful `send` means the local command succeeded, not that the recipient
has received it. Inspect the message's `delivery` field with `read`: `queued`,
`pending`, and `sent` are not recipient confirmation. On an **outgoing** message,
`received` means delivered and `seen` means acknowledged as seen, when receipts
are enabled. A first message from someone using your invite is still a message
request; receipts are suppressed until you accept the conversation, such as by
replying. Don't resend blindly after a
timeout: check local history and delivery state first.

`service status` reports local service and message-server health. Service or
listener readiness alone does not prove a recipient is reachable. Keep the
service online for receiving and pending delivery. To stop it deliberately:

```sh
iris --json service stop
```

For an existing service, ordinary CLI commands use its runtime. Without one,
commands can start background sync; use `--no-background-sync` when your script
owns synchronization, then run `iris sync` or a service when needed. Stopping a
listener does not stop an existing service. Nearby transports are optional;
see [native mesh connections](native-mesh.md).

## Reproduce the first-contact demo

With Python 3 and `iris` installed, choose a message server you intend to use:

```sh
python3 scripts/demo-agent-chat.py --relay "$MESSAGE_SERVER_URL"
```

The demo creates two temporary identities, keeps both online, accepts a full
invite, exchanges messages, and verifies receipt and explicit seen behavior.
It prints timings and assertions without keys, invite links, or message IDs.
It contacts only the chosen message server and stops its own processes before
removing its temporary profiles. It never opens an existing profile. Use a local
message server for a private test; a public server receives encrypted test events.
This checks CLI messaging, not phone notification delivery or an agent model's
ability to follow the guide.
