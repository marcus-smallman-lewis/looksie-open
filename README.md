# Looksie: the open parts

[Looksie](https://looksie.dev) is a desktop app that walks you through a big pull request one decision at a time.

This repository is the part of Looksie that touches your code, your tokens and the network, published so you can
check it yourself. **Every byte that leaves your computer goes through the code here.**

## What is here

| Folder | What it does | Licence |
| --- | --- | --- |
| `crates/looksie-connectors` | Every call to GitHub: reading pull requests and files, posting your review. Your token is used here and nowhere else. | Apache-2.0 |
| `crates/looksie-ai` | The one request to Anthropic or OpenAI when you use your own API key: exactly what is sent. | Apache-2.0 |
| `crates/looksie-core` | Fetches a review through a connector and hands it to the engine; where AI answers are kept; the MCP server (`looksie --mcp`) your own Claude Code or Codex uses. | Apache-2.0 |
| `crates/looksie-protocol` | The data model: reviews, steps, briefings, comments. | Apache-2.0 |
| `app` | The desktop app: the Tauri shell (accounts and the OS keychain, licence checks, connecting Claude Code and Codex, updates) and the whole React interface. | FSL-1.1-ALv2 |

## What is not here

**The engine** (`looksie-engine`): the rules, how a change is split into decisions and ordered, the briefing, the
claim checks, the AI prompt and the checks on AI answers. It is closed, and it is the part you pay for.

The engine has **no network code**. It is handed a pull request by the code above and returns a review; it cannot send
anything anywhere by itself. Because the app and `looksie-core` link the engine, they do not build from this repository
alone. The three crates that do not need it build and test on their own:

```bash
cargo test
```

## What leaves your computer

There are no Looksie servers and no telemetry. Looksie talks to:

- **GitHub** (`api.github.com`, or your company's GitHub server), with your own token: `crates/looksie-connectors/src/github.rs`.
- **Anthropic or OpenAI** (`api.anthropic.com`, `api.openai.com`), only if you add your own API key, with the prompt
  you can see first under "Show what will be sent": `crates/looksie-ai/src/lib.rs`.
- **Polar** (`api.polar.sh`), our payment partner, about once a week to check a Pro licence key: `app/src-tauri/src/store.rs`.
- **GitHub Releases**, to check for updates (`tauri.conf.json`, `plugins.updater`).

Your own Claude Code or Codex, if you connect it, runs on your computer under your own account and talks to Looksie
through the MCP server in `crates/looksie-core/src/mcp.rs`. How Looksie sets that up is in `app/src-tauri/src/agents.rs`.

To check this for yourself, run Looksie behind a firewall that shows connections (Little Snitch, GlassWire,
mitmproxy): you should see only the hosts above.

## Licences

See [LICENSE.md](LICENSE.md). In short: the crates are Apache-2.0. The app is source-available under the Functional
Source License (FSL-1.1-ALv2): read it, run it and change it, but do not use it to sell a competing product. Each
version becomes Apache-2.0 two years after it is released.

This repository is published from Looksie's private repository at each release. Questions or a security report:
support@looksie.dev.
