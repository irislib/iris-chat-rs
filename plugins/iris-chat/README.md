# Iris Chat agent plugin

The plugin teaches an agent to use the existing command line tool. It contains
one portable skill, no background hooks, and no hosted service or API key.
Install the tool on the agent's execution host using the [workflow guide](https://github.com/irislib/iris-chat-rs/blob/main/docs/agents.md).
Installing the plugin alone does not install binaries or start a service.

## Codex and ChatGPT desktop

```sh
codex plugin marketplace add irislib/iris-chat-rs
codex plugin add iris-chat@iris-chat-plugins
```

Restart the agent session and select Iris Chat, or ask it to use the
`iris-chat` skill. For a local checkout, replace the repository argument with
`.` while in this repository. The repository also exposes the plugin through
`.agents/plugins/marketplace.json` in supported desktop clients.

OpenAI marks plugins that call local programs as **Desktop only**. A dot needs a
supported local task on a connected computer for this plugin; installing it does
not give a cloud-only dot access to your computer or networking privileges.
Public-directory submission is separate from installing this repository.

## Claude Code

```sh
claude plugin marketplace add irislib/iris-chat-rs
claude plugin install iris-chat@iris-chat-plugins
```

Start a new session, then invoke `/iris-chat:iris-chat` with your request.
For local evaluation, run `claude --plugin-dir ./plugins/iris-chat` from this checkout.

## Grok Build and Grok Bot

Grok Build supports Claude-compatible plugins. From this checkout:

```sh
grok plugin install ./plugins/iris-chat
grok inspect --json
```

Start a new session and use its plugin/skills menu to select the workflow.
Grok Bot is a separate surface: give it the [workflow guide](https://github.com/irislib/iris-chat-rs/blob/main/docs/agents.md) and ask it to perform the task
with its terminal, then save the verified procedure as a skill. This repository
is not automatically listed in Grok Bot's Connect apps directory.

## Hermes Agent

Hermes supports the root `plugin.json` Agent Plugins v1 package directly:

```sh
hermes plugins install irislib/iris-chat-rs/plugins/iris-chat --no-enable
hermes plugins enable iris-chat
```

Start a new session and ask Hermes to load `iris-chat:iris-chat` using
`skills_list` and `skill_view`. No Hermes-specific Python extension or MCP
server is needed. The terminal environment must have the CLI and persistent
storage for its identity. Plugin updates must not replace that identity.

## Other Agent Skills clients

Install `plugins/iris-chat/skills/iris-chat` from the repository using your client's skill installer. Keep the whole
folder together. The [skill](skills/iris-chat/SKILL.md) is the canonical
workflow; platform manifests all refer to it.

## What it accesses

When asked to message, the agent invokes the Iris CLI with a chosen local profile.
The CLI stores its identity and message database outside the plugin, connects to
configured message servers and peers, and sends only to the intended invite or
conversation. An explicitly started service stays online until stopped.
The skill never asks for secret keys in chat. No telemetry is added.

## Versions and updates

The plugin and `iris` have separate versions. Plugin installation and updates do
not upgrade the CLI. A fresh `latest`/unpinned setup uses that channel's current
release; existing binaries change only through a separate upgrade. Preserve
the identity and data directories when upgrading.

A compatible CLI release needs no plugin update while the documented commands,
JSON fields, permissions, and setup still work. When instructions or package
metadata change, bump both manifests' `version` together, validate, and publish
a new ZIP or catalog revision. Catalog review and each client's update settings
govern delivery; changing the repository does not update every installed copy.

## Package and validate

From a committed checkout, create a small distribution ZIP without app sources,
build artifacts, identities, or configuration:

```sh
git archive --format=zip --output=iris-chat-plugin.zip HEAD:plugins/iris-chat
claude plugin validate plugins/iris-chat
```

Keep generated ZIPs outside commits. Validate an extracted ZIP too, and run the
workflow with a fresh test identity before submitting to a directory. Public
catalogs have separate review requirements; a valid package is not a listing.

Packaging references: [OpenAI](https://developers.openai.com/plugins/build/plugins),
[desktop execution](https://developers.openai.com/plugins/deploy/desktop-only),
[Claude](https://code.claude.com/docs/en/plugins/publish),
[Grok](https://docs.x.ai/build/features/skills-plugins-marketplaces), and
[Hermes](https://hermes-agent.nousresearch.com/docs/developer-guide/plugins).
