# Changelog

All notable changes to `agent-sync` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-09-02

### Added

- Git-native canonical profiles for global and project-scoped skills, instructions, and memory.
- Discovery, import, plan, apply, status, doctor, and adapter-listing commands.
- Adapters for Claude Code, OpenAI Codex CLI, Gemini CLI, Cline, Cursor, GitHub Copilot CLI, OpenCode, and Aider.
- Portable Agent Skills `SKILL.md` validation and universal `.agents/skills` fallback support.
- Hash-based drift detection, explicit conflict handling, path containment, symlink avoidance, secret filtering, and atomic writes.
