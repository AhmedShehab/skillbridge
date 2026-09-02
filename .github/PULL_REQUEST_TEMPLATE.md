## Summary

<!-- What does this change do, and why? -->

## Testing

<!-- Include commands and relevant manual checks. -->

```text
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## Checklist

- [ ] I added or updated tests for behavior changes.
- [ ] I updated documentation or changelog entries where needed.
- [ ] I checked that no credentials, private paths, transcripts, or generated files are included.
- [ ] I considered path safety, symlinks, conflicts, and backwards compatibility.
- [ ] This change is ready for review and does not require follow-up work hidden from reviewers.

## Breaking changes

<!-- Describe profile, CLI, or behavior changes. Write “None” when not applicable. -->

## Related issues

<!-- Link issues with “Closes #123” where appropriate. -->
