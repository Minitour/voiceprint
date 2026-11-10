# voiceprint Agent Plugin

The portable way to give an agent your voice: voiceprint's agent skills, with no
CAPA, MCP servers, subagents or hooks. It follows
[Agent Plugins 1.0](https://agent-plugins.org/specification). For the CLI itself and
how the model works, see the [root README](../../README.md).

## What is included

- `write-like-me`: before drafting, read your voice card; after drafting, run
  `voiceprint check`, revise each flagged word or phrase using your own vocabulary,
  and re-check until no high-severity flags remain.

The root `plugin.json` is authoritative. `.claude-plugin/plugin.json`,
`.codex-plugin/plugin.json` and `.cursor-plugin/plugin.json` are metadata-only
adapters for clients that look for their own entry point. The skill in this
directory is canonical; `capa add Minitour/voiceprint@write-like-me` installs this
same file.

## What is not included

No binaries, no index, and none of your writing. The skill calls the `voiceprint`
CLI (it falls back to `npx voiceprint` when it isn't installed) and reads the index
you build from your own text.

## Prerequisites

- Node.js 16+ (for `npx voiceprint`), or `npm install -g voiceprint`
- A voice index built from text you wrote, once:

  ```bash
  npx voiceprint build ~/writing -o ~/.voiceprint/voice.vpi
  ```

  The skill looks for it at `$VOICEPRINT_INDEX`, or `~/.voiceprint/voice.vpi` by default.

## Load in Claude Code

```text
/plugin marketplace add Minitour/voiceprint
/plugin install voiceprint@voiceprint
```

The skill appears as `/voiceprint:write-like-me`, and Claude also uses it on its own
when you ask for text "in my voice". Reload with `/reload-plugins` after updates.

See [Claude Code plugins](https://code.claude.com/docs/en/plugins).

## Load in Cursor

In Cursor Agent chat:

```text
/add-plugin https://github.com/Minitour/voiceprint
```

Then install **voiceprint** from the imported marketplace. Teams can instead import
`https://github.com/Minitour/voiceprint` under **Dashboard → Plugins** and enable
auto-refresh.

See [Cursor plugins](https://cursor.com/docs/plugins).

## Load in Codex

Register the GitHub repository as a marketplace:

```bash
codex plugin marketplace add Minitour/voiceprint
```

Then install `voiceprint` from the Plugin Directory. The `.codex-plugin/plugin.json`
adapter supports clients that still expect Codex's client-specific entry point.

See [Codex plugin packaging](https://developers.openai.com/plugins/build/plugins).

## Load with CAPA

```bash
capa add Minitour/voiceprint@write-like-me --install
```

## Updating and developing

Marketplace installs don't need a clone. When you change the skill, bump `version`
in all four `plugin.json` files and in `.claude-plugin/marketplace.json`; CI checks
that they match `Cargo.toml`, so the plugin and the CLI release together.
