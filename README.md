# SkillBridge

[![CI](https://github.com/AhmedShehab/skillbridge/actions/workflows/ci.yml/badge.svg)](https://github.com/AhmedShehab/skillbridge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`skillbridge` is a local-first, Git-native CLI for keeping AI coding-agent skills and instructions consistent across machines and tools.

It stores portable content in a human-readable profile, then generates the native files expected by supported agents. It does not upload your data, copy credentials, or silently overwrite edits.

## Supported agents

The built-in adapters cover the main local coding-agent CLIs and agent TUIs:

- Claude Code, OpenAI Codex CLI, Gemini CLI, Cline, Cursor, GitHub Copilot CLI, OpenCode, and Aider
- ZCode, Zed Agent, Windsurf, Continue CLI, Amazon Q Developer, and Kiro CLI
- Qwen Code, Pi, Goose, Crush, Factory Droid, OpenHands, Roo Code, Amp, Kimi Code CLI, Junie, and Mistral Vibe

Agent Skills are stored using the open `SKILL.md` format. The profile also supports Markdown instructions and user-authored memory notes. Adapter support means file-based discovery and materialization; credentials, provider/model configuration, MCP settings, sessions, transcripts, caches, and vendor-generated memories stay local by design. ZCode and Windsurf automatic memories are explicitly excluded because they are machine-local state. Aider, Continue, Goose, Amazon Q, and Windsurf receive their documented rules/conventions files rather than native Agent Skills.

Use `skillbridge adapters` to see every adapter, its capabilities, and a short summary of the native files it understands. Paths are based on each tool's documented conventions and may evolve as vendors add new formats.

## Install from source

```sh
cargo install --path .
```

To install the latest version directly from GitHub:

```sh
cargo install --git https://github.com/AhmedShehab/skillbridge --branch main skill-bridge
```

After the crates.io release, install it with:

```sh
cargo install skill-bridge
```

## Quick start

Create a profile outside the repository you are working on:

```sh
skillbridge init --profile "$HOME/.skillbridge"
skillbridge scan
skillbridge import --profile "$HOME/.skillbridge"
skillbridge plan --profile "$HOME/.skillbridge"
skillbridge apply --profile "$HOME/.skillbridge" --yes
```

Commit the profile with Git and clone it on another machine. Run `plan` before `apply` whenever the profile or a target agent may have changed.

Without `--agent`, discovery and materialization target agents detected by their installed directories or command-line binaries. Pass an explicit adapter name to prepare a target that is not installed yet.

## Safety model

- No network service or account is required.
- API keys, tokens, credentials, private keys, `.env` files, caches, logs, databases, and transcripts are excluded.
- Symlinks are not followed.
- Target writes are atomic and constrained to the adapter-owned destination.
- A target changed outside the recorded manifest becomes an explicit conflict.
- Deletions are reported but never performed automatically.

## Canonical profile layout

```text
profile/
  config.toml
  global/
    skills/<skill-name>/SKILL.md
    instructions/*.md
    memory/*.md
  projects/<project-id>/
    skills/
    instructions/
    memory/
  state/manifest.json
```

## Commands

```text
skillbridge init       Create a profile
skillbridge scan       Discover native files without writing
skillbridge import     Copy discovered files into the profile
skillbridge plan       Preview generated target changes
skillbridge apply      Apply a reviewed plan with --yes
skillbridge status     Show drift and conflicts
skillbridge doctor     Validate the profile and adapter environment
skillbridge adapters    List built-in adapters
```

Use `--scope global`, `--scope project`, or the default `--scope both`. Use `--agent claude` to limit an operation to one adapter and `--project PATH` to select a project root. Several adapters also understand the portable `.agents/skills` location.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Adapters are deliberately implemented behind a small trait so new agents can be added without changing the canonical store or sync engine.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and adapter guidelines. Security issues should be reported privately through [SECURITY.md](SECURITY.md).

## Roadmap

Future releases may add hooks, MCP configuration, custom agents, model settings, vendor-specific memory importers, history migration, an optional watcher, and encrypted remote backends. Those features are intentionally outside the first local-only release.

## License

Released under the [MIT License](LICENSE).
