# Security policy

## Scope

`skillbridge` is designed to run locally and write only to its canonical profile and adapter-owned target paths. It intentionally excludes credentials, authentication stores, databases, caches, logs, and transcripts.

## Reporting a vulnerability

Please do not open a public issue for a suspected security vulnerability. Report it privately through the repository's [GitHub Security Advisory form](https://github.com/AhmedShehab/skillbridge/security/advisories/new). If that form is unavailable, contact the repository maintainer through [their GitHub profile](https://github.com/AhmedShehab).

Include:

- the affected version or commit;
- reproduction steps or a minimal fixture;
- the expected and observed behavior;
- any relevant operating-system details;
- whether the issue involves credentials, symlinks, path traversal, or unintended writes.

Please allow maintainers reasonable time to investigate before public disclosure.

## Supported versions

Only the latest released version on the default branch is currently supported with security fixes. Pre-release builds are provided for testing and may change without notice.
