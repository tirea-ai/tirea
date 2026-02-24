# Contributing

Thanks for contributing to Tirea.

## Development setup

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --lib --bins --examples -- -D clippy::correctness
```

## Coding standards

- Follow existing crate boundaries and naming conventions.
- Keep behavior deterministic for state/patch operations.
- Prefer explicit error context with `thiserror` + `.context()`.
- Add/adjust tests for every behavior change.

## Pull requests

Before opening a PR, make sure:

- `cargo test --workspace` passes
- `cargo fmt` (or repo format checks) passes
- new behavior is covered by tests
- relevant docs are updated when API/schema/config changes

## Commit messages

This repository uses conventional commit style with emoji:

`<emoji> <type>(<scope>): <subject>`

