# Changelog

All notable changes to `skillbridge` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Adapters for ZCode, Zed Agent, Windsurf, Continue CLI, Amazon Q Developer, Kiro CLI, Qwen Code, Pi, Goose, Crush, Factory Droid, OpenHands, Roo Code, Amp, Kimi Code CLI, Junie, and Mistral Vibe.
- Recursive discovery for documented Markdown rule directories, with filtering for non-instruction configuration files and generated SkillBridge output.

### Changed

- Renamed the package and CLI from `agent-sync` to `skill-bridge` / `skillbridge`; the legacy `AGENT_SYNC_HOME` environment variable remains accepted as a compatibility fallback.

## [0.1.0] - 2026-09-02

### Added

- Git-native canonical profiles for global and project-scoped skills, instructions, and memory.
- Discovery, import, plan, apply, status, doctor, and adapter-listing commands.
- Adapters for Claude Code, OpenAI Codex CLI, Gemini CLI, Cline, Cursor, GitHub Copilot CLI, OpenCode, and Aider.
- Portable Agent Skills `SKILL.md` validation and universal `.agents/skills` fallback support.
- Hash-based drift detection, explicit conflict handling, path containment, symlink avoidance, secret filtering, and atomic writes.
