# Contributing to agent-sync

Thanks for helping make `agent-sync` safer and more useful across AI coding tools.

## Development setup

Install a current stable Rust toolchain, then run the checks used by CI:

```sh
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

Build and try the CLI locally with:

```sh
cargo run -- --help
```

## Pull requests

- Open an issue first for large behavior changes or new integration families.
- Keep changes focused and explain user-visible behavior in the PR description.
- Add or update tests for behavior changes.
- Do not add credentials, personal machine paths, generated target files, or agent transcripts.
- Preserve the local-first safety model: no network access, no symlink traversal, atomic writes, and explicit conflicts.

Before requesting review, run the same formatting, test, and Clippy commands shown above.

## Adding an adapter

Adapters should describe one tool's native locations and capabilities while keeping canonical profile formats tool-independent. Add the adapter to `src/adapters.rs`, cover discovery and materialization behavior in tests, and document the supported locations in the README when the integration is user-facing.

## Commit and review style

Use clear, imperative commit messages. Reviewers will pay particular attention to path safety, accidental data collection, conflict handling, portability, and backwards-compatible profile changes.

Please report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md), rather than opening a public issue.
